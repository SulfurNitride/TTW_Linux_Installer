use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::Instant;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use tracing::{warn, info};
use sysinfo::System;

use crate::models::{Asset, Location};
use crate::services::{
    LocationResolver, BsaHandler, BsaWriterManager, AudioProcessor, AudioFormat, XdeltaManager,
    BsaCache, MemoryMonitor, MemoryReport, print_ram_status,
};

/// Get number of chunks based on available RAM
/// Now that we use streaming BSA builders (write to disk, not RAM),
/// we can be much more aggressive with chunk sizes.
/// Peak RAM is ~2GB regardless of total assets.
///
/// - 4GB or less → 2 chunks (conservative)
/// - 6GB+ → 1 chunk (all at once, no pauses!)
fn get_chunk_count_for_ram() -> usize {
    let mut sys = System::new_all();
    sys.refresh_memory();
    let available_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    // With streaming BSA builders, we only need ~2GB peak RAM
    // So even modest systems can process in one chunk
    let chunks = if available_gb >= 6.0 {
        1  // All at once - no chunk boundary pauses!
    } else if available_gb >= 4.0 {
        2  // Two chunks for tighter systems
    } else {
        4  // Conservative for very low RAM (<4GB)
    };

    info!(
        "System RAM: {:.1}GB available → {} chunk(s) (streaming mode)",
        available_gb, chunks
    );

    chunks
}

/// Find a file with case-insensitive matching (for Linux compatibility)
fn find_file_case_insensitive(path: &Path) -> Option<PathBuf> {
    // If exact path exists, return it
    if path.exists() {
        return Some(path.to_path_buf());
    }

    let parent = path.parent()?;
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();

    // First, find the parent directory case-insensitively
    let actual_parent = find_dir_case_insensitive(parent)?;

    // Then find the file in that directory
    if let Ok(entries) = fs::read_dir(&actual_parent) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().to_lowercase() == file_name {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Find a directory with case-insensitive matching
fn find_dir_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }

    // Build path component by component
    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();

    for (i, component) in components.iter().enumerate() {
        let comp_str = component.as_os_str().to_string_lossy();

        if i == 0 {
            // Root or first component
            current.push(component);
            if !current.exists() {
                return None;
            }
            continue;
        }

        // Try to find matching directory
        let target = comp_str.to_lowercase();
        let mut found = false;

        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_name().to_string_lossy().to_lowercase() == target {
                    current = entry.path();
                    found = true;
                    break;
                }
            }
        }

        if !found {
            // Last resort: just append and hope it exists
            current.push(component);
            if !current.exists() {
                return None;
            }
        }
    }

    if current.exists() {
        Some(current)
    } else {
        None
    }
}

/// Operation types from the manifest
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    Copy = 0,
    New = 1,
    Patch = 2,
    OggEnc2 = 4,
    AudioEnc = 5,
}

impl OpType {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Copy),
            1 => Some(Self::New),
            2 => Some(Self::Patch),
            4 => Some(Self::OggEnc2),
            5 => Some(Self::AudioEnc),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::New => "New",
            Self::Patch => "Patch",
            Self::OggEnc2 => "OggEnc2",
            Self::AudioEnc => "AudioEnc",
        }
    }
}

/// Processes assets from TTW manifest (thread-safe)
pub struct AssetProcessor {
    resolver: Arc<LocationResolver>,
    bsa_handler: Arc<Mutex<BsaHandler>>,
    bsa_writer: Arc<Mutex<BsaWriterManager>>,
    /// SQLite-based cache for pre-extracted BSA files (disk-based, low RAM usage)
    bsa_cache: Arc<BsaCache>,
    xdelta: Arc<XdeltaManager>,
    mpi_dir: PathBuf,
    dest_dir: PathBuf,
    dry_run: bool,
}

