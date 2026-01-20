use anyhow::{Result, Context, bail};
use ba2::tes4::{Archive, ArchiveKey, ArchiveOptions, ArchiveFlags, ArchiveTypes, Directory, DirectoryKey, File as BsaFile, Version};
use ba2::{ByteSlice, CompressableFrom, Reader};
use std::collections::HashMap;
use std::path::Path;
use std::fs;
use std::io::BufWriter;

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
pub struct BsaHandler {
    /// Cache of opened archives for reading
    archive_cache: HashMap<String, Archive<'static>>,
}

impl BsaHandler {
    pub fn new() -> Self {
        Self {
            archive_cache: HashMap::new(),
        }
    }

    /// Extract a file from a BSA archive
    pub fn extract_file(&mut self, bsa_path: &Path, file_path: &str) -> Result<Vec<u8>> {
        let bsa_key = bsa_path.to_string_lossy().to_string();

        // Open archive if not cached
        if !self.archive_cache.contains_key(&bsa_key) {
            let (archive, _) = Archive::read(bsa_path)
                .with_context(|| format!("Failed to open BSA: {}", bsa_path.display()))?;
            // We need to leak the archive to get a static lifetime for the cache
            // This is safe since we're caching for the lifetime of the handler
            let archive: Archive<'static> = unsafe { std::mem::transmute(archive) };
            self.archive_cache.insert(bsa_key.clone(), archive);
        }

        let archive = self.archive_cache.get(&bsa_key).unwrap();

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

    /// Clear the archive cache
    pub fn clear_cache(&mut self) {
        self.archive_cache.clear();
    }
}

impl Default for BsaHandler {
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

/// Manages multiple BSA archives being built during installation
pub struct BsaWriterManager {
    /// BSA builders keyed by location index
    builders: HashMap<i32, (String, BsaBuilder)>, // (bsa_name, builder)
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
    ) {
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

        let builder = BsaBuilder {
            files: HashMap::new(),
            archive_flags: flags,
            archive_types: types,
            version,
        };

        // Get version string for logging
        let version_str = match version {
            Version::v103 => "v103 (Oblivion)",
            Version::v104 => "v104 (FO3/FNV)",
            Version::v105 => "v105 (SSE)",
        };

        self.builders.insert(location_index, (output_name.to_string(), builder));
        println!("  Registered BSA target: Location[{}] = {} -> {} [{}, flags=0x{:x}, types=0x{:x}]",
            location_index, bsa_name, output_name, version_str, flags.bits(), types.bits());
    }

    /// Check if a location is a registered BSA target
    pub fn is_bsa_location(&self, location_index: i32) -> bool {
        self.builders.contains_key(&location_index)
    }

    /// Add a file to a BSA
    pub fn add_file(&mut self, location_index: i32, file_path: &str, data: Vec<u8>) -> Result<()> {
        let (_, builder) = self.builders.get_mut(&location_index)
            .ok_or_else(|| anyhow::anyhow!("Location {} is not a BSA target", location_index))?;

        builder.add_file(file_path, data);
        Ok(())
    }

    /// Write all BSA archives to the destination directory
    pub fn write_all(&self, dest_dir: &Path) -> Result<(usize, usize)> {
        let non_empty: Vec<_> = self.builders.iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .collect();

        if non_empty.is_empty() {
            println!("\nNo BSA files to create (all are empty)");
            return Ok((0, 0));
        }

        println!("\n=== Writing {} BSA Archives ===\n", non_empty.len());

        let mut success_count = 0;
        let mut fail_count = 0;

        for (idx, (location_index, (bsa_name, _))) in non_empty.iter().enumerate() {
            let output_path = dest_dir.join(bsa_name);
            let (_, builder) = self.builders.get(location_index).unwrap();

            print!("  [{}/{}] {} ({} files) ... ",
                idx + 1, non_empty.len(), bsa_name, builder.file_count());

            // We need to rebuild since we can't move out of the reference
            // This is a limitation - in a real implementation we'd restructure this
            match self.write_single_bsa(**location_index, &output_path) {
                Ok(_) => {
                    println!("OK");
                    success_count += 1;
                }
                Err(e) => {
                    println!("FAILED: {}", e);
                    fail_count += 1;
                }
            }
        }

        println!("\nBSA Creation: {}/{} succeeded, {} failed",
            success_count, non_empty.len(), fail_count);

        Ok((success_count, fail_count))
    }

    /// Write all BSA archives with progress callback for GUI
    /// callback(current, total, bsa_name)
    pub fn write_all_with_callback<F>(&self, dest_dir: &Path, callback: F) -> Result<(usize, usize)>
    where
        F: Fn(usize, usize, &str),
    {
        let non_empty: Vec<_> = self.builders.iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .collect();

        if non_empty.is_empty() {
            return Ok((0, 0));
        }

        let mut success_count = 0;
        let mut fail_count = 0;

        for (idx, (location_index, (bsa_name, _))) in non_empty.iter().enumerate() {
            callback(idx + 1, non_empty.len(), bsa_name);

            let output_path = dest_dir.join(bsa_name);

            match self.write_single_bsa(**location_index, &output_path) {
                Ok(_) => {
                    success_count += 1;
                }
                Err(_) => {
                    fail_count += 1;
                }
            }
        }

        Ok((success_count, fail_count))
    }

    fn write_single_bsa(&self, location_index: i32, output_path: &Path) -> Result<()> {
        let (bsa_name, builder) = self.builders.get(&location_index)
            .ok_or_else(|| anyhow::anyhow!("Location {} not found", location_index))?;

        if builder.is_empty() {
            bail!("BSA {} is empty", bsa_name);
        }

        // Build the archive structure
        let archive: Archive = builder.files.iter().map(|(dir_path, files)| {
            let directory: Directory = files.iter().map(|(file_name, data)| {
                let file = BsaFile::from_decompressed(&data[..]);
                (DirectoryKey::from(file_name.as_bytes()), file)
            }).collect();
            (ArchiveKey::from(dir_path.as_bytes()), directory)
        }).collect();

        // Set up write options
        let options = ArchiveOptions::builder()
            .version(builder.version)
            .flags(builder.archive_flags)
            .types(builder.archive_types)
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

impl Default for BsaWriterManager {
    fn default() -> Self {
        Self::new()
    }
}
