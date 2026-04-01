//! Isolated Phase Benchmark for TTW Installation
//!
//! Tests each operation in COMPLETE isolation with REAL disk I/O:
//! 1. Decompress - extract files from source BSAs, write to disk
//! 2. Copy - decompress + write to real output files
//! 3. New - read files from MPI package, write to disk
//! 4. Patch - decompress + apply xdelta3 patches, write to disk
//! 5. OggEnc2 - decompress + OGG resample, write to disk
//! 6. AudioEnc - decompress + audio format convert, write to disk
//! 7. BSA Build - build all 26 real BSAs from real staged data
//!
//! Usage:
//!   cargo run --release --example bench_phases -- \
//!     --mpi <path-to-mpi-or-extracted-dir> \
//!     --fo3 <fallout3-dir> --fnv <fnv-dir> --oblivion <oblivion-dir> \
//!     --dest <benchmark-output-dir>

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ba2::tes4::{Archive, FileCompressionOptions};
use ba2::{ByteSlice, Reader};
use rayon::prelude::*;
use sysinfo::System;

use ttw_installer::models::{Asset, InstallConfig, Location};
use ttw_installer::services::{
    AudioProcessor, AudioFormat, BsaWriterManager, LocationResolver,
    ManifestLoader, MemoryMonitor, MpiExtractor, StreamingBsaBuilder, XdeltaManager,
};

// ─── Helpers ────────────────────────────────────────────────────────────────

fn get_rss_mb() -> f64 {
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let parts: Vec<&str> = statm.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(rss_pages) = parts[1].parse::<u64>() {
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                return (rss_pages * page_size) as f64 / 1024.0 / 1024.0;
            }
        }
    }
    0.0
}

struct PhaseResult {
    name: String,
    elapsed: Duration,
    ops: usize,
    bytes_processed: usize,
    peak_rss_mb: f64,
    rss_delta_mb: f64,
}

impl PhaseResult {
    fn print(&self) {
        let secs = self.elapsed.as_secs_f64();
        let mb = self.bytes_processed as f64 / 1024.0 / 1024.0;
        let throughput = if secs > 0.0 { mb / secs } else { 0.0 };
        let per_op_ms = if self.ops > 0 { secs * 1000.0 / self.ops as f64 } else { 0.0 };

        println!("  {:<25} {:>7.1}s  {:>7} ops  {:>8.1} MB  {:>7.0} MB/s  {:>6.2}ms/op  peak {:.0} MB (Δ{:.0})",
            self.name, secs, self.ops, mb, throughput, per_op_ms, self.peak_rss_mb, self.rss_delta_mb);
    }
}

struct BsaFileGroup {
    bsa_path: PathBuf,
    /// normalized_path -> list of assets needing that file
    files: HashMap<String, Vec<Asset>>,
}

fn group_assets_by_bsa(
    assets: &[Asset],
    resolver: &LocationResolver,
    op_filter: Option<i32>,
) -> (Vec<BsaFileGroup>, Vec<Asset>) {
    let mut bsa_groups: HashMap<PathBuf, HashMap<String, Vec<Asset>>> = HashMap::new();
    let mut dir_assets: Vec<Asset> = Vec::new();

    for asset in assets {
        if let Some(filter) = op_filter {
            if asset.op_type != filter { continue; }
        }
        if resolver.is_bsa_location(asset.source_loc) {
            if let Ok(bsa_path) = resolver.get_bsa_path(asset.source_loc) {
                let normalized = asset.source_path.replace('/', "\\").to_lowercase();
                bsa_groups.entry(bsa_path).or_default()
                    .entry(normalized).or_default().push(asset.clone());
            }
        } else {
            dir_assets.push(asset.clone());
        }
    }

    let mut groups: Vec<BsaFileGroup> = bsa_groups.into_iter().map(|(path, files)| {
        BsaFileGroup { bsa_path: path, files }
    }).collect();
    groups.sort_by(|a, b| {
        let sa = std::fs::metadata(&a.bsa_path).map(|m| m.len()).unwrap_or(0);
        let sb = std::fs::metadata(&b.bsa_path).map(|m| m.len()).unwrap_or(0);
        sb.cmp(&sa)
    });

    (groups, dir_assets)
}

