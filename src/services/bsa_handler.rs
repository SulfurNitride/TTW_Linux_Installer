use anyhow::{Result, Context, bail};
use ba2::tes4::{Archive, ArchiveKey, ArchiveOptions, ArchiveFlags, ArchiveTypes, Directory, DirectoryKey, File as BsaFile, Version};
use ba2::{ByteSlice, CompressableFrom, Reader};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{BufWriter, BufReader, Write, Read as IoRead, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::sync::Mutex;
use sysinfo::System;
use tracing::info;

/// Strip common MPI prefixes from BSA names
/// These prefixes are internal identifiers, not part of actual output filenames
fn strip_bsa_prefix(name: &str) -> &str {
    // Common prefixes used in MPI manifests to identify BSA sources/targets
    // Check case-insensitively
    let name_lower = name.to_lowercase();

    // NOTE: "new " is NOT stripped - it's part of the actual filename (e.g., "New Fallout - Textures2.bsa")
    // Only strip prefixes that are purely internal identifiers
    const PREFIXES: &[&str] = &[
        "ttw ",   // Tale of Two Wastelands internal identifier
        "fo3 ",   // Fallout 3 source identifier
        "fnv ",   // Fallout New Vegas source identifier
        "tes4 ", // Oblivion source identifier
    ];

    for prefix in PREFIXES {
        if name_lower.starts_with(prefix) {
            return &name[prefix.len()..];
        }
    }
    name
}

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
        let (archive, _) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to open BSA: {}", bsa_path.display()))?;

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
                        // Extract file data
                        let data = if file.is_compressed() {
                            file.decompress(&Default::default())?.as_bytes().to_vec()
                        } else {
                            file.as_bytes().to_vec()
                        };
                        return Ok(data);
                    }
                }
            }
        }

        bail!("File not found in BSA: {} (searched for dir='{}', file='{}')",
            file_path, dir_name, file_name);
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
    pub fn extract_batch(
        &mut self,
        bsa_path: &Path,
        file_paths: &[&str],
    ) -> Result<usize> {
        // Open BSA once
        let (archive, _) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to open BSA: {}", bsa_path.display()))?;

        // Build a set of normalized paths we need
        let mut needed: HashSet<String> = file_paths.iter()
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
                    // Extract file data
                    let data = if file.is_compressed() {
                        file.decompress(&Default::default())?.as_bytes().to_vec()
                    } else {
                        file.as_bytes().to_vec()
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
                    let original_path = file_paths.iter()
                        .find(|p| p.replace('/', "\\").to_lowercase() == full_path)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| full_path.clone());

                    self.cache.insert(
                        (bsa_pathbuf.clone(), original_path),
                        data,
                    );
                    self.current_size.fetch_add(data_size, Ordering::Relaxed);
                    extracted += 1;
                }
            }
        }

        Ok(extracted)
    }

    /// Get a file from cache
    pub fn get(&self, bsa_path: &Path, file_path: &str) -> Option<&Vec<u8>> {
        self.cache.get(&(bsa_path.to_path_buf(), file_path.to_string()))
    }

    /// Take a file from cache (removes it)
    pub fn take(&mut self, bsa_path: &Path, file_path: &str) -> Option<Vec<u8>> {
        if let Some(data) = self.cache.remove(&(bsa_path.to_path_buf(), file_path.to_string())) {
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

/// Builder for creating BSA archives using ba2
pub struct BsaBuilder {
    /// Files organized by directory path -> file name -> data
    files: HashMap<String, HashMap<String, Vec<u8>>>,
    archive_flags: ArchiveFlags,
    archive_types: ArchiveTypes,
    version: Version,
}

impl BsaBuilder {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            archive_flags: ArchiveFlags::DIRECTORY_STRINGS
                | ArchiveFlags::FILE_STRINGS
                | ArchiveFlags::COMPRESSED
                | ArchiveFlags::RETAIN_DIRECTORY_NAMES
                | ArchiveFlags::RETAIN_FILE_NAMES
                | ArchiveFlags::RETAIN_FILE_NAME_OFFSETS,
            archive_types: ArchiveTypes::empty(),
            version: Version::v104, // FNV version
        }
    }

    /// Set archive flags
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.archive_flags = ArchiveFlags::from_bits_truncate(flags);
        self
    }

    /// Set archive types (file types contained in the archive)
    pub fn with_types(mut self, types: u16) -> Self {
        self.archive_types = ArchiveTypes::from_bits_truncate(types);
        self
    }

    /// Set the BSA version
    pub fn with_version(mut self, version: u32) -> Self {
        self.version = match version {
            103 => Version::v103,
            105 => Version::v105,
            _ => Version::v104, // Default to FNV/FO3 version
        };
        self
    }

    /// Derive archive types, version, and flags from BSA name
    pub fn with_types_from_name(mut self, bsa_name: &str) -> Self {
        let name_lower = bsa_name.to_lowercase();

        // Detect game type from BSA name to set correct version and flags
        // Oblivion uses v103, Fallout 3/NV use v104
        if name_lower.contains("oblivion") || name_lower.contains("shiveringisles") {
            self.version = Version::v103;  // Oblivion
            // Oblivion BSA flags - NO compression for decompressed output
            // Use minimal flags: directory strings + file strings only
            self.archive_flags = ArchiveFlags::DIRECTORY_STRINGS | ArchiveFlags::FILE_STRINGS;
        } else {
            self.version = Version::v104;  // Fallout 3/NV (default)
            // Keep default FO3/FNV flags with compression
        }

        self.archive_types = if name_lower.contains("meshes") {
            ArchiveTypes::MESHES
        } else if name_lower.contains("textures") {
            ArchiveTypes::TEXTURES
        } else if name_lower.contains("menuvoices") {
            ArchiveTypes::MENUS | ArchiveTypes::VOICES
        } else if name_lower.contains("voices") {
            ArchiveTypes::VOICES
        } else if name_lower.contains("sound") {
            ArchiveTypes::SOUNDS
        } else {
            // Default to MISC for main, misc, or unrecognized types
            ArchiveTypes::MISC
        };

        self
    }

    /// Add a file to the archive
    pub fn add_file(&mut self, file_path: &str, data: Vec<u8>) {
        // Normalize path separators and split into dir/file
        let normalized = file_path.replace('\\', "/");
        let normalized = normalized.trim_start_matches('/');

        let (dir_path, file_name) = if let Some(idx) = normalized.rfind('/') {
            (normalized[..idx].to_string(), normalized[idx + 1..].to_string())
        } else {
            (".".to_string(), normalized.to_string())
        };

        self.files
            .entry(dir_path)
            .or_default()
            .insert(file_name, data);
    }

    /// Get the number of files added
    pub fn file_count(&self) -> usize {
        self.files.values().map(|dir| dir.len()).sum()
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.file_count() == 0
    }

    /// Build and write the BSA archive to disk
    pub fn build(self, output_path: &Path) -> Result<()> {
        if self.is_empty() {
            bail!("Cannot create empty BSA archive");
        }

        // Build the archive structure
        let archive: Archive = self.files.iter().map(|(dir_path, files)| {
            let directory: Directory = files.iter().map(|(file_name, data)| {
                let file = BsaFile::from_decompressed(&data[..]);
                (DirectoryKey::from(file_name.as_bytes()), file)
            }).collect();
            (ArchiveKey::from(dir_path.as_bytes()), directory)
        }).collect();

        // Set up write options
        let options = ArchiveOptions::builder()
            .version(self.version)
            .flags(self.archive_flags)
            .types(self.archive_types)
            .build();

        // Create parent directory if needed
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the archive
        let file = fs::File::create(output_path)
            .with_context(|| format!("Failed to create BSA file: {}", output_path.display()))?;
        let mut writer = BufWriter::new(file);

        archive.write(&mut writer, &options)
            .with_context(|| format!("Failed to write BSA: {}", output_path.display()))?;

        Ok(())
    }
}

