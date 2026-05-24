use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{info, warn};

use super::paths::get_chunk_count_for_ram;
use super::stats::{AtomicOpTimings, OpType, ProcessingStats};
use crate::models::{Asset, Location};
use crate::services::{
    print_ram_status, BsaCache, BsaHandler, BsaWriterManager, LocationResolver, MemoryMonitor,
    MpiStore, XdeltaManager,
};

/// Processes assets from TTW manifest (thread-safe)
pub struct AssetProcessor {
    pub(super) resolver: Arc<LocationResolver>,
    pub(super) bsa_handler: Arc<Mutex<BsaHandler>>,
    pub(super) bsa_writer: Arc<Mutex<BsaWriterManager>>,
    /// SQLite-based cache for pre-extracted BSA files (disk-based, low RAM usage)
    pub(super) bsa_cache: Arc<BsaCache>,
    pub(super) xdelta: Arc<XdeltaManager>,
    /// In-memory MPI package (if loaded). Used for instant file lookups.
    pub(super) mpi_store: Option<Arc<MpiStore>>,
    pub(super) mpi_dir: PathBuf,
    pub(super) dest_dir: PathBuf,
    pub(super) dry_run: bool,
}

fn push_limited_error(errors: &Mutex<Vec<String>>, error_msg: String, limit: usize) {
    match errors.lock() {
        Ok(mut errs) => {
            if errs.len() < limit {
                errs.push(error_msg);
            }
        }
        Err(_) => warn!("Failed to record asset error because the error list lock is poisoned"),
    }
}

fn take_errors(errors: Arc<Mutex<Vec<String>>>) -> Vec<String> {
    match Arc::try_unwrap(errors) {
        Ok(mutex) => mutex
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        Err(errors) => errors
            .lock()
            .map(|errs| errs.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone()),
    }
}

impl AssetProcessor {
    pub fn new(
        resolver: LocationResolver,
        xdelta: XdeltaManager,
        mpi_dir: PathBuf,
        dest_dir: PathBuf,
        locations: &[Location],
        bsa_targets: &[Location], // Separate BSA target locations (may come from different profile)
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
            println!(
                "\nRegistering {} BSA targets with manifest flags:",
                bsa_targets.len()
            );
            for bsa_loc in bsa_targets {
                let name = bsa_loc.name.as_deref().unwrap_or("");
                let value = bsa_loc.value.as_deref().unwrap_or("");

                // Find matching location index in the main profile
                let name_lower = name.to_lowercase();
                let location_idx =
                    bsa_name_to_index
                        .get(&name_lower)
                        .copied()
                        .unwrap_or_else(|| {
                            // Try without "NEW " prefix
                            let stripped = name
                                .strip_prefix("NEW ")
                                .map(|s| s.to_lowercase())
                                .unwrap_or_else(|| name_lower.clone());
                            bsa_name_to_index.get(&stripped).copied().unwrap_or(-1)
                        });

                if location_idx >= 0 {
                    // Extract filename from Value path for the output name
                    let normalized = value.replace('\\', "/");
                    let bsa_filename = normalized.rsplit('/').next().unwrap_or(&normalized);

                    println!(
                        "  BSA: {} -> loc[{}] type={:?} flags={:?} types={:?} compressed={:?}",
                        bsa_filename,
                        location_idx,
                        bsa_loc.archive_type,
                        bsa_loc.archive_flags,
                        bsa_loc.files_flags,
                        bsa_loc.archive_compressed
                    );

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
                    bsa_writer.register_bsa(
                        i as i32,
                        name,
                        loc.archive_type,
                        loc.archive_flags,
                        loc.files_flags,
                        loc.archive_compressed,
                    )?;
                }
                // Format 2: Type 2 with .bsa in VALUE (original MPI format)
                else if loc.loc_type == 2 && is_bsa_value {
                    let normalized = value.replace('\\', "/");
                    let bsa_name = normalized.rsplit('/').next().unwrap_or(&normalized);
                    bsa_writer.register_bsa(
                        i as i32,
                        bsa_name,
                        loc.archive_type,
                        loc.archive_flags,
                        loc.files_flags,
                        loc.archive_compressed,
                    )?;
                }
                // Format 3: Type 1 with "NEW " prefix
                else if loc.loc_type == 1 && has_new_prefix && is_bsa_value {
                    bsa_writer.register_bsa(
                        i as i32,
                        name,
                        loc.archive_type,
                        loc.archive_flags,
                        loc.files_flags,
                        loc.archive_compressed,
                    )?;
                }
            }
        }

