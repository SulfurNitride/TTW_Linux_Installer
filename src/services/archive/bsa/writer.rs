use super::StreamingBsaBuilder;
use anyhow::{Context, Result};
use ba2::tes4::{ArchiveFlags, ArchiveTypes, Version};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// These prefixes are internal identifiers, not part of actual output filenames
fn strip_bsa_prefix(name: &str) -> &str {
    // Common prefixes used in MPI manifests to identify BSA sources/targets
    // Check case-insensitively
    let name_lower = name.to_lowercase();

    // NOTE: "new " is NOT stripped - it's part of the actual filename (e.g., "New Fallout - Textures2.bsa")
    // Only strip prefixes that are purely internal identifiers
    const PREFIXES: &[&str] = &[
        "ttw ",  // Tale of Two Wastelands internal identifier
        "fo3 ",  // Fallout 3 source identifier
        "fnv ",  // Fallout New Vegas source identifier
        "tes4 ", // Oblivion source identifier
    ];

    for prefix in PREFIXES {
        if name_lower.starts_with(prefix) {
            return &name[prefix.len()..];
        }
    }
    name
}
/// Manages multiple BSA archives being built during installation
/// Uses streaming builders to keep RAM usage low (~5MB vs ~10GB for 50k files)
pub struct BsaWriterManager {
    /// BSA builders keyed by location index
    /// Uses StreamingBsaBuilder for disk-backed storage instead of RAM
    builders: HashMap<i32, (String, StreamingBsaBuilder)>, // (bsa_name, builder)
    /// Directory for staging files (uses output dir, not temp)
    staging_dir: PathBuf,
}