impl Default for BsaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Entry metadata for a file in the staging area
#[derive(Clone)]
struct StagingEntry {
    offset: u64,
    size: usize,
}

/// Thread-safe streaming BSA builder that writes to disk instead of RAM
///
/// Instead of accumulating all file data in a HashMap (which uses 10-15GB+ RAM),
/// this writes files to a temporary staging file and only keeps lightweight
/// metadata (~100 bytes per file) in memory.
///
/// Memory usage during add_file: ~100 bytes per file (metadata only)
/// Memory usage during build: One BSA's worth of data (not ALL BSAs combined)
///
/// For 50,000 files averaging 200KB across 10 BSAs:
/// - Old approach: ALL files in RAM = ~10GB peak
/// - New approach: ONE BSA at a time = ~1GB peak (10x reduction)
pub struct StreamingBsaBuilder {
    /// Path to staging file
    staging_path: PathBuf,
    /// Staging file writer (mutex for thread-safe writes)
    staging_writer: Mutex<Option<BufWriter<File>>>,
    /// Current write offset in staging file
    current_offset: AtomicU64,
    /// File entries: dir_path -> file_name -> (offset, size)
    entries: Mutex<HashMap<String, HashMap<String, StagingEntry>>>,
    /// Total number of files added
    file_count: AtomicUsize,
    /// Archive settings
    archive_flags: ArchiveFlags,
    archive_types: ArchiveTypes,
    version: Version,
}