impl AssetProcessor {
    pub fn new(
        resolver: LocationResolver,
        xdelta: XdeltaManager,
        mpi_dir: PathBuf,
        dest_dir: PathBuf,
        locations: &[Location],
        bsa_targets: &[Location],  // Separate BSA target locations (may come from different profile)
    ) -> Result<Self> {
        let mut bsa_writer = BsaWriterManager::new(dest_dir.clone());

        let mut type_counts = std::collections::HashMap::new();
        for loc in locations.iter() {
            *type_counts.entry(loc.loc_type).or_insert(0) += 1;
        }
        println!("\nLocation type counts: {:?}", type_counts);

        // Build a mapping from BSA name to location index in the main locations array
        // This is needed because assets reference location indices, not names
        let mut bsa_name_to_index: HashMap<String, i32> = HashMap::new();
        for (i, loc) in locations.iter().enumerate() {
            let name = loc.name.as_deref().unwrap_or("");
            if name.starts_with("NEW ") || name.to_lowercase().ends_with(".bsa") {
                bsa_name_to_index.insert(name.to_lowercase(), i as i32);
            }
        }

        // Register BSA targets using the proper configuration from bsa_targets
        // These may come from a different profile (e.g., Profile 1 for Windows)
        // but the location indices are matched by name to the current profile
        if !bsa_targets.is_empty() {
            println!("\nRegistering {} BSA targets with manifest flags:", bsa_targets.len());
            for bsa_loc in bsa_targets {
                let name = bsa_loc.name.as_deref().unwrap_or("");
                let value = bsa_loc.value.as_deref().unwrap_or("");

                // Find matching location index in the main profile
                let name_lower = name.to_lowercase();
                let location_idx = bsa_name_to_index.get(&name_lower).copied()
                    .unwrap_or_else(|| {
                        // Try without "NEW " prefix
                        let stripped = name.strip_prefix("NEW ")
                            .map(|s| s.to_lowercase())
                            .unwrap_or_else(|| name_lower.clone());
                        bsa_name_to_index.get(&stripped).copied().unwrap_or(-1)
                    });

                if location_idx >= 0 {
                    // Extract filename from Value path for the output name
                    let normalized = value.replace('\\', "/");
                    let bsa_filename = normalized.rsplit('/').next().unwrap_or(&normalized);

                    println!("  BSA: {} -> loc[{}] type={:?} flags={:?} types={:?} compressed={:?}",
                        bsa_filename, location_idx,
                        bsa_loc.archive_type, bsa_loc.archive_flags,
                        bsa_loc.files_flags, bsa_loc.archive_compressed);

                    bsa_writer.register_bsa(
                        location_idx,
                        bsa_filename,
                        bsa_loc.archive_type,
                        bsa_loc.archive_flags,
                        bsa_loc.files_flags,
                        bsa_loc.archive_compressed,
                    )?;
                }
            }
        } else {
            // Fallback: detect BSA targets from main locations array
            println!("\nNo separate BSA targets provided, detecting from locations...");
            for (i, loc) in locations.iter().enumerate() {
                let name = loc.name.as_deref().unwrap_or("");
                let value = loc.value.as_deref().unwrap_or("");

                let is_bsa_name = name.to_lowercase().ends_with(".bsa");
                let is_bsa_value = value.to_lowercase().ends_with(".bsa");
                let has_new_prefix = name.starts_with("NEW ") || name.starts_with("new ");

                // Format 1: Type 0 with .bsa in NAME (TTW 3.4 style)
                if loc.loc_type == 0 && is_bsa_name {
                    bsa_writer.register_bsa(i as i32, name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed)?;
                }
                // Format 2: Type 2 with .bsa in VALUE (original MPI format)
                else if loc.loc_type == 2 && is_bsa_value {
                    let normalized = value.replace('\\', "/");
                    let bsa_name = normalized.rsplit('/').next().unwrap_or(&normalized);
                    bsa_writer.register_bsa(i as i32, bsa_name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed)?;
                }
                // Format 3: Type 1 with "NEW " prefix
                else if loc.loc_type == 1 && has_new_prefix && is_bsa_value {
                    bsa_writer.register_bsa(i as i32, name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed)?;
                }
            }
        }

        // Create SQLite-based cache in the output directory
        let bsa_cache = BsaCache::new_at(dest_dir.clone())
            .context("Failed to create SQLite BSA cache")?;

        Ok(Self {
            resolver: Arc::new(resolver),
            bsa_handler: Arc::new(Mutex::new(BsaHandler::new())),
            bsa_writer: Arc::new(Mutex::new(bsa_writer)),
            bsa_cache: Arc::new(bsa_cache),
            xdelta: Arc::new(xdelta),
            mpi_dir,
            dest_dir,
            dry_run: false,
        })
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Pre-extract files from a BSA into the SQLite cache
    /// Uses streaming inserts to avoid loading all files into RAM at once
    /// Returns (files_extracted, bytes_used)
    fn pre_extract_bsa_files(
        &self,
        bsa_path: &Path,
        file_paths: &[&str],
    ) -> Result<(usize, usize)> {
        use ba2::tes4::{Archive, ArchiveKey, DirectoryKey, File as BsaFile, FileCompressionOptions};
        use ba2::{ByteSlice, Reader};
        use std::collections::HashSet;

        // Open BSA once - get options for proper decompression
        let (archive, options): (Archive, _) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to open BSA for pre-extraction: {}", bsa_path.display()))?;
        let compression_options: FileCompressionOptions = (&options).into();

        // Build set of needed paths (normalized to lowercase with backslashes)
        let needed: HashSet<String> = file_paths.iter()
            .map(|p| p.replace('/', "\\").to_lowercase())
            .collect();

        // Build lookup map for original path casing
        let path_lookup: HashMap<String, &str> = file_paths.iter()
            .map(|p| (p.replace('/', "\\").to_lowercase(), *p))
            .collect();

        // Stream files directly to SQLite - each file is inserted and dropped immediately
        // This keeps RAM usage low (only one file in memory at a time)
        let (count, bytes) = self.bsa_cache.insert_streaming(bsa_path, |inserter| {
            for (dir_key, folder) in archive.iter() {
                let dir_key: &ArchiveKey = dir_key;
                let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();

                for (file_key, file) in folder.iter() {
                    let file_key: &DirectoryKey = file_key;
                    let file: &BsaFile = file;
                    let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
                    let full_path = if dir_name.is_empty() || dir_name == "." {
                        file_name.clone()
                    } else {
                        format!("{}\\{}", dir_name, file_name)
                    };

                    if needed.contains(&full_path) {
                        // Extract file data with proper compression options
                        let data = if file.is_decompressed() {
                            file.as_bytes().to_vec()
                        } else {
                            file.decompress(&compression_options)?.as_bytes().to_vec()
                        };

                        // Find original path case from lookup
                        let original_path = path_lookup.get(&full_path)
                            .map(|s| s.to_string())
                            .unwrap_or(full_path);

                        // Insert immediately - data is dropped after this call
                        inserter(original_path, data)?;
                    }
                }
            }
            Ok(())
        })?;

        Ok((count, bytes))
    }

    /// Clear the BSA extraction cache
    fn clear_bsa_cache(&self) {
        if let Err(e) = self.bsa_cache.clear() {
            warn!("Failed to clear BSA cache: {}", e);
        }
    }

    /// Process a list of assets in parallel with smart BSA caching (chunked mode)
    pub fn process_assets(&self, assets: &[Asset]) -> Result<ProcessingStats> {
        let monitor = MemoryMonitor::start(std::time::Duration::from_millis(250));
        print_ram_status("Start (chunked mode)");
        // Group assets by operation type for progress display
        let mut by_type: HashMap<i32, Vec<&Asset>> = HashMap::new();
        for asset in assets {
            by_type.entry(asset.op_type).or_default().push(asset);
        }

        println!("\nProcessing {} total assets:", assets.len());
        for (op_type, group) in &by_type {
            let name = OpType::from_i32(*op_type)
                .map(|t| t.name())
                .unwrap_or("Unknown");
            println!("  {} ({}): {}", name, op_type, group.len());
        }

        // === Determine chunk count based on available RAM ===
        let num_chunks = get_chunk_count_for_ram();
        let chunk_size = assets.len().div_ceil(num_chunks);

        println!(
            "\n=== Processing in {} chunk(s) ({} assets/chunk) ===\n",
            num_chunks, chunk_size
        );

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let pb = ProgressBar::new(assets.len() as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"));

        // Process each chunk
        for (chunk_idx, chunk) in assets.chunks(chunk_size).enumerate() {
            pb.set_message(format!(
                "Chunk {}/{} ({} files)",
                chunk_idx + 1,
                num_chunks,
                chunk.len()
            ));

            // Pre-extract BSA files for this chunk only
            let mut bsa_files_needed: HashMap<PathBuf, Vec<&str>> = HashMap::new();
            for asset in chunk {
                if self.resolver.is_bsa_location(asset.source_loc) {
                    if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                        bsa_files_needed.entry(bsa_path)
                            .or_default()
                            .push(&asset.source_path);
                    }
                }
            }

            // Pre-extract this chunk's BSA files to SQLite (parallel)
            bsa_files_needed.par_iter().for_each(|(bsa_path, file_paths)| {
                if let Err(e) = self.pre_extract_bsa_files(bsa_path, file_paths) {
                    warn!("Failed to pre-extract from {}: {}", bsa_path.display(), e);
                }
            });

            // Process this chunk in parallel
            chunk.par_iter().for_each(|asset| {
                let result = self.process_asset(asset);

                match result {
                    Ok(_) => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg = format!(
                            "{} (op={}): {}",
                            asset.source_path, asset.op_type, e
                        );
                        warn!("Asset failed: {}", error_msg);
                        let mut errs = errors.lock().unwrap();
                        if errs.len() < 100 {
                            errs.push(error_msg);
                        }
                    }
                }

                pb.inc(1);
                let s = success.load(Ordering::Relaxed);
                let f = failed.load(Ordering::Relaxed);
                pb.set_message(format!("OK:{} Fail:{} (chunk {}/{})", s, f, chunk_idx + 1, num_chunks));
            });

            // Clear cache after each chunk to free memory
            self.clear_bsa_cache();
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        pb.finish_with_message(format!(
            "Done: {} success, {} failed",
            final_success, final_failed
        ));

        let memory_report = monitor.stop();
        print_ram_status("End (chunked mode)");

        let final_errors = Arc::try_unwrap(errors)
            .unwrap_or_else(|e| e.lock().unwrap().clone().into())
            .into_inner()
            .unwrap();

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            errors: final_errors,
            memory: Some(memory_report),
            timings: None,
        })
    }

    /// Process assets with a progress callback for GUI usage
    /// callback(current, total, message)
    pub fn process_assets_with_callback<F>(&self, assets: &[Asset], callback: F) -> Result<ProcessingStats>
    where
        F: Fn(usize, usize, &str) + Send + Sync,
    {
        let total = assets.len();
        let callback = Arc::new(callback);

        // === Determine chunk count based on available RAM ===
        let num_chunks = get_chunk_count_for_ram();
        let chunk_size = total.div_ceil(num_chunks);

        callback(0, total, &format!(
            "Processing in {} chunk(s) ({} assets/chunk)...",
            num_chunks, chunk_size
        ));

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let processed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // Process each chunk
        for (chunk_idx, chunk) in assets.chunks(chunk_size).enumerate() {
            callback(
                processed.load(Ordering::Relaxed),
                total,
                &format!("Chunk {}/{} ({} files): Pre-extracting...", chunk_idx + 1, num_chunks, chunk.len())
            );

            // Pre-extract BSA files for this chunk only
            let mut bsa_files_needed: HashMap<PathBuf, Vec<&str>> = HashMap::new();
            for asset in chunk {
                if self.resolver.is_bsa_location(asset.source_loc) {
                    if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                        bsa_files_needed.entry(bsa_path)
                            .or_default()
                            .push(&asset.source_path);
                    }
                }
            }

            // Pre-extract this chunk's BSA files to SQLite (parallel)
            bsa_files_needed.par_iter().for_each(|(bsa_path, file_paths)| {
                if let Err(e) = self.pre_extract_bsa_files(bsa_path, file_paths) {
                    warn!("Failed to pre-extract from {}: {}", bsa_path.display(), e);
                }
            });

            // Process this chunk in parallel
            chunk.par_iter().for_each(|asset| {
                let result = self.process_asset(asset);

                match result {
                    Ok(_) => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg = format!(
                            "{} (op={}): {}",
                            asset.source_path, asset.op_type, e
                        );
                        warn!("Asset failed: {}", error_msg);
                        let mut errs = errors.lock().unwrap();
                        if errs.len() < 100 {
                            errs.push(error_msg);
                        }
                    }
                }

                let current = processed.fetch_add(1, Ordering::Relaxed) + 1;

                let callback_interval = if total > 1000 { 50 } else if total > 100 { 10 } else { 1 };
                if current % callback_interval == 0 || current == total || current <= 5 {
                    callback(current, total, &format!("Chunk {}/{}", chunk_idx + 1, num_chunks));
                }
            });

            // Clear cache after each chunk to free memory
            self.clear_bsa_cache();
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        let final_errors = Arc::try_unwrap(errors)
            .unwrap_or_else(|e| e.lock().unwrap().clone().into())
            .into_inner()
            .unwrap();

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            errors: final_errors,
            memory: None,
            timings: None,
        })
    }

    /// Process a single asset
    fn process_asset(&self, asset: &Asset) -> Result<()> {
        let op_type = OpType::from_i32(asset.op_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown operation type: {}", asset.op_type))?;

        match op_type {
            OpType::Copy => self.process_copy(asset),
            OpType::New => self.process_new(asset),
            OpType::Patch => self.process_patch(asset),
            OpType::OggEnc2 => self.process_ogg_resample(asset),
            OpType::AudioEnc => self.process_audio_encode(asset),
        }
    }

    /// Copy operation: copy file from source to target
    fn process_copy(&self, asset: &Asset) -> Result<()> {
        let source_data = self.get_source_data(asset)?;

        if self.dry_run {
            return Ok(());
        }

        self.write_to_target(asset, &source_data)?;
        Ok(())
    }

    /// New operation: copy new file from MPI package
    fn process_new(&self, asset: &Asset) -> Result<()> {
        // New files come from the MPI package directory
        // Normalize path separators for cross-platform
        let normalized_path = asset.source_path.replace('\\', "/");
        let source_path = self.mpi_dir.join(&normalized_path);

        // Try case-insensitive lookup for Linux
        let actual_path = find_file_case_insensitive(&source_path)
            .ok_or_else(|| anyhow::anyhow!("New file not found in MPI: {}", source_path.display()))?;

        let source_data = fs::read(&actual_path)
            .with_context(|| format!("Failed to read: {}", actual_path.display()))?;

        if self.dry_run {
            return Ok(());
        }

        self.write_to_target(asset, &source_data)?;
        Ok(())
    }

    /// Patch operation: apply xdelta patch to source
    fn process_patch(&self, asset: &Asset) -> Result<()> {
        let source_data = self.get_source_data(asset)?;

        // Patch file is named based on TARGET path + ".xd3", not source path!
        let target_file = asset.target_path.as_deref()
            .unwrap_or(&asset.source_path);
        let patch_file_name = format!("{}.xd3", target_file).replace('\\', "/");
        let patch_path = self.mpi_dir.join(&patch_file_name);

        // Try case-insensitive lookup for Linux
        let actual_patch_path = find_file_case_insensitive(&patch_path)
            .ok_or_else(|| anyhow::anyhow!("Patch file not found: {}", patch_path.display()))?;

        let patch_data = fs::read(&actual_patch_path)
            .with_context(|| format!("Failed to read patch: {}", actual_patch_path.display()))?;

        if self.dry_run {
            return Ok(());
        }

        let patched = self.xdelta.apply_patch_from_bytes(&source_data, &patch_data)?;
        self.write_to_target(asset, &patched)?;
        Ok(())
    }

    /// OggEnc2 operation: decode OGG, resample, re-encode
    fn process_ogg_resample(&self, asset: &Asset) -> Result<()> {
        let source_data = self.get_source_data(asset)?;

        if self.dry_run {
            return Ok(());
        }

        // Create audio processor with manifest params (e.g. "-f:24000 -q:5")
        let audio_processor = AudioProcessor::new().with_params(&asset.params);
        let processed = audio_processor.process_ogg_resample(&source_data)?;
        self.write_to_target(asset, &processed)?;
        Ok(())
    }

    /// AudioEnc operation: convert audio format
    fn process_audio_encode(&self, asset: &Asset) -> Result<()> {
        let source_data = self.get_source_data(asset)?;

        // Parse output format from params or target path
        let output_format = self.get_audio_output_format(asset)?;

        // Determine input format from source path
        let input_format = Path::new(&asset.source_path)
            .extension()
            .and_then(|e| e.to_str());

        if self.dry_run {
            return Ok(());
        }

        // Create audio processor with manifest params (e.g. "-f:24000 -q:5")
        let audio_processor = AudioProcessor::new().with_params(&asset.params);
        let processed = audio_processor.process_audio_conversion(
            &source_data,
            input_format,
            output_format,
        )?;
        self.write_to_target(asset, &processed)?;
        Ok(())
    }

    /// Get source data, either from SQLite cache, BSA, or directory
    fn get_source_data(&self, asset: &Asset) -> Result<Vec<u8>> {
        if self.resolver.is_bsa_location(asset.source_loc) {
            let bsa_path = self.resolver.get_bsa_path(asset.source_loc)?;

            // First, check the SQLite cache
            if let Some(data) = self.bsa_cache.get(&bsa_path, &asset.source_path)? {
                return Ok(data);
            }

            // Not in cache - extract directly (fallback, slower path)
            let mut handler = self.bsa_handler.lock().unwrap();
            handler.extract_file(&bsa_path, &asset.source_path)
        } else {
            // Read from directory
            let source_dir = self.resolver.resolve_path(asset.source_loc)?;
            let normalized_path = asset.source_path.replace('\\', "/");
            let source_path = source_dir.join(&normalized_path);

            // Try case-insensitive lookup for Linux
            let actual_path = find_file_case_insensitive(&source_path)
                .ok_or_else(|| anyhow::anyhow!("Source file not found: {}", source_path.display()))?;

            fs::read(&actual_path)
                .with_context(|| format!("Failed to read: {}", actual_path.display()))
        }
    }

    /// Get target path for an asset
    fn get_target_path(&self, asset: &Asset) -> Result<PathBuf> {
        let target_dir = self.resolver.get_directory_path(asset.target_loc)?;
        let target_file = asset.target_path.as_deref()
            .unwrap_or(&asset.source_path);

        // Normalize path separators for Linux
        let normalized = target_file.replace('\\', "/");
        Ok(target_dir.join(normalized))
    }

    /// Write data to target location
    fn write_to_target(&self, asset: &Asset, data: &[u8]) -> Result<()> {
        let writer = self.bsa_writer.lock().unwrap();
        if writer.is_bsa_location(asset.target_loc) {
            drop(writer); // Release lock before acquiring again
            // Add to BSA writer for later packing
            let target_file = asset.target_path.as_deref()
                .unwrap_or(&asset.source_path);
            // Normalize path for BSA
            let normalized = target_file.replace('\\', "/");
            let writer = self.bsa_writer.lock().unwrap();
            writer.add_file(asset.target_loc, &normalized, data.to_vec())?;
        } else {
            drop(writer); // Release lock
            // Write directly to target directory
            let target_path = self.get_target_path(asset)?;

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target_path, data)?;
        }

        Ok(())
    }

    /// Finalize installation by writing all BSA archives
    pub fn finalize_bsas(&self) -> Result<(usize, usize)> {
        if self.dry_run {
            println!("\n[DRY RUN] Would write BSA archives to: {}", self.dest_dir.display());
            return Ok((0, 0));
        }

        let mut writer = self.bsa_writer.lock().unwrap();
        writer.write_all(&self.dest_dir)
    }

    /// Finalize BSAs with progress callback for GUI
    /// callback(current, total, bsa_name)
    pub fn finalize_bsas_with_callback<F>(&self, callback: F) -> Result<(usize, usize)>
    where
        F: Fn(usize, usize, &str) + Sync,
    {
        if self.dry_run {
            return Ok((0, 0));
        }

        let mut writer = self.bsa_writer.lock().unwrap();
        writer.write_all_with_callback(&self.dest_dir, callback)
    }

    /// Get audio output format from asset params or target path
    fn get_audio_output_format(&self, asset: &Asset) -> Result<AudioFormat> {
        // Try to get from target path extension
        let target_file = asset.target_path.as_deref()
            .unwrap_or(&asset.source_path);

        if let Some(ext) = Path::new(target_file).extension().and_then(|e| e.to_str()) {
            if let Some(format) = AudioFormat::from_extension(ext) {
                return Ok(format);
            }
        }

        // Default to OGG
        Ok(AudioFormat::Ogg)
    }

    /// Clear BSA cache to free memory
    pub fn clear_cache(&self) {
        let mut handler = self.bsa_handler.lock().unwrap();
        handler.clear_cache();
    }

    // ========================================================================
    // PRODUCER-CONSUMER MODE - Overlapped extraction and processing
    // ========================================================================

    /// Process assets using sequential BSA extraction with full-thread parallel decompression.
    ///
    /// Benchmarking shows this is as fast as all-parallel extraction but uses ~3x less RAM:
    /// - Sequential: ~1.4 GB peak for full extraction
    /// - All-parallel: ~4.7 GB peak for same work
    /// - Throughput is identical (~7.7 GB/s) because decompression is CPU-bound
    ///
    /// Pipeline per BSA:
    /// 1. Open BSA, scan for matching files (fast)
    /// 2. Decompress matched files in parallel across all CPU cores
    /// 3. Process each decompressed file (patch/audio/copy) immediately
    /// 4. Write result to BSA staging or output directory
    /// 5. Drop data, move to next BSA
    pub fn process_assets_streaming(&self, assets: &[Asset]) -> Result<ProcessingStats> {
        let monitor = MemoryMonitor::start(std::time::Duration::from_millis(250));
        print_ram_status("Start");

        info!("Using sequential BSA pipeline (full-thread decompression per BSA)");

        // === Step 1: Group assets by source BSA ===
        let mut bsa_assets: HashMap<PathBuf, Vec<&Asset>> = HashMap::new();
        let mut dir_assets: Vec<&Asset> = Vec::new();

        for asset in assets {
            if self.resolver.is_bsa_location(asset.source_loc) {
                if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                    bsa_assets.entry(bsa_path).or_default().push(asset);
                }
            } else {
                dir_assets.push(asset);
            }
        }

        let mut bsa_entries: Vec<_> = bsa_assets.into_iter().collect();
        // Sort by size descending - process largest BSAs first for better progress visibility
        bsa_entries.sort_by(|a, b| {
            let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
            let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
            size_b.cmp(&size_a)
        });

        let num_bsas = bsa_entries.len();
        let total_assets = assets.len();

        println!("\nSequential BSA pipeline ({} source BSAs, {} threads):",
            num_bsas,
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
        for (i, (path, assets_list)) in bsa_entries.iter().take(8).enumerate() {
            let size = fs::metadata(path).map(|m| m.len() / 1024 / 1024).unwrap_or(0);
            let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            println!("  {}. {} ({} MB, {} files)", i + 1, name, size, assets_list.len());
        }
        if bsa_entries.len() > 8 {
            println!("  ... and {} more BSAs", bsa_entries.len() - 8);
        }
        println!("  Directory assets: {}", dir_assets.len());

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let op_timings = AtomicOpTimings::new();

        let pb = ProgressBar::new(total_assets as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"));

        // === Step 2: Process BSAs sequentially, decompress+process in parallel ===
        for (bsa_idx, (bsa_path, assets_for_bsa)) in bsa_entries.iter().enumerate() {
            let bsa_name = bsa_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            pb.set_message(format!("BSA {}/{}: {} ({} files)",
                bsa_idx + 1, num_bsas, bsa_name, assets_for_bsa.len()));

            // Build lookup: normalized_path -> Vec<Asset>
            let mut path_to_assets: HashMap<String, Vec<&Asset>> = HashMap::new();
            for asset in assets_for_bsa {
                let normalized = asset.source_path.replace('/', "\\").to_lowercase();
                path_to_assets.entry(normalized).or_default().push(asset);
            }

            // Open BSA and collect matching file references
            use ba2::tes4::{Archive as TesArchive, ArchiveOptions, FileCompressionOptions as FcOpts};
            use ba2::Reader as BsaReader;
            let (archive, options): (TesArchive, ArchiveOptions) = match TesArchive::read(bsa_path.as_path()) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Failed to open {}: {}", bsa_name, e);
                    let missing_count = assets_for_bsa.len();
                    failed.fetch_add(missing_count, Ordering::Relaxed);
                    pb.inc(missing_count as u64);
                    continue;
                }
            };
            let compression_options = FcOpts::from(&options);

            // Phase 1: Scan archive for matching files (fast, single-threaded)
            let mut matched: Vec<(Vec<&Asset>, &ba2::tes4::File<'_>)> = Vec::new();
            {
                use ba2::ByteSlice;
                for (dir_key, folder) in archive.iter() {
                    if path_to_assets.is_empty() {
                        break;
                    }

                    let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();

                    for (file_key, file) in folder.iter() {
                        let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
                        let full_path = if dir_name.is_empty() || dir_name == "." {
                            file_name
                        } else {
                            format!("{}\\{}", dir_name, file_name)
                        };

                        if let Some(assets_needing) = path_to_assets.remove(&full_path) {
                            matched.push((assets_needing, file));
                        }
                    }
                }
            }

            // Phase 2: Decompress + process in parallel (all CPU cores)
            let success_ref = &success;
            let failed_ref = &failed;
            let errors_ref = &errors;
            let pb_ref = &pb;
            let timings_ref = &op_timings;

            matched.par_iter().for_each(|(assets_needing, file)| {
                // Decompress (CPU-intensive, benefits from parallelism)
                let decompress_start = Instant::now();
                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&compression_options) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(e) => {
                            for asset in assets_needing {
                                failed_ref.fetch_add(1, Ordering::Relaxed);
                                let error_msg = format!("{} (op={}): decompression failed: {}",
                                    asset.source_path, asset.op_type, e);
                                warn!("{}", error_msg);
                                if let Ok(mut errs) = errors_ref.lock() {
                                    if errs.len() < 100 { errs.push(error_msg); }
                                }
                                pb_ref.inc(1);
                            }
                            return;
                        }
                    }
                };
                timings_ref.record_decompress(decompress_start.elapsed().as_nanos() as u64);

                // Process each asset that needs this file
                for asset in assets_needing {
                    let op_start = Instant::now();
                    let result = self.process_asset_with_data(asset, &data);
                    if let Some(op) = OpType::from_i32(asset.op_type) {
                        timings_ref.record(op, op_start.elapsed().as_nanos() as u64);
                    }
                    match result {
                        Ok(_) => { success_ref.fetch_add(1, Ordering::Relaxed); }
                        Err(e) => {
                            failed_ref.fetch_add(1, Ordering::Relaxed);
                            let error_msg = format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                            warn!("{}", error_msg);
                            if let Ok(mut errs) = errors_ref.lock() {
                                if errs.len() < 100 { errs.push(error_msg); }
                            }
                        }
                    }
                    pb_ref.inc(1);
                }
                // data is dropped here - keeps RAM low
            });

            // Log RAM after each BSA
            if (bsa_idx + 1) % 5 == 0 || bsa_idx == 0 {
                let peak = monitor.peak_rss_mb();
                let current = monitor.current_rss_mb();
                pb.set_message(format!("BSA {}/{} done | RAM: {:.0} MB (peak {:.0} MB)",
                    bsa_idx + 1, num_bsas, current, peak));
            }
        }

        // === Step 3: Process directory assets ===
        if !dir_assets.is_empty() {
            pb.set_message(format!("Processing {} directory assets...", dir_assets.len()));
            dir_assets.par_iter().for_each(|asset| {
                let op_start = Instant::now();
                let result = self.process_asset(asset);
                if let Some(op) = OpType::from_i32(asset.op_type) {
                    op_timings.record(op, op_start.elapsed().as_nanos() as u64);
                }
                match result {
                    Ok(_) => { success.fetch_add(1, Ordering::Relaxed); }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg = format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("{}", error_msg);
                        if let Ok(mut errs) = errors.lock() {
                            errs.push(error_msg);
                        }
                    }
                }
                pb.inc(1);
            });
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        pb.finish_with_message(format!("Done: {} success, {} failed", final_success, final_failed));

        let memory_report = monitor.stop();
        print_ram_status("End");

        let final_errors = Arc::try_unwrap(errors)
            .unwrap_or_else(|e| e.lock().unwrap().clone().into())
            .into_inner()
            .unwrap();

        if !final_errors.is_empty() {
            println!("\nErrors ({}):", final_errors.len());
            for (i, err) in final_errors.iter().take(20).enumerate() {
                println!("  {}. {}", i + 1, err);
            }
            if final_errors.len() > 20 {
                println!("  ... and {} more", final_errors.len() - 20);
            }
        }

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            errors: final_errors,
            memory: Some(memory_report),
            timings: Some(op_timings.snapshot()),
        })
    }

    /// Process a single asset with pre-loaded source data
    fn process_asset_with_data(&self, asset: &Asset, source_data: &[u8]) -> Result<()> {
        let op_type = OpType::from_i32(asset.op_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown operation type: {}", asset.op_type))?;

        match op_type {
            OpType::Copy => {
                if !self.dry_run {
                    self.write_to_target(asset, source_data)?;
                }
                Ok(())
            }
            OpType::New => self.process_new(asset),
            OpType::Patch => {
                let target_file = asset.target_path.as_deref()
                    .unwrap_or(&asset.source_path);
                let patch_file_name = format!("{}.xd3", target_file).replace('\\', "/");
                let patch_path = self.mpi_dir.join(&patch_file_name);

                let actual_patch_path = find_file_case_insensitive(&patch_path)
                    .ok_or_else(|| anyhow::anyhow!("Patch file not found: {}", patch_path.display()))?;

                let patch_data = fs::read(&actual_patch_path)
                    .with_context(|| format!("Failed to read patch: {}", actual_patch_path.display()))?;

                if !self.dry_run {
                    let patched = self.xdelta.apply_patch_from_bytes(source_data, &patch_data)?;
                    self.write_to_target(asset, &patched)?;
                }
                Ok(())
            }
            OpType::OggEnc2 => {
                if !self.dry_run {
                    let audio_processor = AudioProcessor::new().with_params(&asset.params);
                    let processed = audio_processor.process_ogg_resample(source_data)?;
                    self.write_to_target(asset, &processed)?;
                }
                Ok(())
            }
            OpType::AudioEnc => {
                let output_format = self.get_audio_output_format(asset)?;
                let input_format = Path::new(&asset.source_path)
                    .extension()
                    .and_then(|e| e.to_str());

                if !self.dry_run {
                    let audio_processor = AudioProcessor::new().with_params(&asset.params);
                    let processed = audio_processor.process_audio_conversion(
                        source_data,
                        input_format,
                        output_format,
                    )?;
                    self.write_to_target(asset, &processed)?;
                }
                Ok(())
            }
        }
    }

    /// Process assets with sequential BSA pipeline and progress callback for GUI.
    /// Same algorithm as `process_assets_streaming` but with GUI progress callbacks.
    pub fn process_assets_streaming_with_callback<F>(&self, assets: &[Asset], callback: F) -> Result<ProcessingStats>
    where
        F: Fn(usize, usize, &str) + Send + Sync,
    {
        let callback = Arc::new(callback);
        info!("Using sequential BSA pipeline with GUI callback");

        // === Step 1: Group assets by source BSA ===
        let mut bsa_assets: HashMap<PathBuf, Vec<&Asset>> = HashMap::new();
        let mut dir_assets: Vec<&Asset> = Vec::new();

        for asset in assets {
            if self.resolver.is_bsa_location(asset.source_loc) {
                if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                    bsa_assets.entry(bsa_path).or_default().push(asset);
                }
            } else {
                dir_assets.push(asset);
            }
        }

        let mut bsa_entries: Vec<_> = bsa_assets.into_iter().collect();
        bsa_entries.sort_by(|a, b| {
            let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
            let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
            size_b.cmp(&size_a)
        });

        let total = assets.len();
        let num_bsas = bsa_entries.len();
        let processed = AtomicUsize::new(0);

        callback(0, total, &format!("Processing {} BSAs...", num_bsas));

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // === Step 2: Process BSAs sequentially, decompress+process in parallel ===
        for (bsa_idx, (bsa_path, assets_for_bsa)) in bsa_entries.iter().enumerate() {
            let bsa_name = bsa_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            callback(processed.load(Ordering::Relaxed), total,
                &format!("BSA {}/{}: {} ({} files)", bsa_idx + 1, num_bsas, bsa_name, assets_for_bsa.len()));

            let mut path_to_assets: HashMap<String, Vec<&Asset>> = HashMap::new();
            for asset in assets_for_bsa {
                let normalized = asset.source_path.replace('/', "\\").to_lowercase();
                path_to_assets.entry(normalized).or_default().push(asset);
            }

            use ba2::tes4::{Archive as TesArchive, ArchiveOptions, FileCompressionOptions as FcOpts};
            use ba2::Reader as BsaReader;
            let (archive, options): (TesArchive, ArchiveOptions) = match TesArchive::read(bsa_path.as_path()) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Failed to open {}: {}", bsa_name, e);
                    let missing_count = assets_for_bsa.len();
                    failed.fetch_add(missing_count, Ordering::Relaxed);
                    processed.fetch_add(missing_count, Ordering::Relaxed);
                    continue;
                }
            };
            let compression_options = FcOpts::from(&options);

            let mut matched: Vec<(Vec<&Asset>, &ba2::tes4::File<'_>)> = Vec::new();
            {
                use ba2::ByteSlice;
                for (dir_key, folder) in archive.iter() {
                    if path_to_assets.is_empty() { break; }
                    let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();
                    for (file_key, file) in folder.iter() {
                        let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
                        let full_path = if dir_name.is_empty() || dir_name == "." {
                            file_name
                        } else {
                            format!("{}\\{}", dir_name, file_name)
                        };
                        if let Some(assets_needing) = path_to_assets.remove(&full_path) {
                            matched.push((assets_needing, file));
                        }
                    }
                }
            }

            let success_ref = &success;
            let failed_ref = &failed;
            let errors_ref = &errors;
            let processed_ref = &processed;
            let callback_ref = &callback;

            matched.par_iter().for_each(|(assets_needing, file)| {
                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&compression_options) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(e) => {
                            for asset in assets_needing {
                                failed_ref.fetch_add(1, Ordering::Relaxed);
                                let error_msg = format!("{} (op={}): decompression failed: {}",
                                    asset.source_path, asset.op_type, e);
                                warn!("{}", error_msg);
                                if let Ok(mut errs) = errors_ref.lock() {
                                    if errs.len() < 100 { errs.push(error_msg); }
                                }
                                processed_ref.fetch_add(1, Ordering::Relaxed);
                            }
                            return;
                        }
                    }
                };

                for asset in assets_needing {
                    let result = self.process_asset_with_data(asset, &data);
                    match result {
                        Ok(_) => { success_ref.fetch_add(1, Ordering::Relaxed); }
                        Err(e) => {
                            failed_ref.fetch_add(1, Ordering::Relaxed);
                            let error_msg = format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                            warn!("{}", error_msg);
                            if let Ok(mut errs) = errors_ref.lock() {
                                if errs.len() < 100 { errs.push(error_msg); }
                            }
                        }
                    }
                    let current = processed_ref.fetch_add(1, Ordering::Relaxed) + 1;
                    if current % 100 == 0 || current <= 5 {
                        callback_ref(current, total, &format!("BSA {}/{}", bsa_idx + 1, num_bsas));
                    }
                }
            });
        }

        // === Step 3: Process directory assets ===
        if !dir_assets.is_empty() {
            callback(processed.load(Ordering::Relaxed), total, "Processing directory assets...");

            dir_assets.par_iter().for_each(|asset| {
                let result = self.process_asset(asset);
                match result {
                    Ok(_) => { success.fetch_add(1, Ordering::Relaxed); }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg = format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("{}", error_msg);
                        if let Ok(mut errs) = errors.lock() {
                            errs.push(error_msg);
                        }
                    }
                }

                let current = processed.fetch_add(1, Ordering::Relaxed) + 1;
                if current % 100 == 0 || current == total {
                    callback(current, total, "Processing loose files");
                }
            });
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        if final_failed > 0 {
            warn!("Processing complete: {} succeeded, {} failed", final_success, final_failed);
        }

        let final_errors = Arc::try_unwrap(errors)
            .unwrap_or_else(|e| e.lock().unwrap().clone().into())
            .into_inner()
            .unwrap();

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            errors: final_errors,
            memory: None,
            timings: None,
        })
    }

}

