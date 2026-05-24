use anyhow::{bail, Context, Result};
use ba2::tes4::{
    Archive, ArchiveFlags, ArchiveKey, ArchiveOptions, ArchiveTypes, Directory, DirectoryKey,
    File as BsaFile, Version,
};
use ba2::CompressableFrom;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read as IoRead, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

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
    /// Create with specific archive settings
    pub fn with_settings(
        staging_dir: &Path,
        flags: ArchiveFlags,
        types: ArchiveTypes,
        version: Version,
    ) -> Result<Self> {
        // Create staging file in the output directory (not temp - temp may be tmpfs with limited space)
        let staging_path = staging_dir.join(format!(
            ".ttw_bsa_staging_{}_{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let file = File::create(&staging_path).with_context(|| {
            format!("Failed to create staging file: {}", staging_path.display())
        })?;

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
            (
                normalized[..idx].to_string(),
                normalized[idx + 1..].to_string(),
            )
        } else {
            (".".to_string(), normalized.to_string())
        };

        // Write to staging file (under lock)
        let offset = {
            let mut writer_guard = self
                .staging_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA staging writer lock poisoned"))?;
            let writer = writer_guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Staging file already closed"))?;
            let offset = self.current_offset.load(Ordering::SeqCst);

            writer
                .write_all(&data)
                .with_context(|| "Failed to write to BSA staging file")?;

            self.current_offset
                .fetch_add(data_len as u64, Ordering::SeqCst);
            offset
        };

        // Store only metadata (under separate lock to minimize contention)
        {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA staging entries lock poisoned"))?;
            entries.entry(dir_path).or_default().insert(
                file_name,
                StagingEntry {
                    offset,
                    size: data_len,
                },
            );
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
    /// Phase 1: Read files from staging
    /// Phase 2: Compress files in PARALLEL across all cores
    /// Phase 3: Assemble archive from pre-compressed files and write
    pub fn build(self, output_path: &Path) -> Result<()> {
        use ba2::tes4::FileCompressionOptions;

        if self.is_empty() {
            bail!("Cannot create empty BSA archive");
        }

        let should_compress = self.archive_flags.contains(ArchiveFlags::COMPRESSED);

        // Flush and close the staging writer
        {
            let mut writer_guard = self
                .staging_writer
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA staging writer lock poisoned"))?;
            if let Some(mut writer) = writer_guard.take() {
                writer.flush()?;
            }
        }

        // Get the entries (take ownership via std::mem::take)
        let entries = {
            let mut entries_guard = self
                .entries
                .lock()
                .map_err(|_| anyhow::anyhow!("BSA staging entries lock poisoned"))?;
            std::mem::take(&mut *entries_guard)
        };

        // Open staging file for reading
        let staging_file = File::open(&self.staging_path)
            .with_context(|| "Failed to open staging file for reading")?;
        let mut staging_reader = BufReader::with_capacity(1024 * 1024, staging_file);

        // Phase 1: Read all files from staging into flat vec for parallel processing
        struct FileEntry {
            dir_path: String,
            file_name: String,
            data: Vec<u8>,
        }

        let mut file_entries: Vec<FileEntry> = Vec::new();
        for (dir_path, files) in entries {
            for (file_name, entry) in files {
                let mut data = vec![0u8; entry.size];
                staging_reader.seek(SeekFrom::Start(entry.offset))?;
                staging_reader.read_exact(&mut data)?;
                file_entries.push(FileEntry {
                    dir_path: dir_path.clone(),
                    file_name,
                    data,
                });
            }
        }

        // Close staging reader and clean up staging file
        drop(staging_reader);
        let _ = fs::remove_file(&self.staging_path);

        // Phase 2: Compress files in PARALLEL (the expensive part)
        // Consume file_entries to avoid cloning data
        let version = self.version;
        let processed: Result<Vec<(String, String, BsaFile<'static>)>> = file_entries
            .into_par_iter()
            .map(|entry| {
                let uncompressed = BsaFile::from_decompressed(entry.data.into_boxed_slice());

                let file = if should_compress {
                    let compression_options =
                        FileCompressionOptions::builder().version(version).build();
                    uncompressed
                        .compress(&compression_options)
                        .with_context(|| {
                            format!("Failed to compress: {}/{}", entry.dir_path, entry.file_name)
                        })?
                } else {
                    uncompressed
                };

                Ok((entry.dir_path, entry.file_name, file))
            })
            .collect();

        let processed = processed?;

        // Phase 3: Assemble archive from pre-compressed files
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

        // Write the archive (all files already compressed, just writes bytes)
        let file = File::create(output_path)
            .with_context(|| format!("Failed to create BSA file: {}", output_path.display()))?;
        let mut writer = BufWriter::new(file);

        archive
            .write(&mut writer, &options)
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