impl StreamingBsaBuilder {
    /// Create a new streaming BSA builder with a temporary staging file
    pub fn new() -> Result<Self> {
        Self::with_settings(
            ArchiveFlags::DIRECTORY_STRINGS
                | ArchiveFlags::FILE_STRINGS
                | ArchiveFlags::COMPRESSED
                | ArchiveFlags::RETAIN_DIRECTORY_NAMES
                | ArchiveFlags::RETAIN_FILE_NAMES
                | ArchiveFlags::RETAIN_FILE_NAME_OFFSETS,
            ArchiveTypes::empty(),
            Version::v104,
        )
    }

    /// Create with specific archive settings
    pub fn with_settings(
        flags: ArchiveFlags,
        types: ArchiveTypes,
        version: Version,
    ) -> Result<Self> {
        // Create temp file for staging
        let staging_path = std::env::temp_dir().join(format!(
            "ttw_bsa_staging_{}.tmp",
            std::process::id()
        ));

        // Use a unique suffix to allow multiple builders
        let staging_path = staging_path.with_extension(format!(
            "{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let file = File::create(&staging_path)
            .with_context(|| format!("Failed to create staging file: {}", staging_path.display()))?;

        Ok(Self {
            staging_path,
            staging_writer: Mutex::new(Some(BufWriter::with_capacity(1024 * 1024, file))), // 1MB buffer
            current_offset: AtomicU64::new(0),
            entries: Mutex::new(HashMap::new()),
            file_count: AtomicUsize::new(0),
            archive_flags: flags,
            archive_types: types,
            version,
        })
    }

    /// Add a file to the archive (thread-safe)
    /// Data is written immediately to disk, only metadata kept in RAM
    pub fn add_file(&self, file_path: &str, data: Vec<u8>) -> Result<()> {
        let data_len = data.len();

        // Normalize path separators and split into dir/file
        let normalized = file_path.replace('\\', "/");
        let normalized = normalized.trim_start_matches('/');

        let (dir_path, file_name) = if let Some(idx) = normalized.rfind('/') {
            (normalized[..idx].to_string(), normalized[idx + 1..].to_string())
        } else {
            (".".to_string(), normalized.to_string())
        };

        // Write to staging file (under lock)
        let offset = {
            let mut writer_guard = self.staging_writer.lock().unwrap();
            let writer = writer_guard.as_mut()
                .ok_or_else(|| anyhow::anyhow!("Staging file already closed"))?;
            let offset = self.current_offset.load(Ordering::SeqCst);

            writer.write_all(&data)
                .with_context(|| "Failed to write to BSA staging file")?;

            self.current_offset.fetch_add(data_len as u64, Ordering::SeqCst);
            offset
        };

        // Store only metadata (under separate lock to minimize contention)
        {
            let mut entries = self.entries.lock().unwrap();
            entries
                .entry(dir_path)
                .or_default()
                .insert(file_name, StagingEntry {
                    offset,
                    size: data_len,
                });
        }

        self.file_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Get the number of files added
    pub fn file_count(&self) -> usize {
        self.file_count.load(Ordering::Relaxed)
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.file_count() == 0
    }

    /// Build and write the BSA archive to disk
    /// Loads ONE BSA's worth of data from staging (not all BSAs combined)
    pub fn build(self, output_path: &Path) -> Result<()> {
        if self.is_empty() {
            bail!("Cannot create empty BSA archive");
        }

        // Flush and close the staging writer
        {
            let mut writer_guard = self.staging_writer.lock().unwrap();
            if let Some(mut writer) = writer_guard.take() {
                writer.flush()?;
            }
        }

        // Get the entries (take ownership via std::mem::take)
        let entries = {
            let mut entries_guard = self.entries.lock().unwrap();
            std::mem::take(&mut *entries_guard)
        };

        // Open staging file for reading
        let staging_file = File::open(&self.staging_path)
            .with_context(|| "Failed to open staging file for reading")?;
        let mut staging_reader = BufReader::with_capacity(1024 * 1024, staging_file);

        // Load all file data from staging into an owned structure
        // This loads ONE BSA's worth of data, not all BSAs combined
        let mut owned_files: HashMap<String, HashMap<String, Vec<u8>>> = HashMap::new();

        for (dir_path, files) in entries {
            let mut dir_files = HashMap::new();
            for (file_name, entry) in files {
                let mut data = vec![0u8; entry.size];
                staging_reader.seek(SeekFrom::Start(entry.offset))?;
                staging_reader.read_exact(&mut data)?;
                dir_files.insert(file_name, data);
            }
            owned_files.insert(dir_path, dir_files);
        }

        // Close staging reader and clean up staging file
        drop(staging_reader);
        let _ = fs::remove_file(&self.staging_path);

        // Build the archive from owned data (same as original BsaBuilder)
        let archive: Archive = owned_files.iter().map(|(dir_path, files)| {
            let directory: Directory = files.iter().map(|(file_name, data)| {
                let file = BsaFile::from_decompressed(&data[..]);
                (DirectoryKey::from(file_name.as_bytes()), file)
            }).collect();
            (ArchiveKey::from(dir_path.as_bytes()), directory)
        }).collect();

        // owned_files is dropped here, freeing memory before write

        // Set up write options
        let options = ArchiveOptions::builder()
            .version(self.version)
            .flags(self.archive_flags)
            .types(self.archive_types)
            .build();

        // Create parent directory if needed
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the archive
        let file = File::create(output_path)
            .with_context(|| format!("Failed to create BSA file: {}", output_path.display()))?;
        let mut writer = BufWriter::new(file);

        archive.write(&mut writer, &options)
            .with_context(|| format!("Failed to write BSA: {}", output_path.display()))?;

        Ok(())
    }

    /// Get the staging file path (for diagnostics)
    pub fn staging_path(&self) -> &Path {
        &self.staging_path
    }
}

impl Drop for StreamingBsaBuilder {
    fn drop(&mut self) {
        // Clean up staging file if it still exists
        let _ = fs::remove_file(&self.staging_path);
    }
}

/// Manages multiple BSA archives being built during installation
/// Uses streaming builders to keep RAM usage low (~5MB vs ~10GB for 50k files)
pub struct BsaWriterManager {
    /// BSA builders keyed by location index
    /// Uses StreamingBsaBuilder for disk-backed storage instead of RAM
    builders: HashMap<i32, (String, StreamingBsaBuilder)>, // (bsa_name, builder)
}

impl BsaWriterManager {
    pub fn new() -> Self {
        Self {
            builders: HashMap::new(),
        }
    }

    /// Register a BSA target location
    /// Parameters from manifest:
    /// - archive_type: BSA version (1=v103/Oblivion, 2=v104/FO3+FNV, 3=v105/SSE)
    /// - archive_flags: Header flags
    /// - file_flags: Content type flags
    /// - archive_compressed: Whether to compress files
    pub fn register_bsa(
        &mut self,
        location_index: i32,
        bsa_name: &str,
        archive_type: Option<u16>,
        archive_flags: Option<u32>,
        file_flags: Option<u32>,
        archive_compressed: Option<bool>,
    ) -> Result<()> {
        // Strip common prefixes from BSA name for output filename
        let output_name = strip_bsa_prefix(bsa_name);
        let name_lower = output_name.to_lowercase();

        // Determine version - prioritize name-based detection as it's more reliable
        // Then fall back to archive_type if name doesn't give a clear answer
        let is_oblivion = name_lower.contains("oblivion")
            || name_lower.contains("shiveringisles")
            || name_lower.contains("dlcshiveringisles")
            || name_lower.contains("dlcbattlehorn")
            || name_lower.contains("dlcfrostcrag")
            || name_lower.contains("dlchorse")
            || name_lower.contains("dlcorrery")
            || name_lower.contains("dlcthievesden")
            || name_lower.contains("dlcvilelair")
            || name_lower.contains("knights");

        let version = if is_oblivion {
            Version::v103  // Oblivion
        } else {
            // Use archive_type for non-Oblivion, or default to v104
            match archive_type {
                Some(103) => Version::v103,  // Direct version number
                Some(104) => Version::v104,
                Some(105) => Version::v105,
                _ => Version::v104  // Default to FO3/FNV
            }
        };

        // Use manifest flags if provided, otherwise use sensible defaults
        let mut flags = if let Some(f) = archive_flags {
            ArchiveFlags::from_bits_truncate(f)
        } else {
            // Default flags based on game version
            if version == Version::v103 {
                ArchiveFlags::DIRECTORY_STRINGS | ArchiveFlags::FILE_STRINGS
            } else {
                ArchiveFlags::DIRECTORY_STRINGS
                    | ArchiveFlags::FILE_STRINGS
                    | ArchiveFlags::COMPRESSED
                    | ArchiveFlags::RETAIN_DIRECTORY_NAMES
                    | ArchiveFlags::RETAIN_FILE_NAMES
                    | ArchiveFlags::RETAIN_FILE_NAME_OFFSETS
            }
        };

        // Override compression based on manifest's archive_compressed field
        if let Some(compressed) = archive_compressed {
            if compressed {
                flags |= ArchiveFlags::COMPRESSED;
            } else {
                flags &= !ArchiveFlags::COMPRESSED;
            }
        }

        // Use manifest file types if provided, otherwise detect from name
        let types = if let Some(t) = file_flags {
            ArchiveTypes::from_bits_truncate(t as u16)
        } else if name_lower.contains("meshes") {
            ArchiveTypes::MESHES
        } else if name_lower.contains("textures") {
            ArchiveTypes::TEXTURES
        } else if name_lower.contains("menuvoices") {
            ArchiveTypes::MENUS | ArchiveTypes::VOICES
        } else if name_lower.contains("voices") {
            ArchiveTypes::VOICES
        } else if name_lower.contains("sound") {
            ArchiveTypes::SOUNDS
        } else {
            // Default to MISC for main, misc, or unrecognized types
            ArchiveTypes::MISC
        };

        // Create streaming builder (writes to temp file instead of RAM)
        let builder = StreamingBsaBuilder::with_settings(flags, types, version)
            .with_context(|| format!("Failed to create streaming BSA builder for {}", bsa_name))?;

        // Get version string for logging
        let version_str = match version {
            Version::v103 => "v103 (Oblivion)",
            Version::v104 => "v104 (FO3/FNV)",
            Version::v105 => "v105 (SSE)",
        };

        info!("Registered BSA target: Location[{}] = {} -> {} [{}, flags=0x{:x}, types=0x{:x}]",
            location_index, bsa_name, output_name, version_str, flags.bits(), types.bits());

        self.builders.insert(location_index, (output_name.to_string(), builder));
        Ok(())
    }

    /// Check if a location is a registered BSA target
    pub fn is_bsa_location(&self, location_index: i32) -> bool {
        self.builders.contains_key(&location_index)
    }

    /// Add a file to a BSA (thread-safe, writes to disk immediately)
    pub fn add_file(&self, location_index: i32, file_path: &str, data: Vec<u8>) -> Result<()> {
        let (_, builder) = self.builders.get(&location_index)
            .ok_or_else(|| anyhow::anyhow!("Location {} is not a BSA target", location_index))?;

        builder.add_file(file_path, data)
    }

    /// Get file count for a specific BSA
    pub fn file_count(&self, location_index: i32) -> Option<usize> {
        self.builders.get(&location_index).map(|(_, b)| b.file_count())
    }

    /// Write all BSA archives to the destination directory (parallel)
    /// Uses Rayon to build multiple BSAs concurrently for better CPU utilization
    pub fn write_all(&mut self, dest_dir: &Path) -> Result<(usize, usize)> {
        // Collect all non-empty builders
        let non_empty_keys: Vec<_> = self.builders.iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .map(|(idx, _)| *idx)
            .collect();

        if non_empty_keys.is_empty() {
            println!("\nNo BSA files to create (all are empty)");
            return Ok((0, 0));
        }

        // Extract builders from HashMap for parallel processing
        let builders_to_process: Vec<_> = non_empty_keys.iter()
            .filter_map(|idx| {
                self.builders.remove(idx).map(|(name, builder)| {
                    let file_count = builder.file_count();
                    (*idx, name, builder, file_count)
                })
            })
            .collect();

        let total = builders_to_process.len();
        println!("\n=== Writing {} BSA Archives (parallel) ===\n", total);

        let success_count = AtomicUsize::new(0);
        let fail_count = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let dest_dir = dest_dir.to_path_buf();

        // Process BSAs in parallel
        builders_to_process.into_par_iter().for_each(|(_, bsa_name, builder, file_count)| {
            let output_path = dest_dir.join(&bsa_name);
            let idx = completed.fetch_add(1, Ordering::SeqCst) + 1;

            println!("  [{}/{}] {} ({} files) ... building", idx, total, bsa_name, file_count);

            match builder.build(&output_path) {
                Ok(_) => {
                    println!("  [{}/{}] {} ... OK", idx, total, bsa_name);
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => {
                    println!("  [{}/{}] {} ... FAILED: {}", idx, total, bsa_name, e);
                    fail_count.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        let success = success_count.load(Ordering::SeqCst);
        let fail = fail_count.load(Ordering::SeqCst);

        println!("\nBSA Creation: {}/{} succeeded, {} failed", success, total, fail);

        Ok((success, fail))
    }

    /// Write all BSA archives with progress callback for GUI (parallel)
    /// callback(current, total, bsa_name)
    pub fn write_all_with_callback<F>(&mut self, dest_dir: &Path, callback: F) -> Result<(usize, usize)>
    where
        F: Fn(usize, usize, &str) + Sync,
    {
        // Collect all non-empty builders
        let non_empty_keys: Vec<_> = self.builders.iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .map(|(idx, _)| *idx)
            .collect();

        if non_empty_keys.is_empty() {
            return Ok((0, 0));
        }

        // Extract builders from HashMap for parallel processing
        let builders_to_process: Vec<_> = non_empty_keys.iter()
            .filter_map(|idx| {
                self.builders.remove(idx).map(|(name, builder)| {
                    (*idx, name, builder)
                })
            })
            .collect();

        let total = builders_to_process.len();
        let success_count = AtomicUsize::new(0);
        let fail_count = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let dest_dir = dest_dir.to_path_buf();

        // Process BSAs in parallel
        builders_to_process.into_par_iter().for_each(|(_, bsa_name, builder)| {
            let idx = completed.fetch_add(1, Ordering::SeqCst) + 1;
            callback(idx, total, &bsa_name);

            let output_path = dest_dir.join(&bsa_name);

            match builder.build(&output_path) {
                Ok(_) => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    fail_count.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        Ok((success_count.load(Ordering::SeqCst), fail_count.load(Ordering::SeqCst)))
    }
}

impl Default for BsaWriterManager {
    fn default() -> Self {
        Self::new()
    }
}
