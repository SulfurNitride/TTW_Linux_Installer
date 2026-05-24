use anyhow::{Context, Result};
#[cfg(not(feature = "dream-reader"))]
use ba2::tes4::Archive;
#[cfg(not(feature = "dream-reader"))]
use ba2::{ByteSlice, Reader};
#[cfg(feature = "dream-reader")]
use dream_archive::bsa::tes4::Archive;
use indicatif::ProgressBar;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tracing::warn;

use super::archive_progress_style;
#[cfg(not(feature = "dream-reader"))]
use super::extractor::MpiExtractor;
#[cfg(not(feature = "dream-reader"))]
use super::LZ4_FRAME_MAGIC;

/// In-memory MPI package store.
///
/// Holds all extracted files in a HashMap for instant lookups, avoiding a
/// 27k+ file extraction pass and case-insensitive filesystem crawling.
pub struct MpiStore {
    /// Files stored by lowercase normalized path -> data
    files: HashMap<String, Vec<u8>>,
    /// Total bytes stored
    total_bytes: usize,
}

impl MpiStore {
    /// Load an MPI file entirely into memory.
    pub fn load(mpi_path: &Path) -> Result<Self> {
        if !mpi_path.exists() {
            anyhow::bail!("MPI file not found: {}", mpi_path.display());
        }

        println!(
            "\nLoading MPI package into memory: {}",
            mpi_path.file_name().unwrap_or_default().to_string_lossy()
        );

        #[cfg(not(feature = "dream-reader"))]
        let (files, total_bytes, _total_files) = {
            let (archive, options) =
                Archive::read(mpi_path).context("Failed to open MPI archive")?;
            let compression_options: ba2::tes4::FileCompressionOptions = (&options).into();

            struct FileEntry<'a> {
                path: String,
                file: &'a ba2::tes4::File<'a>,
            }

            let mut entries: Vec<FileEntry> = Vec::new();
            for (dir_key, folder) in archive.iter() {
                let dir_name =
                    String::from_utf8_lossy(dir_key.name().as_bytes()).replace('\\', "/");
                for (file_key, file) in folder.iter() {
                    let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_string();
                    let path = if dir_name.is_empty() || dir_name == "." {
                        file_name
                    } else {
                        format!("{}/{}", dir_name, file_name)
                    };
                    entries.push(FileEntry { path, file });
                }
            }

            let total_files = entries.len();
            println!("  {} files in archive, decompressing...", total_files);

            let pb = ProgressBar::new(total_files as u64);
            pb.set_style(archive_progress_style());

            let files: Mutex<HashMap<String, Vec<u8>>> =
                Mutex::new(HashMap::with_capacity(total_files));
            let total_bytes = AtomicUsize::new(0);

            entries.par_iter().for_each(|entry| {
                let data = if entry.file.is_compressed() {
                    let compressed = entry.file.as_bytes();
                    if compressed.len() >= 4 && compressed[0..4] == LZ4_FRAME_MAGIC {
                        MpiExtractor::decompress_lz4_frame(compressed).ok()
                    } else {
                        entry
                            .file
                            .decompress(&compression_options)
                            .map(|d| d.as_bytes().to_vec())
                            .ok()
                    }
                } else {
                    Some(entry.file.as_bytes().to_vec())
                };

                if let Some(data) = data {
                    let key = entry.path.to_lowercase();
                    match files.lock() {
                        Ok(mut files) => {
                            total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                            files.insert(key, data);
                        }
                        Err(_) => warn!("MPI store lock poisoned while loading {}", entry.path),
                    }
                }

                pb.inc(1);
            });

            pb.finish_with_message("Loaded into memory");
            (
                files
                    .into_inner()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                total_bytes.load(Ordering::Relaxed),
                total_files,
            )
        };

        #[cfg(feature = "dream-reader")]
        let (files, total_bytes, _total_files) = {
            let archive = Archive::open_path(mpi_path).context("Failed to open MPI archive")?;
            let entries: Vec<_> = archive
                .entries()
                .iter()
                .filter_map(|entry| {
                    entry
                        .path()
                        .map(|path| (path.to_string().replace('\\', "/"), entry.clone()))
                })
                .collect();

            let total_files = entries.len();
            println!(
                "  {} files in archive, decompressing with dream_archive...",
                total_files
            );

            let pb = ProgressBar::new(total_files as u64);
            pb.set_style(archive_progress_style());

            let files: Mutex<HashMap<String, Vec<u8>>> =
                Mutex::new(HashMap::with_capacity(total_files));
            let total_bytes = AtomicUsize::new(0);

            entries.par_iter().for_each(|(path, entry)| {
                if let Ok(data) = archive.read_entry(entry) {
                    match files.lock() {
                        Ok(mut files) => {
                            total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                            files.insert(path.to_lowercase(), data);
                        }
                        Err(_) => warn!("MPI store lock poisoned while loading {}", path),
                    }
                }
                pb.inc(1);
            });

            pb.finish_with_message("Loaded into memory");
            (
                files
                    .into_inner()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                total_bytes.load(Ordering::Relaxed),
                total_files,
            )
        };

        println!(
            "  MPI loaded: {} files, {:.1} MB in memory",
            files.len(),
            total_bytes as f64 / 1024.0 / 1024.0
        );

        Ok(Self { files, total_bytes })
    }

    /// Get a file by path (case-insensitive, handles both / and \ separators).
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        let normalized = path.replace('\\', "/").to_lowercase();
        self.files.get(&normalized).map(|v| v.as_slice())
    }

    /// Get the manifest file (tries common locations).
    pub fn get_manifest(&self) -> Option<&[u8]> {
        for name in ["_package/index.json", "manifest.json", "index.json"] {
            if let Some(data) = self.get(name) {
                return Some(data);
            }
        }
        None
    }

    /// Total bytes stored in memory.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Number of files stored.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}
