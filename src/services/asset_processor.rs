use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::models::{Asset, Location};
use crate::services::{
    LocationResolver, BsaHandler, BsaWriterManager, AudioProcessor, AudioFormat, XdeltaManager,
};

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
    ) -> Self {
        let mut bsa_writer = BsaWriterManager::new();

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
                    );
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
                    bsa_writer.register_bsa(i as i32, name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed);
                }
                // Format 2: Type 2 with .bsa in VALUE (original MPI format)
                else if loc.loc_type == 2 && is_bsa_value {
                    let normalized = value.replace('\\', "/");
                    let bsa_name = normalized.rsplit('/').next().unwrap_or(&normalized);
                    bsa_writer.register_bsa(i as i32, bsa_name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed);
                }
                // Format 3: Type 1 with "NEW " prefix
                else if loc.loc_type == 1 && has_new_prefix && is_bsa_value {
                    bsa_writer.register_bsa(i as i32, name, loc.archive_type, loc.archive_flags, loc.files_flags, loc.archive_compressed);
                }
            }
        }

        Self {
            resolver: Arc::new(resolver),
            bsa_handler: Arc::new(Mutex::new(BsaHandler::new())),
            bsa_writer: Arc::new(Mutex::new(bsa_writer)),
            xdelta: Arc::new(xdelta),
            mpi_dir,
            dest_dir,
            dry_run: false,
        }
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Process a list of assets in parallel
    pub fn process_assets(&self, assets: &[Asset]) -> Result<ProcessingStats> {
        // Group assets by operation type for progress display
        let mut by_type: HashMap<i32, Vec<&Asset>> = HashMap::new();
        for asset in assets {
            by_type.entry(asset.op_type).or_default().push(asset);
        }

        println!("\nProcessing {} total assets (parallel):", assets.len());
        for (op_type, group) in &by_type {
            let name = OpType::from_i32(*op_type)
                .map(|t| t.name())
                .unwrap_or("Unknown");
            println!("  {} ({}): {}", name, op_type, group.len());
        }

        // Thread-safe counters
        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let pb = ProgressBar::new(assets.len() as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("#>-"));

        // Process assets in parallel
        assets.par_iter().for_each(|asset| {
            let result = self.process_asset(asset);

            match result {
                Ok(_) => {
                    success.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    let mut errs = errors.lock().unwrap();
                    if errs.len() < 10 {
                        errs.push(format!(
                            "{} ({}): {}",
                            asset.source_path, asset.op_type, e
                        ));
                    }
                }
            }

            pb.inc(1);
            let s = success.load(Ordering::Relaxed);
            let f = failed.load(Ordering::Relaxed);
            pb.set_message(format!("OK:{} Fail:{}", s, f));
        });

        let final_success = success.load(Ordering::Relaxed);
        let final_failed = failed.load(Ordering::Relaxed);

        pb.finish_with_message(format!(
            "Done: {} success, {} failed",
            final_success, final_failed
        ));

        let final_errors = Arc::try_unwrap(errors)
            .unwrap_or_else(|e| e.lock().unwrap().clone().into())
            .into_inner()
            .unwrap();

        Ok(ProcessingStats {
            success: final_success,
            failed: final_failed,
            errors: final_errors,
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

        // Thread-safe counters
        let success = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let processed = AtomicUsize::new(0);
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // Process assets in parallel
        assets.par_iter().for_each(|asset| {
            let result = self.process_asset(asset);

            match result {
                Ok(_) => {
                    success.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    let mut errs = errors.lock().unwrap();
                    if errs.len() < 50 {
                        errs.push(format!(
                            "{} ({}): {}",
                            asset.source_path, asset.op_type, e
                        ));
                    }
                }
            }

            let current = processed.fetch_add(1, Ordering::Relaxed) + 1;
            let s = success.load(Ordering::Relaxed);
            let f = failed.load(Ordering::Relaxed);

            // Call progress callback frequently enough for responsive UI
            // - Every 50 assets for large packages
            // - Every 10 assets for medium packages (100-1000)
            // - Every asset for small packages (<100)
            let callback_interval = if total > 1000 { 50 } else if total > 100 { 10 } else { 1 };
            if current % callback_interval == 0 || current == total || current <= 5 {
                callback(current, total, &format!("Processing: {} OK, {} failed", s, f));
            }
        });

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

        // Create audio processor per call (stateless)
        let audio_processor = AudioProcessor::new();
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

        // Create audio processor per call (stateless)
        let audio_processor = AudioProcessor::new();
        let processed = audio_processor.process_audio_conversion(
            &source_data,
            input_format,
            output_format,
        )?;
        self.write_to_target(asset, &processed)?;
        Ok(())
    }

    /// Get source data, either from BSA or directory
    fn get_source_data(&self, asset: &Asset) -> Result<Vec<u8>> {
        if self.resolver.is_bsa_location(asset.source_loc) {
            // Extract from BSA (thread-safe with mutex)
            let bsa_path = self.resolver.get_bsa_path(asset.source_loc)?;
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
            let mut writer = self.bsa_writer.lock().unwrap();
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

        let writer = self.bsa_writer.lock().unwrap();
        writer.write_all(&self.dest_dir)
    }

    /// Finalize BSAs with progress callback for GUI
    /// callback(current, total, bsa_name)
    pub fn finalize_bsas_with_callback<F>(&self, callback: F) -> Result<(usize, usize)>
    where
        F: Fn(usize, usize, &str),
    {
        if self.dry_run {
            return Ok((0, 0));
        }

        let writer = self.bsa_writer.lock().unwrap();
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
}

/// Statistics from processing
#[derive(Debug, Default)]
pub struct ProcessingStats {
    pub success: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

impl ProcessingStats {
    pub fn print_summary(&self) {
        println!("\nProcessing Summary:");
        println!("  Successful: {}", self.success);
        println!("  Failed: {}", self.failed);

        if !self.errors.is_empty() {
            println!("\nErrors:");
            for error in &self.errors {
                println!("  - {}", error);
            }
        }
    }
}