        // Create SQLite-based cache in the output directory
        let bsa_cache =
            BsaCache::new_at(dest_dir.clone()).context("Failed to create SQLite BSA cache")?;

        Ok(Self {
            resolver: Arc::new(resolver),
            bsa_handler: Arc::new(Mutex::new(BsaHandler::new())),
            bsa_writer: Arc::new(Mutex::new(bsa_writer)),
            bsa_cache: Arc::new(bsa_cache),
            xdelta: Arc::new(xdelta),
            mpi_store: None,
            mpi_dir,
            dest_dir,
            dry_run: false,
        })
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Attach an in-memory MPI store for instant file lookups.
    pub fn with_mpi_store(mut self, store: MpiStore) -> Self {
        self.mpi_store = Some(Arc::new(store));
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
        use ba2::tes4::{
            Archive, ArchiveKey, DirectoryKey, File as BsaFile, FileCompressionOptions,
        };
        use ba2::{ByteSlice, Reader};
        use std::collections::HashSet;

        // Open BSA once - get options for proper decompression
        let (archive, options): (Archive, _) = Archive::read(bsa_path).with_context(|| {
            format!(
                "Failed to open BSA for pre-extraction: {}",
                bsa_path.display()
            )
        })?;
        let compression_options: FileCompressionOptions = (&options).into();

        // Build set of needed paths (normalized to lowercase with backslashes)
        let needed: HashSet<String> = file_paths
            .iter()
            .map(|p| p.replace('/', "\\").to_lowercase())
            .collect();

        // Build lookup map for original path casing
        let path_lookup: HashMap<String, &str> = file_paths
            .iter()
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
                    let file_name =
                        String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
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
                        let original_path = path_lookup
                            .get(&full_path)
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
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );

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
                        bsa_files_needed
                            .entry(bsa_path)
                            .or_default()
                            .push(&asset.source_path);
                    }
                }
            }

            // Pre-extract this chunk's BSA files to SQLite (parallel)
            bsa_files_needed
                .par_iter()
                .for_each(|(bsa_path, file_paths)| {
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
                        let error_msg =
                            format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("Asset failed: {}", error_msg);
                        push_limited_error(&errors, error_msg, 100);
                    }
                }

                pb.inc(1);
                let s = success.load(Ordering::Relaxed);
                let f = failed.load(Ordering::Relaxed);
                pb.set_message(format!(
                    "OK:{} Fail:{} (chunk {}/{})",
                    s,
                    f,
                    chunk_idx + 1,
                    num_chunks
                ));
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

        let final_errors = take_errors(errors);

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            bsa_success: 0,
            bsa_failed: 0,
            errors: final_errors,
            memory: Some(memory_report),
            timings: None,
        })
    }

    /// Process assets with a progress callback for GUI usage
    /// callback(current, total, message)
    pub fn process_assets_with_callback<F>(
        &self,
        assets: &[Asset],
        callback: F,
    ) -> Result<ProcessingStats>
    where
        F: Fn(usize, usize, &str) + Send + Sync,
    {
        let total = assets.len();
        let callback = Arc::new(callback);

        // === Determine chunk count based on available RAM ===
        let num_chunks = get_chunk_count_for_ram();
        let chunk_size = total.div_ceil(num_chunks);

        callback(
            0,
            total,
            &format!(
                "Processing in {} chunk(s) ({} assets/chunk)...",
                num_chunks, chunk_size
            ),
        );

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let processed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // Process each chunk
        for (chunk_idx, chunk) in assets.chunks(chunk_size).enumerate() {
            callback(
                processed.load(Ordering::Relaxed),
                total,
                &format!(
                    "Chunk {}/{} ({} files): Pre-extracting...",
                    chunk_idx + 1,
                    num_chunks,
                    chunk.len()
                ),
            );

            // Pre-extract BSA files for this chunk only
            let mut bsa_files_needed: HashMap<PathBuf, Vec<&str>> = HashMap::new();
            for asset in chunk {
                if self.resolver.is_bsa_location(asset.source_loc) {
                    if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                        bsa_files_needed
                            .entry(bsa_path)
                            .or_default()
                            .push(&asset.source_path);
                    }
                }
            }

            // Pre-extract this chunk's BSA files to SQLite (parallel)
            bsa_files_needed
                .par_iter()
                .for_each(|(bsa_path, file_paths)| {
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
                        let error_msg =
                            format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("Asset failed: {}", error_msg);
                        push_limited_error(&errors, error_msg, 100);
                    }
                }

                let current = processed.fetch_add(1, Ordering::Relaxed) + 1;

                let callback_interval = if total > 1000 {
                    50
                } else if total > 100 {
                    10
                } else {
                    1
                };
                if current.is_multiple_of(callback_interval) || current == total || current <= 5 {
                    callback(
                        current,
                        total,
                        &format!("Chunk {}/{}", chunk_idx + 1, num_chunks),
                    );
                }
            });

            // Clear cache after each chunk to free memory
            self.clear_bsa_cache();
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        let final_errors = take_errors(errors);

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            bsa_success: 0,
            bsa_failed: 0,
            errors: final_errors,
            memory: None,
            timings: None,
        })
    }

    // ========================================================================
    // OVERLAPPED PIPELINE - Decompress → Process → Build BSAs concurrently
    // ========================================================================

    /// Process assets with a fully overlapped pipeline:
    ///
    /// 1. Decompressor threads open source BSAs and decompress matched files
    /// 2. Decompressed files are sent via channel to the worker pool
    /// 3. Workers immediately process each file (copy/patch/audio)
    /// 4. Processed results are written to BSA staging files
    /// 5. When an output BSA has all its files staged, a background thread builds it
    ///
    /// This overlaps decompression with audio processing with BSA building,
    /// keeping all CPU cores busy throughout the entire install.
    pub fn process_assets_streaming(&self, assets: &[Asset]) -> Result<ProcessingStats> {
        use crossbeam_channel::bounded;

        let monitor = MemoryMonitor::start(std::time::Duration::from_millis(250));
        print_ram_status("Start");

        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        info!("Using overlapped pipeline ({} CPUs)", num_cpus);

        // === Step 1: Group assets by source BSA and build readiness map ===
        let mut bsa_assets: HashMap<PathBuf, Vec<&Asset>> = HashMap::new();
        let mut dir_assets: Vec<&Asset> = Vec::new();

        // Track expected file count per output BSA for readiness detection
        let mut expected_counts: HashMap<i32, AtomicUsize> = HashMap::new();

        for asset in assets {
            if self.resolver.is_bsa_location(asset.source_loc) {
                if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                    bsa_assets.entry(bsa_path).or_default().push(asset);
                }
            } else {
                dir_assets.push(asset);
            }

            // Count files expected per output BSA
            let writer = self
                .bsa_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
            if writer.is_bsa_location(asset.target_loc) {
                expected_counts
                    .entry(asset.target_loc)
                    .or_insert_with(|| AtomicUsize::new(0))
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        // Snapshot expected counts (immutable after this point)
        let expected: HashMap<i32, usize> = expected_counts
            .iter()
            .map(|(k, v)| (*k, v.load(Ordering::Relaxed)))
            .collect();
        let staged_counts: HashMap<i32, AtomicUsize> =
            expected.keys().map(|k| (*k, AtomicUsize::new(0))).collect();
        let staged_counts = Arc::new(staged_counts);

        let mut bsa_entries: Vec<_> = bsa_assets.into_iter().collect();
        bsa_entries.sort_by(|a, b| {
            let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
            let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
            size_b.cmp(&size_a)
        });

        let num_source_bsas = bsa_entries.len();
        let total_assets = assets.len();

        println!(
            "\nOverlapped pipeline ({} source BSAs, {} output BSAs, {} threads):",
            num_source_bsas,
            expected.len(),
            num_cpus
        );
        for (i, (path, assets_list)) in bsa_entries.iter().take(8).enumerate() {
            let size = fs::metadata(path)
                .map(|m| m.len() / 1024 / 1024)
                .unwrap_or(0);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            println!(
                "  {}. {} ({} MB, {} files)",
                i + 1,
                name,
                size,
                assets_list.len()
            );
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
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );

        // === Step 2: Background BSA builder ===
        // Channel for sending ready BSA location indices to the builder thread
        let (bsa_ready_tx, bsa_ready_rx) = bounded::<i32>(32);
        let bsa_writer_clone = self.bsa_writer.clone();
        let dest_dir = self.dest_dir.clone();
        let bsa_build_success = Arc::new(AtomicUsize::new(0));
        let bsa_build_fail = Arc::new(AtomicUsize::new(0));
        let bsa_build_success_ref = bsa_build_success.clone();
        let bsa_build_fail_ref = bsa_build_fail.clone();

        let bsa_builder_handle = std::thread::Builder::new()
            .name("bsa-builder".into())
            .spawn(move || {
                for loc_idx in bsa_ready_rx {
                    let mut writer = match bsa_writer_clone.lock() {
                        Ok(writer) => writer,
                        Err(_) => {
                            warn!("BSA writer lock poisoned while building ready archive");
                            bsa_build_fail_ref.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    if let Some((bsa_name, builder)) = writer.take_builder(loc_idx) {
                        let file_count = builder.file_count();
                        let output_path = dest_dir.join(&bsa_name);
                        drop(writer); // Release lock during build

                        println!("  [BSA] Building {} ({} files)...", bsa_name, file_count);
                        match builder.build(&output_path) {
                            Ok(_) => {
                                let size_mb = fs::metadata(&output_path)
                                    .map(|m| m.len() / 1024 / 1024)
                                    .unwrap_or(0);
                                println!("  [BSA] {} ... OK ({} MB)", bsa_name, size_mb);
                                bsa_build_success_ref.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                warn!("BSA build failed: {} - {}", bsa_name, e);
                                println!("  [BSA] {} ... FAILED: {}", bsa_name, e);
                                bsa_build_fail_ref.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            })
            .context("Failed to spawn BSA builder thread")?;

        // === Step 3: Process source BSAs - decompress + process, check readiness ===
        for (bsa_idx, (bsa_path, assets_for_bsa)) in bsa_entries.iter().enumerate() {
            let bsa_name = bsa_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            pb.set_message(format!(
                "BSA {}/{}: {} ({} files)",
                bsa_idx + 1,
                num_source_bsas,
                bsa_name,
                assets_for_bsa.len()
            ));

            // Build lookup: normalized_path -> Vec<Asset>
            let mut path_to_assets: HashMap<String, Vec<&Asset>> = HashMap::new();
            for asset in assets_for_bsa {
                let normalized = asset.source_path.replace('/', "\\").to_lowercase();
                path_to_assets.entry(normalized).or_default().push(asset);
            }

            let (archive, compression_options) = {
                use ba2::tes4::{
                    Archive as TesArchive, ArchiveOptions, FileCompressionOptions as FcOpts,
                };
                use ba2::Reader as BsaReader;
                let (archive, options): (TesArchive, ArchiveOptions) =
                    match TesArchive::read(bsa_path.as_path()) {
                        Ok(a) => a,
                        Err(e) => {
                            warn!("Failed to open {}: {}", bsa_name, e);
                            let missing_count = assets_for_bsa.len();
                            failed.fetch_add(missing_count, Ordering::Relaxed);
                            pb.inc(missing_count as u64);
                            continue;
                        }
                    };
                (archive, FcOpts::from(&options))
            };

            let matched = {
                let mut matched: Vec<(Vec<&Asset>, &ba2::tes4::File<'_>)> = Vec::new();
                use ba2::ByteSlice;
                for (dir_key, folder) in archive.iter() {
                    if path_to_assets.is_empty() {
                        break;
                    }
                    let dir_name =
                        String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();
                    for (file_key, file) in folder.iter() {
                        let file_name =
                            String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
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
                matched
            };

            // Decompress + process in parallel, track readiness
            let success_ref = &success;
            let failed_ref = &failed;
            let errors_ref = &errors;
            let pb_ref = &pb;
            let timings_ref = &op_timings;
            let staged_ref = &staged_counts;
            let expected_ref = &expected;
            let ready_tx = &bsa_ready_tx;

            matched.par_iter().for_each(|(assets_needing, file)| {
                let decompress_start = Instant::now();
                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&compression_options) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(e) => {
                            for asset in assets_needing {
                                failed_ref.fetch_add(1, Ordering::Relaxed);
                                let error_msg = format!(
                                    "{} (op={}): decompression failed: {}",
                                    asset.source_path, asset.op_type, e
                                );
                                warn!("{}", error_msg);
                                if let Ok(mut errs) = errors_ref.lock() {
                                    if errs.len() < 100 {
                                        errs.push(error_msg);
                                    }
                                }
                                pb_ref.inc(1);
                            }
                            return;
                        }
                    }
                };
                timings_ref.record_decompress(decompress_start.elapsed().as_nanos() as u64);

                for asset in assets_needing {
                    let op_start = Instant::now();
                    let result = self.process_asset_with_data(asset, &data);
                    if let Some(op) = OpType::from_i32(asset.op_type) {
                        timings_ref.record(op, op_start.elapsed().as_nanos() as u64);
                    }
                    match &result {
                        Ok(_) => {
                            success_ref.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            failed_ref.fetch_add(1, Ordering::Relaxed);
                            let error_msg =
                                format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                            warn!("{}", error_msg);
                            if let Ok(mut errs) = errors_ref.lock() {
                                if errs.len() < 100 {
                                    errs.push(error_msg);
                                }
                            }
                        }
                    }
                    pb_ref.inc(1);

                    // Track readiness: if this file went to a BSA target, check if that BSA is complete
                    if result.is_ok() {
                        if let Some(counter) = staged_ref.get(&asset.target_loc) {
                            let new_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(&expected_count) = expected_ref.get(&asset.target_loc) {
                                if new_count == expected_count {
                                    // This output BSA has all files staged - send to builder
                                    let _ = ready_tx.send(asset.target_loc);
                                }
                            }
                        }
                    }
                }
            });

            // Log progress after each source BSA
            if (bsa_idx + 1) % 5 == 0 || bsa_idx == 0 {
                let peak = monitor.peak_rss_mb();
                let current = monitor.current_rss_mb();
                let built = bsa_build_success.load(Ordering::Relaxed);
                pb.set_message(format!(
                    "BSA {}/{} | RAM: {:.0} MB (peak {:.0}) | {} output BSAs built",
                    bsa_idx + 1,
                    num_source_bsas,
                    current,
                    peak,
                    built
                ));
            }
        }

        // === Step 4: Process directory assets ===
        if !dir_assets.is_empty() {
            pb.set_message(format!(
                "Processing {} directory assets...",
                dir_assets.len()
            ));
            dir_assets.par_iter().for_each(|asset| {
                let op_start = Instant::now();
                let result = self.process_asset(asset);
                if let Some(op) = OpType::from_i32(asset.op_type) {
                    op_timings.record(op, op_start.elapsed().as_nanos() as u64);
                }
                match &result {
                    Ok(_) => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg =
                            format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("{}", error_msg);
                        push_limited_error(&errors, error_msg, 100);
                    }
                }
                pb.inc(1);

                // Track readiness for directory assets too
                if result.is_ok() {
                    if let Some(counter) = staged_counts.get(&asset.target_loc) {
                        let new_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        if let Some(&expected_count) = expected.get(&asset.target_loc) {
                            if new_count == expected_count {
                                let _ = bsa_ready_tx.send(asset.target_loc);
                            }
                        }
                    }
                }
            });
        }

        // === Step 5: Signal builder thread to finish, build any remaining BSAs ===
        drop(bsa_ready_tx);

        // Wait for background builds to complete
        bsa_builder_handle
            .join()
            .map_err(|_| anyhow::anyhow!("BSA builder thread panicked"))?;

        // Build any BSAs that weren't triggered by readiness (edge case: errors reduced count)
        {
            let mut writer = self
                .bsa_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
            let remaining_keys: Vec<i32> = writer
                .bsa_location_indices()
                .into_iter()
                .filter(|idx| writer.file_count(*idx).unwrap_or(0) > 0)
                .collect();

            if !remaining_keys.is_empty() {
                println!("\n  Building {} remaining BSAs...", remaining_keys.len());
                for loc_idx in remaining_keys {
                    if let Some((bsa_name, builder)) = writer.take_builder(loc_idx) {
                        let file_count = builder.file_count();
                        let output_path = self.dest_dir.join(&bsa_name);
                        drop(writer); // Release during build

                        println!("  [BSA] {} ({} files)...", bsa_name, file_count);
                        match builder.build(&output_path) {
                            Ok(_) => {
                                let size_mb = fs::metadata(&output_path)
                                    .map(|m| m.len() / 1024 / 1024)
                                    .unwrap_or(0);
                                println!("  [BSA] {} ... OK ({} MB)", bsa_name, size_mb);
                                bsa_build_success.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                println!("  [BSA] {} ... FAILED: {}", bsa_name, e);
                                bsa_build_fail.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        writer = self
                            .bsa_writer
                            .lock()
                            .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
                    }
                }
            }
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);
        let bsa_ok = bsa_build_success.load(Ordering::Relaxed);
        let bsa_err = bsa_build_fail.load(Ordering::Relaxed);

        pb.finish_with_message(format!(
            "Done: {} success, {} failed | {} BSAs built",
            final_success, final_failed, bsa_ok
        ));

        let memory_report = monitor.stop();
        print_ram_status("End");

        println!(
            "\n  BSA archives: {}/{} built ({} failed)",
            bsa_ok,
            bsa_ok + bsa_err,
            bsa_err
        );

        let final_errors = take_errors(errors);

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
            bsa_success: bsa_ok,
            bsa_failed: bsa_err,
            errors: final_errors,
            memory: Some(memory_report),
            timings: Some(op_timings.snapshot()),
        })
    }

    /// Process assets with sequential BSA pipeline and progress callback for GUI.
    /// Same algorithm as `process_assets_streaming` but with GUI progress callbacks.
    pub fn process_assets_streaming_with_callback<F>(
        &self,
        assets: &[Asset],
        callback: F,
    ) -> Result<ProcessingStats>
    where
        F: Fn(usize, usize, &str) + Send + Sync,
    {
        use crossbeam_channel::bounded;

        let callback = Arc::new(callback);
        info!("Using overlapped BSA pipeline with progress callback");

        // === Step 1: Group assets by source BSA and build readiness map ===
        let mut bsa_assets: HashMap<PathBuf, Vec<&Asset>> = HashMap::new();
        let mut dir_assets: Vec<&Asset> = Vec::new();
        let mut expected_counts: HashMap<i32, AtomicUsize> = HashMap::new();

        for asset in assets {
            if self.resolver.is_bsa_location(asset.source_loc) {
                if let Ok(bsa_path) = self.resolver.get_bsa_path(asset.source_loc) {
                    bsa_assets.entry(bsa_path).or_default().push(asset);
                }
            } else {
                dir_assets.push(asset);
            }

            let writer = self
                .bsa_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
            if writer.is_bsa_location(asset.target_loc) {
                expected_counts
                    .entry(asset.target_loc)
                    .or_insert_with(|| AtomicUsize::new(0))
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        let expected: HashMap<i32, usize> = expected_counts
            .iter()
            .map(|(k, v)| (*k, v.load(Ordering::Relaxed)))
            .collect();
        let staged_counts: HashMap<i32, AtomicUsize> =
            expected.keys().map(|k| (*k, AtomicUsize::new(0))).collect();
        let staged_counts = Arc::new(staged_counts);

        let mut bsa_entries: Vec<_> = bsa_assets.into_iter().collect();
        bsa_entries.sort_by(|a, b| {
            let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
            let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
            size_b.cmp(&size_a)
        });

        let total = assets.len();
        let num_bsas = bsa_entries.len();
        let processed = Arc::new(AtomicUsize::new(0));

        callback(0, total, &format!("Processing {} BSAs...", num_bsas));

        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let bsa_build_success = Arc::new(AtomicUsize::new(0));
        let bsa_build_fail = Arc::new(AtomicUsize::new(0));

        let (bsa_ready_tx, bsa_ready_rx) = bounded::<i32>(32);
        let bsa_writer_clone = self.bsa_writer.clone();
        let dest_dir = self.dest_dir.clone();
        let bsa_build_success_ref = bsa_build_success.clone();
        let bsa_build_fail_ref = bsa_build_fail.clone();

        let bsa_builder_handle = std::thread::Builder::new()
            .name("bsa-builder".into())
            .spawn(move || {
                for loc_idx in bsa_ready_rx {
                    let mut writer = match bsa_writer_clone.lock() {
                        Ok(writer) => writer,
                        Err(_) => {
                            warn!("BSA writer lock poisoned while building ready archive");
                            bsa_build_fail_ref.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };

                    if let Some((bsa_name, builder)) = writer.take_builder(loc_idx) {
                        let file_count = builder.file_count();
                        let output_path = dest_dir.join(&bsa_name);
                        drop(writer);

                        tracing::info!("Building ready BSA: {} ({} files)", bsa_name, file_count);

                        match builder.build(&output_path) {
                            Ok(_) => {
                                bsa_build_success_ref.fetch_add(1, Ordering::Relaxed);
                                tracing::info!("Built ready BSA: {}", bsa_name);
                            }
                            Err(e) => {
                                warn!("BSA build failed: {} - {}", bsa_name, e);
                                bsa_build_fail_ref.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            })
            .context("Failed to spawn BSA builder thread")?;

        // === Step 2: Process BSAs sequentially, decompress+process in parallel ===
        for (bsa_idx, (bsa_path, assets_for_bsa)) in bsa_entries.iter().enumerate() {
            let bsa_name = bsa_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            callback(
                processed.load(Ordering::Relaxed),
                total,
                &format!(
                    "BSA {}/{}: {} ({} files)",
                    bsa_idx + 1,
                    num_bsas,
                    bsa_name,
                    assets_for_bsa.len()
                ),
            );

            let mut path_to_assets: HashMap<String, Vec<&Asset>> = HashMap::new();
            for asset in assets_for_bsa {
                let normalized = asset.source_path.replace('/', "\\").to_lowercase();
                path_to_assets.entry(normalized).or_default().push(asset);
            }

            let (archive, compression_options) = {
                use ba2::tes4::{
                    Archive as TesArchive, ArchiveOptions, FileCompressionOptions as FcOpts,
                };
                use ba2::Reader as BsaReader;
                let (archive, options): (TesArchive, ArchiveOptions) =
                    match TesArchive::read(bsa_path.as_path()) {
                        Ok(a) => a,
                        Err(e) => {
                            warn!("Failed to open {}: {}", bsa_name, e);
                            let missing_count = assets_for_bsa.len();
                            failed.fetch_add(missing_count, Ordering::Relaxed);
                            processed.fetch_add(missing_count, Ordering::Relaxed);
                            continue;
                        }
                    };
                (archive, FcOpts::from(&options))
            };

            let matched = {
                let mut matched: Vec<(Vec<&Asset>, &ba2::tes4::File<'_>)> = Vec::new();
                use ba2::ByteSlice;
                for (dir_key, folder) in archive.iter() {
                    if path_to_assets.is_empty() {
                        break;
                    }
                    let dir_name =
                        String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();
                    for (file_key, file) in folder.iter() {
                        let file_name =
                            String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
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
                matched
            };

            let success_ref = &success;
            let failed_ref = &failed;
            let errors_ref = &errors;
            let processed_ref = &processed;
            let callback_ref = &callback;
            let staged_ref = &staged_counts;
            let expected_ref = &expected;
            let ready_tx = &bsa_ready_tx;

            matched.par_iter().for_each(|(assets_needing, file)| {
                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&compression_options) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(e) => {
                            for asset in assets_needing {
                                failed_ref.fetch_add(1, Ordering::Relaxed);
                                let error_msg = format!(
                                    "{} (op={}): decompression failed: {}",
                                    asset.source_path, asset.op_type, e
                                );
                                warn!("{}", error_msg);
                                if let Ok(mut errs) = errors_ref.lock() {
                                    if errs.len() < 100 {
                                        errs.push(error_msg);
                                    }
                                }
                                processed_ref.fetch_add(1, Ordering::Relaxed);
                            }
                            return;
                        }
                    }
                };

                for asset in assets_needing {
                    let result = self.process_asset_with_data(asset, &data);
                    match &result {
                        Ok(_) => {
                            success_ref.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            failed_ref.fetch_add(1, Ordering::Relaxed);
                            let error_msg =
                                format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                            warn!("{}", error_msg);
                            if let Ok(mut errs) = errors_ref.lock() {
                                if errs.len() < 100 {
                                    errs.push(error_msg);
                                }
                            }
                        }
                    }
                    let current = processed_ref.fetch_add(1, Ordering::Relaxed) + 1;
                    if current.is_multiple_of(100) || current <= 5 {
                        callback_ref(current, total, &format!("BSA {}/{}", bsa_idx + 1, num_bsas));
                    }
                    if result.is_ok() {
                        if let Some(counter) = staged_ref.get(&asset.target_loc) {
                            let new_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(&expected_count) = expected_ref.get(&asset.target_loc) {
                                if new_count == expected_count {
                                    let _ = ready_tx.send(asset.target_loc);
                                }
                            }
                        }
                    }
                }
            });
        }

        // === Step 3: Process directory assets ===
        if !dir_assets.is_empty() {
            callback(
                processed.load(Ordering::Relaxed),
                total,
                "Processing directory assets...",
            );

            dir_assets.par_iter().for_each(|asset| {
                let result = self.process_asset(asset);
                match &result {
                    Ok(_) => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let error_msg =
                            format!("{} (op={}): {}", asset.source_path, asset.op_type, e);
                        warn!("{}", error_msg);
                        push_limited_error(&errors, error_msg, 100);
                    }
                }

                let current = processed.fetch_add(1, Ordering::Relaxed) + 1;
                if current.is_multiple_of(100) || current == total {
                    callback(current, total, "Processing loose files");
                }
                if result.is_ok() {
                    if let Some(counter) = staged_counts.get(&asset.target_loc) {
                        let new_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        if let Some(&expected_count) = expected.get(&asset.target_loc) {
                            if new_count == expected_count {
                                let _ = bsa_ready_tx.send(asset.target_loc);
                            }
                        }
                    }
                }
            });
        }

        drop(bsa_ready_tx);
        bsa_builder_handle
            .join()
            .map_err(|_| anyhow::anyhow!("BSA builder thread panicked"))?;

        {
            let mut writer = self
                .bsa_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
            let remaining_keys: Vec<i32> = writer
                .bsa_location_indices()
                .into_iter()
                .filter(|idx| writer.file_count(*idx).unwrap_or(0) > 0)
                .collect();

            for loc_idx in remaining_keys {
                if let Some((bsa_name, builder)) = writer.take_builder(loc_idx) {
                    let file_count = builder.file_count();
                    let output_path = self.dest_dir.join(&bsa_name);
                    drop(writer);

                    callback(
                        processed.load(Ordering::Relaxed),
                        total,
                        &format!(
                            "Building remaining BSA: {} ({} files)",
                            bsa_name, file_count
                        ),
                    );
                    match builder.build(&output_path) {
                        Ok(_) => {
                            bsa_build_success.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            warn!("BSA build failed: {} - {}", bsa_name, e);
                            bsa_build_fail.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    writer = self
                        .bsa_writer
                        .lock()
                        .map_err(|_| anyhow::anyhow!("BSA writer lock poisoned"))?;
                }
            }
        }

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);
        let bsa_ok = bsa_build_success.load(Ordering::Relaxed);
        let bsa_err = bsa_build_fail.load(Ordering::Relaxed);

        if final_failed > 0 {
            warn!(
                "Processing complete: {} succeeded, {} failed",
                final_success, final_failed
            );
        }

        let final_errors = take_errors(errors);

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            bsa_success: bsa_ok,
            bsa_failed: bsa_err,
            errors: final_errors,
            memory: None,
            timings: None,
        })
    }
}