impl BsaWriterManager {
    pub fn new(staging_dir: PathBuf) -> Self {
        Self {
            builders: HashMap::new(),
            staging_dir,
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
            Version::v103 // Oblivion
        } else {
            // Use archive_type for non-Oblivion, or default to v104
            match archive_type {
                Some(103) => Version::v103, // Direct version number
                Some(104) => Version::v104,
                Some(105) => Version::v105,
                _ => Version::v104, // Default to FO3/FNV
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

        // Create streaming builder (writes to staging file in output dir, not temp)
        let builder = StreamingBsaBuilder::with_settings(&self.staging_dir, flags, types, version)
            .with_context(|| format!("Failed to create streaming BSA builder for {}", bsa_name))?;

        // Get version string for logging
        let version_str = match version {
            Version::v103 => "v103 (Oblivion)",
            Version::v104 => "v104 (FO3/FNV)",
            Version::v105 => "v105 (SSE)",
        };

        info!(
            "Registered BSA target: Location[{}] = {} -> {} [{}, flags=0x{:x}, types=0x{:x}]",
            location_index,
            bsa_name,
            output_name,
            version_str,
            flags.bits(),
            types.bits()
        );

        self.builders
            .insert(location_index, (output_name.to_string(), builder));
        Ok(())
    }

    /// Check if a location is a registered BSA target
    pub fn is_bsa_location(&self, location_index: i32) -> bool {
        self.builders.contains_key(&location_index)
    }

    /// Get all registered BSA location indices
    pub fn bsa_location_indices(&self) -> Vec<i32> {
        self.builders.keys().copied().collect()
    }

    /// Get the output name for a BSA location
    pub fn bsa_name(&self, location_index: i32) -> Option<&str> {
        self.builders
            .get(&location_index)
            .map(|(name, _)| name.as_str())
    }

    /// Add a file to a BSA (thread-safe, writes to disk immediately)
    pub fn add_file(&self, location_index: i32, file_path: &str, data: Vec<u8>) -> Result<()> {
        let (_, builder) = self
            .builders
            .get(&location_index)
            .ok_or_else(|| anyhow::anyhow!("Location {} is not a BSA target", location_index))?;

        builder.add_file(file_path, data)
    }

    /// Get file count for a specific BSA
    pub fn file_count(&self, location_index: i32) -> Option<usize> {
        self.builders
            .get(&location_index)
            .map(|(_, b)| b.file_count())
    }

    /// Take a builder out of the manager for independent building.
    /// Returns (bsa_name, builder) if the location exists and has files.
    pub fn take_builder(&mut self, location_index: i32) -> Option<(String, StreamingBsaBuilder)> {
        self.builders
            .remove(&location_index)
            .filter(|(_, b)| !b.is_empty())
    }

    /// Build a single BSA by location index. Removes it from the manager.
    /// Returns (bsa_name, output_size_bytes) on success.
    pub fn build_single(
        &mut self,
        location_index: i32,
        dest_dir: &Path,
    ) -> Result<Option<(String, u64)>> {
        let (bsa_name, builder) = match self.take_builder(location_index) {
            Some(b) => b,
            None => return Ok(None),
        };

        let output_path = dest_dir.join(&bsa_name);
        let file_count = builder.file_count();

        info!("Building BSA: {} ({} files)", bsa_name, file_count);
        builder
            .build(&output_path)
            .with_context(|| format!("Failed to build BSA: {}", bsa_name))?;

        let size = fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
        info!("Built BSA: {} ({} MB)", bsa_name, size / 1024 / 1024);

        Ok(Some((bsa_name, size)))
    }

    /// Write all BSA archives to the destination directory (one at a time, full CPU)
    /// Each BSA's compression uses all cores via par_iter internally.
    /// Building one at a time keeps RAM to ~1 BSA's worth instead of all 26.
    pub fn write_all(&mut self, dest_dir: &Path) -> Result<(usize, usize)> {
        // Collect all non-empty builders
        let non_empty_keys: Vec<_> = self
            .builders
            .iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .map(|(idx, _)| *idx)
            .collect();

        if non_empty_keys.is_empty() {
            println!("\nNo BSA files to create (all are empty)");
            return Ok((0, 0));
        }

        // Extract builders, sorted largest first for better progress visibility
        let mut builders_to_process: Vec<_> = non_empty_keys
            .iter()
            .filter_map(|idx| {
                self.builders.remove(idx).map(|(name, builder)| {
                    let file_count = builder.file_count();
                    (*idx, name, builder, file_count)
                })
            })
            .collect();
        builders_to_process.sort_by_key(|b| std::cmp::Reverse(b.3));

        let total = builders_to_process.len();
        println!(
            "\n=== Writing {} BSA Archives (sequential, full CPU per BSA) ===\n",
            total
        );

        let mut success = 0usize;
        let mut fail = 0usize;

        for (idx, (_, bsa_name, builder, file_count)) in builders_to_process.into_iter().enumerate()
        {
            let output_path = dest_dir.join(&bsa_name);

            println!(
                "  [{}/{}] {} ({} files) ... building",
                idx + 1,
                total,
                bsa_name,
                file_count
            );

            match builder.build(&output_path) {
                Ok(_) => {
                    let size_mb = fs::metadata(&output_path)
                        .map(|m| m.len() / 1024 / 1024)
                        .unwrap_or(0);
                    println!(
                        "  [{}/{}] {} ... OK ({} MB)",
                        idx + 1,
                        total,
                        bsa_name,
                        size_mb
                    );
                    success += 1;
                }
                Err(e) => {
                    println!("  [{}/{}] {} ... FAILED: {}", idx + 1, total, bsa_name, e);
                    fail += 1;
                }
            }
        }

        println!(
            "\nBSA Creation: {}/{} succeeded, {} failed",
            success, total, fail
        );

        Ok((success, fail))
    }

    /// Write all BSA archives with progress callback for GUI (sequential, full CPU per BSA)
    /// callback(current, total, bsa_name)
    pub fn write_all_with_callback<F>(
        &mut self,
        dest_dir: &Path,
        callback: F,
    ) -> Result<(usize, usize)>
    where
        F: Fn(usize, usize, &str) + Sync,
    {
        let non_empty_keys: Vec<_> = self
            .builders
            .iter()
            .filter(|(_, (_, b))| !b.is_empty())
            .map(|(idx, _)| *idx)
            .collect();

        if non_empty_keys.is_empty() {
            return Ok((0, 0));
        }

        let mut builders_to_process: Vec<_> = non_empty_keys
            .iter()
            .filter_map(|idx| {
                self.builders
                    .remove(idx)
                    .map(|(name, builder)| (*idx, name, builder))
            })
            .collect();
        builders_to_process.sort_by_key(|b| std::cmp::Reverse(b.2.file_count()));

        let total = builders_to_process.len();
        let mut success = 0usize;
        let mut fail = 0usize;

        for (idx, (_, bsa_name, builder)) in builders_to_process.into_iter().enumerate() {
            callback(idx + 1, total, &bsa_name);
            let output_path = dest_dir.join(&bsa_name);

            match builder.build(&output_path) {
                Ok(_) => {
                    success += 1;
                }
                Err(_) => {
                    fail += 1;
                }
            }
        }

        Ok((success, fail))
    }
}
