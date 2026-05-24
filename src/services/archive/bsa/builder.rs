use anyhow::{bail, Context, Result};
use ba2::tes4::{
    Archive, ArchiveFlags, ArchiveKey, ArchiveOptions, ArchiveTypes, Directory, DirectoryKey,
    File as BsaFile, Version,
};
use ba2::CompressableFrom;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io::BufWriter;
use std::path::Path;

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
            self.version = Version::v103; // Oblivion
                                          // Oblivion BSA flags - NO compression for decompressed output
                                          // Use minimal flags: directory strings + file strings only
            self.archive_flags = ArchiveFlags::DIRECTORY_STRINGS | ArchiveFlags::FILE_STRINGS;
        } else {
            self.version = Version::v104; // Fallout 3/NV (default)
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
            (
                normalized[..idx].to_string(),
                normalized[idx + 1..].to_string(),
            )
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

    /// Build and write the BSA archive to disk with parallel compression
    pub fn build(self, output_path: &Path) -> Result<()> {
        use ba2::tes4::FileCompressionOptions;

        if self.is_empty() {
            bail!("Cannot create empty BSA archive");
        }

        let should_compress = self.archive_flags.contains(ArchiveFlags::COMPRESSED);

        // Flatten to entries for parallel processing
        struct Entry {
            dir_path: String,
            file_name: String,
            data: Vec<u8>,
        }

        let entries: Vec<Entry> = self
            .files
            .into_iter()
            .flat_map(|(dir_path, files)| {
                files.into_iter().map(move |(file_name, data)| Entry {
                    dir_path: dir_path.clone(),
                    file_name,
                    data,
                })
            })
            .collect();

        // Parallel compress
        let version = self.version;
        let processed: Result<Vec<(String, String, BsaFile<'static>)>> = entries
            .par_iter()
            .map(|entry| {
                let uncompressed =
                    BsaFile::from_decompressed(entry.data.clone().into_boxed_slice());
                let file = if should_compress {
                    let opts = FileCompressionOptions::builder().version(version).build();
                    uncompressed.compress(&opts).with_context(|| {
                        format!("Failed to compress: {}/{}", entry.dir_path, entry.file_name)
                    })?
                } else {
                    uncompressed
                };
                Ok((entry.dir_path.clone(), entry.file_name.clone(), file))
            })
            .collect();

        let processed = processed?;

        // Assemble archive
        let mut archive = Archive::new();
        for (dir_path, file_name, file) in processed {
            let archive_key = ArchiveKey::from(dir_path.as_bytes());
            let directory_key = DirectoryKey::from(file_name.as_bytes());
            match archive.get_mut(&archive_key) {
                Some(directory) => {
                    directory.insert(directory_key, file);
                }
                None => {
                    let mut directory = Directory::default();
                    directory.insert(directory_key, file);
                    archive.insert(archive_key, directory);
                }
            }
        }

        let options = ArchiveOptions::builder()
            .version(self.version)
            .flags(self.archive_flags)
            .types(self.archive_types)
            .build();

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = fs::File::create(output_path)
            .with_context(|| format!("Failed to create BSA file: {}", output_path.display()))?;
        let mut writer = BufWriter::new(file);

        archive
            .write(&mut writer, &options)
            .with_context(|| format!("Failed to write BSA: {}", output_path.display()))?;

        Ok(())
    }
}

impl Default for BsaBuilder {
    fn default() -> Self {
        Self::new()
    }
}