/// Iterate a BSA archive and call a closure for each matched file
fn for_each_matched_file<'a, F>(
    group: &BsaFileGroup,
    mut callback: F,
) where
    F: FnMut(&[Asset], &[u8]), // assets needing this file, decompressed data
{
    let (archive, options): (Archive, _) = match Archive::read(group.bsa_path.as_path()) {
        Ok(a) => a,
        Err(_) => return,
    };
    let comp_opts = FileCompressionOptions::from(&options);

    let mut remaining = group.files.clone();

    for (dir_key, folder) in archive.iter() {
        if remaining.is_empty() { break; }
        let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();
        for (file_key, file) in folder.iter() {
            let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
            let full_path = if dir_name.is_empty() || dir_name == "." {
                file_name
            } else {
                format!("{}\\{}", dir_name, file_name)
            };
            if let Some(assets) = remaining.remove(&full_path) {
                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&comp_opts) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(_) => continue,
                    }
                };
                callback(&assets, &data);
            }
        }
    }
}

fn find_file_ci(path: &Path) -> Option<PathBuf> {
    if path.exists() { return Some(path.to_path_buf()); }
    let parent = path.parent()?;
    let target = path.file_name()?.to_string_lossy().to_lowercase();
    let actual_parent = find_dir_ci(parent)?;
    if let Ok(entries) = std::fs::read_dir(&actual_parent) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().to_lowercase() == target {
                return Some(entry.path());
            }
        }
    }
    None
}

fn find_dir_ci(path: &Path) -> Option<PathBuf> {
    if path.exists() { return Some(path.to_path_buf()); }
    let mut current = PathBuf::new();
    for (i, component) in path.components().enumerate() {
        if i == 0 { current.push(component); continue; }
        let target = component.as_os_str().to_string_lossy().to_lowercase();
        let mut found = false;
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_name().to_string_lossy().to_lowercase() == target {
                    current = entry.path(); found = true; break;
                }
            }
        }
        if !found { current.push(component); }
    }
    if current.exists() { Some(current) } else { None }
}

fn find_manifest(mpi_dir: &Path) -> Result<PathBuf> {
    for name in ["_package/index.json", "manifest.json", "index.json"] {
        let path = mpi_dir.join(name);
        if path.exists() { return Ok(path); }
    }
    bail!("No manifest found in {}", mpi_dir.display());
}

// ─── Phase 1: Decompress + Write to Disk ────────────────────────────────────