/// Per-operation timing breakdown
#[derive(Debug, Default, Clone)]
pub struct OpTimings {
    /// Total wall-clock nanoseconds spent per op type (summed across threads)
    pub copy_ns: u64,
    pub new_ns: u64,
    pub patch_ns: u64,
    pub ogg_ns: u64,
    pub audio_ns: u64,
    pub decompress_ns: u64,
    /// Counts per op type
    pub copy_count: usize,
    pub new_count: usize,
    pub patch_count: usize,
    pub ogg_count: usize,
    pub audio_count: usize,
    pub decompress_count: usize,
}

impl std::fmt::Display for OpTimings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_line = |name: &str, ns: u64, count: usize| -> String {
            if count == 0 {
                return String::new();
            }
            let secs = ns as f64 / 1_000_000_000.0;
            let avg_ms = if count > 0 { ns as f64 / count as f64 / 1_000_000.0 } else { 0.0 };
            format!("    {:<14} {:>8} ops  {:>8.1}s thread-time  {:>6.1}ms avg\n", name, count, secs, avg_ms)
        };

        write!(f, "{}{}{}{}{}{}",
            format_line("Decompress", self.decompress_ns, self.decompress_count),
            format_line("Copy", self.copy_ns, self.copy_count),
            format_line("New", self.new_ns, self.new_count),
            format_line("Patch", self.patch_ns, self.patch_count),
            format_line("OggEnc2", self.ogg_ns, self.ogg_count),
            format_line("AudioEnc", self.audio_ns, self.audio_count),
        )
    }
}

