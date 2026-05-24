use anyhow::{bail, Context, Result};
use ba2::tes4::Archive;
use ba2::{ByteSlice, Reader};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use sysinfo::System;
use tracing::info;

/// Handles reading from and writing to BSA archives
///
/// Note: Archives are opened fresh for each extraction to avoid lifetime issues.
/// This is slightly slower but ensures memory safety.
pub struct BsaHandler {
    /// Track which BSA paths have been accessed (for diagnostics)
    accessed_bsas: HashMap<String, usize>,
}

impl BsaHandler {
    pub fn new() -> Self {
        Self {
            accessed_bsas: HashMap::new(),
        }
    }

    /// Extract a file from a BSA archive
    pub fn extract_file(&mut self, bsa_path: &Path, file_path: &str) -> Result<Vec<u8>> {
        let bsa_key = bsa_path.to_string_lossy().to_string();

        // Track access count for diagnostics
        *self.accessed_bsas.entry(bsa_key).or_insert(0) += 1;

        // Open archive fresh each time (avoids lifetime issues with caching)
        let (archive, options) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to open BSA: {}", bsa_path.display()))?;
        let compression_options: ba2::tes4::FileCompressionOptions = (&options).into();

        // Normalize path separators (BSA uses backslashes)
        let normalized_path = file_path.replace('/', "\\");

        // Parse directory and file name
        let (dir_name, file_name) = if let Some(idx) = normalized_path.rfind('\\') {
            (&normalized_path[..idx], &normalized_path[idx + 1..])
        } else {
            ("", normalized_path.as_str())
        };

        // Search for the file
        for (dir_key, folder) in archive.iter() {
            let current_dir = String::from_utf8_lossy(dir_key.name().as_bytes());

            if current_dir.eq_ignore_ascii_case(dir_name) {
                for (file_key, file) in folder.iter() {
                    let current_file = String::from_utf8_lossy(file_key.name().as_bytes());

                    if current_file.eq_ignore_ascii_case(file_name) {
                        let data = if file.is_decompressed() {
                            file.as_bytes().to_vec()
                        } else {
                            file.decompress(&compression_options)?.as_bytes().to_vec()
                        };
                        return Ok(data);
                    }
                }
            }
        }

        bail!(
            "File not found in BSA: {} (searched for dir='{}', file='{}')",
            file_path,
            dir_name,
            file_name
        );
    }