fn bench_decompress(assets: &[Asset], resolver: &LocationResolver, out_dir: &Path) -> PhaseResult {
    println!("\n  Phase 1: Decompress (extract from BSAs, write to disk)...");
    let (groups, _) = group_assets_by_bsa(assets, resolver, None);
    let write_dir = out_dir.join("phase1_decompress");
    std::fs::create_dir_all(&write_dir).unwrap();

    let total_ops = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);

    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    for group in &groups {
        // Collect all matched files first (single-threaded BSA scan)
        let mut all_matched: Vec<(Vec<Asset>, Vec<u8>)> = Vec::new();
        for_each_matched_file(group, |assets, data| {
            all_matched.push((assets.to_vec(), data.to_vec()));
        });

        // Write to disk in parallel
        all_matched.par_iter().for_each(|(assets, data)| {
            for asset in assets {
                let out_path = write_dir.join(asset.source_path.replace('\\', "/"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&out_path, data);
                total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                total_ops.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    let elapsed = start.elapsed();
    let mem = monitor.stop();
    let _ = std::fs::remove_dir_all(&write_dir);

    PhaseResult {
        name: "Decompress+Write".to_string(), elapsed,
        ops: total_ops.load(Ordering::Relaxed),
        bytes_processed: total_bytes.load(Ordering::Relaxed),
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Phase 2: Copy (decompress + write, op_type=0 only) ────────────────────

fn bench_copy(assets: &[Asset], resolver: &LocationResolver, out_dir: &Path) -> PhaseResult {
    println!("\n  Phase 2: Copy (decompress + write to disk, op_type=0)...");
    let (groups, _) = group_assets_by_bsa(assets, resolver, Some(0));
    let write_dir = out_dir.join("phase2_copy");
    std::fs::create_dir_all(&write_dir).unwrap();

    let total_ops = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);

    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    for group in &groups {
        let mut all_matched: Vec<(Vec<Asset>, Vec<u8>)> = Vec::new();
        for_each_matched_file(group, |assets, data| {
            all_matched.push((assets.to_vec(), data.to_vec()));
        });

        all_matched.par_iter().for_each(|(assets, data)| {
            for asset in assets {
                let target = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                let out_path = write_dir.join(target.replace('\\', "/"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&out_path, data);
                total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                total_ops.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    let elapsed = start.elapsed();
    let mem = monitor.stop();
    let _ = std::fs::remove_dir_all(&write_dir);

    PhaseResult {
        name: "Copy".to_string(), elapsed,
        ops: total_ops.load(Ordering::Relaxed),
        bytes_processed: total_bytes.load(Ordering::Relaxed),
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Phase 3: New (read from MPI, write to disk) ───────────────────────────

fn bench_new(assets: &[Asset], mpi_dir: &Path, out_dir: &Path) -> PhaseResult {
    println!("\n  Phase 3: New (read from MPI package, write to disk)...");
    let new_assets: Vec<&Asset> = assets.iter().filter(|a| a.op_type == 1).collect();
    let write_dir = out_dir.join("phase3_new");
    std::fs::create_dir_all(&write_dir).unwrap();

    let total_ops = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);

    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    new_assets.par_iter().for_each(|asset| {
        let normalized = asset.source_path.replace('\\', "/");
        let source_path = mpi_dir.join(&normalized);
        let actual_path = find_file_ci(&source_path).unwrap_or(source_path);

        match std::fs::read(&actual_path) {
            Ok(data) => {
                let target = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                let out_path = write_dir.join(target.replace('\\', "/"));
                if let Some(parent) = out_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&out_path, &data);
                total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                total_ops.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => { errors.fetch_add(1, Ordering::Relaxed); }
        }
    });

    let elapsed = start.elapsed();
    let mem = monitor.stop();
    let errs = errors.load(Ordering::Relaxed);
    if errs > 0 { println!("    ({} files not found)", errs); }
    let _ = std::fs::remove_dir_all(&write_dir);

    PhaseResult {
        name: "New (from MPI)".to_string(), elapsed,
        ops: total_ops.load(Ordering::Relaxed),
        bytes_processed: total_bytes.load(Ordering::Relaxed),
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Phase 4: Patch (decompress + xdelta + write) ──────────────────────────

fn bench_patch(assets: &[Asset], resolver: &LocationResolver, xdelta: &XdeltaManager, mpi_dir: &Path, out_dir: &Path) -> PhaseResult {
    println!("\n  Phase 4: Patch (decompress + xdelta3 + write to disk)...");
    let (groups, _) = group_assets_by_bsa(assets, resolver, Some(2));
    let write_dir = out_dir.join("phase4_patch");
    std::fs::create_dir_all(&write_dir).unwrap();

    let total_ops = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);

    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    for group in &groups {
        let mut all_matched: Vec<(Vec<Asset>, Vec<u8>)> = Vec::new();
        for_each_matched_file(group, |assets, data| {
            all_matched.push((assets.to_vec(), data.to_vec()));
        });

        all_matched.par_iter().for_each(|(assets, source_data)| {
            for asset in assets {
                let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                let patch_name = format!("{}.xd3", target_file).replace('\\', "/");
                let patch_path = mpi_dir.join(&patch_name);

                let actual_patch = match find_file_ci(&patch_path) {
                    Some(p) => p,
                    None => { errors.fetch_add(1, Ordering::Relaxed); continue; }
                };

                let patch_data = match std::fs::read(&actual_patch) {
                    Ok(d) => d,
                    Err(_) => { errors.fetch_add(1, Ordering::Relaxed); continue; }
                };

                match xdelta.apply_patch_from_bytes(source_data, &patch_data) {
                    Ok(patched) => {
                        let out_path = write_dir.join(target_file.replace('\\', "/"));
                        if let Some(parent) = out_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&out_path, &patched);
                        total_bytes.fetch_add(patched.len(), Ordering::Relaxed);
                        total_ops.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => { errors.fetch_add(1, Ordering::Relaxed); }
                }
            }
        });
    }

    let elapsed = start.elapsed();
    let mem = monitor.stop();
    let errs = errors.load(Ordering::Relaxed);
    if errs > 0 { println!("    ({} errors)", errs); }
    let _ = std::fs::remove_dir_all(&write_dir);

    PhaseResult {
        name: "Patch (xdelta3)".to_string(), elapsed,
        ops: total_ops.load(Ordering::Relaxed),
        bytes_processed: total_bytes.load(Ordering::Relaxed),
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Phase 5: Audio (decompress + process + write) ─────────────────────────

fn bench_audio(assets: &[Asset], resolver: &LocationResolver, op_type: i32, label: &str, out_dir: &Path) -> PhaseResult {
    println!("\n  Phase {}: {} (decompress + process + write to disk)...",
        if op_type == 4 { "5a" } else { "5b" }, label);
    let (groups, _) = group_assets_by_bsa(assets, resolver, Some(op_type));
    let write_dir = out_dir.join(format!("phase5_{}", label.replace(' ', "_")));
    std::fs::create_dir_all(&write_dir).unwrap();

    let total_ops = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);

    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    for group in &groups {
        let mut all_matched: Vec<(Vec<Asset>, Vec<u8>)> = Vec::new();
        for_each_matched_file(group, |assets, data| {
            all_matched.push((assets.to_vec(), data.to_vec()));
        });

        all_matched.par_iter().for_each(|(assets, source_data)| {
            for asset in assets {
                let audio_processor = AudioProcessor::new().with_params(&asset.params);

                let result = if op_type == 4 {
                    audio_processor.process_ogg_resample(source_data)
                } else {
                    let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                    let output_format = Path::new(target_file)
                        .extension().and_then(|e| e.to_str())
                        .and_then(AudioFormat::from_extension)
                        .unwrap_or(AudioFormat::Ogg);
                    let input_format = Path::new(&asset.source_path)
                        .extension().and_then(|e| e.to_str());
                    audio_processor.process_audio_conversion(source_data, input_format, output_format)
                };

                match result {
                    Ok(processed) => {
                        let target = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                        let out_path = write_dir.join(target.replace('\\', "/"));
                        if let Some(parent) = out_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&out_path, &processed);
                        total_bytes.fetch_add(processed.len(), Ordering::Relaxed);
                        total_ops.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => { errors.fetch_add(1, Ordering::Relaxed); }
                }
            }
        });
    }

    let elapsed = start.elapsed();
    let mem = monitor.stop();
    let errs = errors.load(Ordering::Relaxed);
    if errs > 0 { println!("    ({} errors)", errs); }
    let _ = std::fs::remove_dir_all(&write_dir);

    PhaseResult {
        name: label.to_string(), elapsed,
        ops: total_ops.load(Ordering::Relaxed),
        bytes_processed: total_bytes.load(Ordering::Relaxed),
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Phase 6: BSA Build (real data, all 26 BSAs) ───────────────────────────

fn bench_bsa_build(
    assets: &[Asset],
    resolver: &LocationResolver,
    locations: &[Location],
    bsa_targets: &[Location],
    dest_dir: &Path,
    mpi_dir: &Path,
    xdelta: &XdeltaManager,
) -> PhaseResult {
    println!("\n  Phase 6: BSA Build (stage ALL real data, then build all 26 BSAs)...");

    // Set up BSA writer with real targets (same as real install)
    let mut bsa_writer = BsaWriterManager::new(dest_dir.to_path_buf());

    let mut bsa_name_to_index: HashMap<String, i32> = HashMap::new();
    for (i, loc) in locations.iter().enumerate() {
        let name = loc.name.as_deref().unwrap_or("");
        if name.starts_with("NEW ") || name.to_lowercase().ends_with(".bsa") {
            bsa_name_to_index.insert(name.to_lowercase(), i as i32);
        }
    }

    for bsa_loc in bsa_targets {
        let name = bsa_loc.name.as_deref().unwrap_or("");
        let value = bsa_loc.value.as_deref().unwrap_or("");
        let name_lower = name.to_lowercase();
        let location_idx = bsa_name_to_index.get(&name_lower).copied()
            .unwrap_or_else(|| {
                let stripped = name.strip_prefix("NEW ")
                    .map(|s| s.to_lowercase()).unwrap_or_else(|| name_lower.clone());
                bsa_name_to_index.get(&stripped).copied().unwrap_or(-1)
            });

        if location_idx >= 0 {
            let normalized = value.replace('\\', "/");
            let bsa_filename = normalized.rsplit('/').next().unwrap_or(&normalized);
            let _ = bsa_writer.register_bsa(
                location_idx, bsa_filename,
                bsa_loc.archive_type, bsa_loc.archive_flags,
                bsa_loc.files_flags, bsa_loc.archive_compressed,
            );
        }
    }

    // Stage all files (process every asset and add result to BSA writer)
    println!("    Staging files to BSA builders...");
    let staged = AtomicUsize::new(0);
    let stage_start = Instant::now();

    // Process BSA-sourced assets
    let (groups, dir_assets) = group_assets_by_bsa(assets, resolver, None);

    for group in &groups {
        let mut all_matched: Vec<(Vec<Asset>, Vec<u8>)> = Vec::new();
        for_each_matched_file(group, |matched_assets, data| {
            all_matched.push((matched_assets.to_vec(), data.to_vec()));
        });

        // Process and stage in parallel
        all_matched.par_iter().for_each(|(matched_assets, source_data)| {
            for asset in matched_assets {
                if !bsa_writer.is_bsa_location(asset.target_loc) { continue; }

                let result_data = match asset.op_type {
                    0 => Some(source_data.clone()), // Copy
                    2 => { // Patch
                        let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                        let patch_name = format!("{}.xd3", target_file).replace('\\', "/");
                        let patch_path = Path::new(&mpi_dir).join(&patch_name);
                        find_file_ci(&patch_path)
                            .and_then(|p| std::fs::read(&p).ok())
                            .and_then(|patch_data| xdelta.apply_patch_from_bytes(source_data, &patch_data).ok())
                    }
                    4 => { // OggEnc2
                        let ap = AudioProcessor::new().with_params(&asset.params);
                        ap.process_ogg_resample(source_data).ok()
                    }
                    5 => { // AudioEnc
                        let ap = AudioProcessor::new().with_params(&asset.params);
                        let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                        let fmt = Path::new(target_file).extension().and_then(|e| e.to_str())
                            .and_then(AudioFormat::from_extension).unwrap_or(AudioFormat::Ogg);
                        let infmt = Path::new(&asset.source_path).extension().and_then(|e| e.to_str());
                        ap.process_audio_conversion(source_data, infmt, fmt).ok()
                    }
                    _ => None,
                };

                if let Some(data) = result_data {
                    let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
                    let normalized = target_file.replace('\\', "/");
                    let _ = bsa_writer.add_file(asset.target_loc, &normalized, data);
                    staged.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }

    // Stage New files (op_type=1) and dir-sourced assets
    for asset in &dir_assets {
        if !bsa_writer.is_bsa_location(asset.target_loc) { continue; }

        let data = if asset.op_type == 1 {
            let normalized = asset.source_path.replace('\\', "/");
            let source_path = mpi_dir.join(&normalized);
            find_file_ci(&source_path).and_then(|p| std::fs::read(&p).ok())
        } else {
            let source_dir = resolver.resolve_path(asset.source_loc).ok();
            source_dir.and_then(|dir| {
                let normalized = asset.source_path.replace('\\', "/");
                let path = dir.join(&normalized);
                find_file_ci(&path).and_then(|p| std::fs::read(&p).ok())
            })
        };

        if let Some(data) = data {
            let target_file = asset.target_path.as_deref().unwrap_or(&asset.source_path);
            let normalized = target_file.replace('\\', "/");
            let _ = bsa_writer.add_file(asset.target_loc, &normalized, data);
            staged.fetch_add(1, Ordering::Relaxed);
        }
    }

    let stage_elapsed = stage_start.elapsed();
    let staged_count = staged.load(Ordering::Relaxed);
    println!("    Staged {} files in {:.1}s", staged_count, stage_elapsed.as_secs_f64());

    // NOW benchmark just the BSA build phase
    println!("    Building BSAs...");
    let monitor = MemoryMonitor::start(Duration::from_millis(100));
    let start = Instant::now();

    let (bsa_success, bsa_fail) = bsa_writer.write_all(dest_dir).unwrap_or((0, 0));

    let elapsed = start.elapsed();
    let mem = monitor.stop();

    // Measure output size
    let mut total_bsa_bytes = 0usize;
    if let Ok(entries) = std::fs::read_dir(dest_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().extension().map(|e| e == "bsa").unwrap_or(false) {
                total_bsa_bytes += std::fs::metadata(entry.path()).map(|m| m.len() as usize).unwrap_or(0);
            }
        }
    }

    println!("    {} BSAs built, {} failed", bsa_success, bsa_fail);

    PhaseResult {
        name: "BSA Build (real)".to_string(), elapsed,
        ops: bsa_success,
        bytes_processed: total_bsa_bytes,
        peak_rss_mb: mem.peak_rss_mb, rss_delta_mb: mem.delta_mb,
    }
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut mpi_path = None;
    let mut fo3_path = None;
    let mut fnv_path = None;
    let mut oblivion_path = None;
    let mut dest_path = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mpi" => { i += 1; mpi_path = Some(PathBuf::from(&args[i])); }
            "--fo3" => { i += 1; fo3_path = Some(PathBuf::from(&args[i])); }
            "--fnv" => { i += 1; fnv_path = Some(PathBuf::from(&args[i])); }
            "--oblivion" => { i += 1; oblivion_path = Some(PathBuf::from(&args[i])); }
            "--dest" => { i += 1; dest_path = Some(PathBuf::from(&args[i])); }
            _ => {}
        }
        i += 1;
    }

    let mpi = mpi_path.expect("--mpi required");
    let dest = dest_path.expect("--dest required");

    println!("\n{}", "=".repeat(100));
    println!("  TTW Isolated Phase Benchmark (REAL I/O - no cheating)");
    println!("{}\n", "=".repeat(100));

    let mut sys = System::new_all();
    sys.refresh_all();
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    println!("  System: {} CPUs, {:.1} GB RAM, {:.1} GB available",
        cpus, sys.total_memory() as f64 / 1e9, sys.available_memory() as f64 / 1e9);
    println!("  Process RSS: {:.0} MB", get_rss_mb());

    // Extract MPI
    let (mpi_dir, _cleanup) = if MpiExtractor::is_mpi_file(&mpi) {
        println!("\n  Extracting MPI package...");
        let extract_dir = dest.join(".bench_mpi");
        if extract_dir.exists() { std::fs::remove_dir_all(&extract_dir)?; }
        let extracted = MpiExtractor::extract_to(&mpi, &extract_dir)?;
        (extracted, true)
    } else if mpi.is_dir() {
        (mpi.clone(), false)
    } else {
        bail!("Invalid MPI path");
    };

    // Load manifest
    let manifest_path = find_manifest(&mpi_dir)?;
    let manifest = ManifestLoader::load_from_file(&manifest_path)?;
    let assets = ManifestLoader::parse_assets(&manifest)?;
    let locations = ManifestLoader::get_locations(&manifest, 0)?;
    let bsa_targets = ManifestLoader::get_bsa_target_locations(&manifest)?;
    let variables = ManifestLoader::get_variables(&manifest, 0).unwrap_or_default();

    let mut op_counts: HashMap<i32, usize> = HashMap::new();
    for a in &assets { *op_counts.entry(a.op_type).or_insert(0) += 1; }
    println!("\n  Assets: {} total", assets.len());
    for (op, count) in &op_counts {
        let name = match op { 0=>"Copy", 1=>"New", 2=>"Patch", 4=>"OggEnc2", 5=>"AudioEnc", _=>"?" };
        println!("    {:<10} ({}): {:>6}", name, op, count);
    }

    let config = InstallConfig {
        fallout3_root: fo3_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        falloutnv_root: fnv_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        oblivion_root: oblivion_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        destination_path: dest.to_string_lossy().to_string(),
        mpi_package_path: mpi_dir.to_string_lossy().to_string(),
    };

    let resolver = LocationResolver::new(locations.clone(), config).with_variables(&variables);
    std::fs::create_dir_all(&dest)?;
    let xdelta = XdeltaManager::ensure_available(dest.to_path_buf())?;

    // ─── Run Phases ─────────────────────────────────────────────────────

    println!("\n{}", "─".repeat(100));
    println!("  ISOLATED PHASE BENCHMARKS (real disk I/O, real data)");
    println!("{}", "─".repeat(100));

    let mut results: Vec<PhaseResult> = Vec::new();

    results.push(bench_decompress(&assets, &resolver, &dest));
    results.last().unwrap().print();

    results.push(bench_copy(&assets, &resolver, &dest));
    results.last().unwrap().print();

    results.push(bench_new(&assets, &mpi_dir, &dest));
    results.last().unwrap().print();

    results.push(bench_patch(&assets, &resolver, &xdelta, &mpi_dir, &dest));
    results.last().unwrap().print();

    results.push(bench_audio(&assets, &resolver, 4, "OggEnc2 (resample)", &dest));
    results.last().unwrap().print();

    results.push(bench_audio(&assets, &resolver, 5, "AudioEnc (convert)", &dest));
    results.last().unwrap().print();

    // BSA build is special - it needs to process ALL assets to stage them, then build
    let bsa_dest = dest.join("bsa_output");
    std::fs::create_dir_all(&bsa_dest)?;
    results.push(bench_bsa_build(&assets, &resolver, &locations, &bsa_targets, &bsa_dest, &mpi_dir, &xdelta));
    results.last().unwrap().print();

    // ─── Summary ────────────────────────────────────────────────────────

    println!("\n{}", "─".repeat(100));
    println!("  SUMMARY");
    println!("{}", "─".repeat(100));

    // Exclude BSA build from "phases total" since it re-does all processing to stage
    let phases_total: Duration = results.iter().take(6).map(|r| r.elapsed).sum();
    println!("\n  Phases 1-6 wall-clock: {:.1}s", phases_total.as_secs_f64());
    println!("  BSA build (phase 7):   {:.1}s", results.last().unwrap().elapsed.as_secs_f64());

    let total: Duration = results.iter().map(|r| r.elapsed).sum();
    println!("\n  {:25} {:>7}  {:>5}   Bar", "Phase", "Time", "%");
    println!("  {}", "─".repeat(70));
    for r in &results {
        let pct = r.elapsed.as_secs_f64() / total.as_secs_f64() * 100.0;
        println!("  {:<25} {:>6.1}s  {:>4.1}%   {}",
            r.name, r.elapsed.as_secs_f64(), pct,
            "█".repeat((pct / 2.0) as usize));
    }

    println!("\n  Final RSS: {:.0} MB", get_rss_mb());

    // Save
    let results_path = dest.join("phase_benchmark_results.txt");
    let mut output = format!("TTW Phase Benchmark (REAL I/O)\nSystem: {} CPUs\n\n", cpus);
    for r in &results {
        output.push_str(&format!("{}: {:.1}s, {} ops, {:.1} MB, peak {:.0} MB RSS\n",
            r.name, r.elapsed.as_secs_f64(), r.ops, r.bytes_processed as f64 / 1e6, r.peak_rss_mb));
    }
    std::fs::write(&results_path, &output)?;
    println!("  Results saved to: {}", results_path.display());

    Ok(())
}