/// Thread-safe atomic counters for operation timing
struct AtomicOpTimings {
    copy_ns: AtomicU64,
    new_ns: AtomicU64,
    patch_ns: AtomicU64,
    ogg_ns: AtomicU64,
    audio_ns: AtomicU64,
    decompress_ns: AtomicU64,
    copy_count: AtomicUsize,
    new_count: AtomicUsize,
    patch_count: AtomicUsize,
    ogg_count: AtomicUsize,
    audio_count: AtomicUsize,
    decompress_count: AtomicUsize,
}

impl AtomicOpTimings {
    fn new() -> Self {
        Self {
            copy_ns: AtomicU64::new(0),
            new_ns: AtomicU64::new(0),
            patch_ns: AtomicU64::new(0),
            ogg_ns: AtomicU64::new(0),
            audio_ns: AtomicU64::new(0),
            decompress_ns: AtomicU64::new(0),
            copy_count: AtomicUsize::new(0),
            new_count: AtomicUsize::new(0),
            patch_count: AtomicUsize::new(0),
            ogg_count: AtomicUsize::new(0),
            audio_count: AtomicUsize::new(0),
            decompress_count: AtomicUsize::new(0),
        }
    }

    fn record(&self, op: OpType, elapsed_ns: u64) {
        match op {
            OpType::Copy => { self.copy_ns.fetch_add(elapsed_ns, Ordering::Relaxed); self.copy_count.fetch_add(1, Ordering::Relaxed); }
            OpType::New => { self.new_ns.fetch_add(elapsed_ns, Ordering::Relaxed); self.new_count.fetch_add(1, Ordering::Relaxed); }
            OpType::Patch => { self.patch_ns.fetch_add(elapsed_ns, Ordering::Relaxed); self.patch_count.fetch_add(1, Ordering::Relaxed); }
            OpType::OggEnc2 => { self.ogg_ns.fetch_add(elapsed_ns, Ordering::Relaxed); self.ogg_count.fetch_add(1, Ordering::Relaxed); }
            OpType::AudioEnc => { self.audio_ns.fetch_add(elapsed_ns, Ordering::Relaxed); self.audio_count.fetch_add(1, Ordering::Relaxed); }
        }
    }