    /// Check if a file exists in a BSA archive
    pub fn file_exists(&mut self, bsa_path: &Path, file_path: &str) -> Result<bool> {
        match self.extract_file(bsa_path, file_path) {
            Ok(_) => Ok(true),
            Err(e) if e.to_string().contains("not found") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Clear tracking data (no-op now that caching is removed, kept for API compatibility)
    pub fn clear_cache(&mut self) {
        self.accessed_bsas.clear();
    }

    /// Get access statistics for diagnostics
    pub fn access_stats(&self) -> &HashMap<String, usize> {
        &self.accessed_bsas
    }
}

impl Default for BsaHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Smart BSA extraction cache with memory limits
///
/// Pre-extracts files from BSAs in batches to avoid repeated BSA parsing.
/// Automatically limits memory usage based on system RAM.
pub struct BsaExtractCache {
    /// Cached file data: (bsa_path, file_path) -> data
    cache: HashMap<(PathBuf, String), Vec<u8>>,
    /// Current cache size in bytes
    current_size: AtomicUsize,
    /// Maximum cache size in bytes
    max_size: usize,
    /// Total system RAM in bytes (for reference)
    system_ram: u64,
}

impl BsaExtractCache {
    /// Create a new cache with smart memory limits
    /// Uses minimum of: 10GB hard cap, or 80% of system RAM
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_memory();
        let system_ram = sys.total_memory(); // in bytes

        const HARD_CAP: usize = 10 * 1024 * 1024 * 1024; // 10GB
        let soft_cap = ((system_ram as f64) * 0.8) as usize;
        let max_size = std::cmp::min(HARD_CAP, soft_cap);

        info!(
            "BSA cache initialized: system RAM = {:.1}GB, cache limit = {:.1}GB",
            system_ram as f64 / 1024.0 / 1024.0 / 1024.0,
            max_size as f64 / 1024.0 / 1024.0 / 1024.0
        );

        Self {
            cache: HashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size,
            system_ram,
        }
    }

    /// Create cache with a specific size limit (for testing)
    pub fn with_limit(max_bytes: usize) -> Self {
        let mut sys = System::new_all();
        sys.refresh_memory();

        Self {
            cache: HashMap::new(),
            current_size: AtomicUsize::new(0),
            max_size: max_bytes,
            system_ram: sys.total_memory(),
        }
    }

    /// Get current cache size in bytes
    pub fn current_size(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Get max cache size in bytes
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Check if adding data would exceed the limit
    pub fn would_exceed(&self, additional_bytes: usize) -> bool {
        self.current_size() + additional_bytes > self.max_size
    }

    /// Check current system memory pressure
    /// Returns true if we should flush cache (available RAM < 20% of total)
    pub fn memory_pressure(&self) -> bool {
        let mut sys = System::new();
        sys.refresh_memory();
        let available = sys.available_memory();
        let threshold = (self.system_ram as f64 * 0.2) as u64;
        available < threshold
    }

    /// Pre-extract multiple files from a single BSA
    /// Returns number of files extracted, or error
    pub fn extract_batch(&mut self, bsa_path: &Path, file_paths: &[&str]) -> Result<usize> {
        // Open BSA once with proper options for decompression
        let (archive, options) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to open BSA: {}", bsa_path.display()))?;
        let compression_options: ba2::tes4::FileCompressionOptions = (&options).into();

        // Build a set of normalized paths we need
        let mut needed: HashSet<String> = file_paths
            .iter()
            .map(|p| p.replace('/', "\\").to_lowercase())
            .collect();

        let mut extracted = 0;
        let bsa_pathbuf = bsa_path.to_path_buf();

        // Iterate through archive and extract matching files
        for (dir_key, folder) in archive.iter() {
            if needed.is_empty() {
                break;
            }

            let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();

            for (file_key, file) in folder.iter() {
                let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
                let full_path = if dir_name.is_empty() || dir_name == "." {
                    file_name.clone()
                } else {
                    format!("{}\\{}", dir_name, file_name)
                };

                if needed.remove(&full_path) {
                    let data = if file.is_decompressed() {
                        file.as_bytes().to_vec()
                    } else {
                        file.decompress(&compression_options)?.as_bytes().to_vec()
                    };

                    let data_size = data.len();

                    // Check memory limit
                    if self.would_exceed(data_size) {
                        // Don't extract more if we'd exceed limit
                        info!("BSA cache limit reached, stopping batch extraction");
                        return Ok(extracted);
                    }

                    // Also check system memory pressure
                    if extracted > 0 && extracted % 1000 == 0 && self.memory_pressure() {
                        info!("System memory pressure detected, stopping batch extraction");
                        return Ok(extracted);
                    }

                    // Add to cache
                    // Use original case for the key (find it in file_paths)
                    let original_path = file_paths
                        .iter()
                        .find(|p| p.replace('/', "\\").to_lowercase() == full_path)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| full_path.clone());

                    self.cache
                        .insert((bsa_pathbuf.clone(), original_path), data);
                    self.current_size.fetch_add(data_size, Ordering::Relaxed);
                    extracted += 1;
                }
            }
        }

        Ok(extracted)
    }

    /// Get a file from cache
    pub fn get(&self, bsa_path: &Path, file_path: &str) -> Option<&Vec<u8>> {
        self.cache
            .get(&(bsa_path.to_path_buf(), file_path.to_string()))
    }

    /// Take a file from cache (removes it)
    pub fn take(&mut self, bsa_path: &Path, file_path: &str) -> Option<Vec<u8>> {
        if let Some(data) = self
            .cache
            .remove(&(bsa_path.to_path_buf(), file_path.to_string()))
        {
            self.current_size.fetch_sub(data.len(), Ordering::Relaxed);
            Some(data)
        } else {
            None
        }
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_size.store(0, Ordering::Relaxed);
    }

    /// Get number of cached files
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for BsaExtractCache {
    fn default() -> Self {
        Self::new()
    }
}