    fn record_decompress(&self, elapsed_ns: u64) {
        self.decompress_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        self.decompress_count.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> OpTimings {
        OpTimings {
            copy_ns: self.copy_ns.load(Ordering::Relaxed),
            new_ns: self.new_ns.load(Ordering::Relaxed),
            patch_ns: self.patch_ns.load(Ordering::Relaxed),
            ogg_ns: self.ogg_ns.load(Ordering::Relaxed),
            audio_ns: self.audio_ns.load(Ordering::Relaxed),
            decompress_ns: self.decompress_ns.load(Ordering::Relaxed),
            copy_count: self.copy_count.load(Ordering::Relaxed),
            new_count: self.new_count.load(Ordering::Relaxed),
            patch_count: self.patch_count.load(Ordering::Relaxed),
            ogg_count: self.ogg_count.load(Ordering::Relaxed),
            audio_count: self.audio_count.load(Ordering::Relaxed),
            decompress_count: self.decompress_count.load(Ordering::Relaxed),
        }
    }
}

/// Statistics from processing
#[derive(Debug, Default)]
pub struct ProcessingStats {
    pub success: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub memory: Option<MemoryReport>,
    pub timings: Option<OpTimings>,
}

impl ProcessingStats {
    pub fn print_summary(&self) {
        println!("\nProcessing Summary:");
        println!("  Successful: {}", self.success);
        println!("  Failed: {}", self.failed);

        if let Some(ref timings) = self.timings {
            println!("\n  Operation Breakdown (thread-time = sum of per-thread durations):");
            print!("{}", timings);
        }

        if let Some(ref mem) = self.memory {
            println!("\n  Memory Usage:");
            println!("    Initial RSS:       {:.0} MB", mem.initial_rss_mb);
            println!("    Peak RSS:          {:.0} MB", mem.peak_rss_mb);
            println!("    Delta (peak-init): {:.0} MB", mem.delta_mb);
            println!("    Final RSS:         {:.0} MB", mem.final_rss_mb);
            println!("    System Available:  {:.1} GB / {:.1} GB total",
                mem.system_available_mb / 1024.0, mem.system_total_mb / 1024.0);
        }

        if !self.errors.is_empty() {
            println!("\nErrors:");
            for error in &self.errors {
                println!("  - {}", error);
            }
        }
    }
}
