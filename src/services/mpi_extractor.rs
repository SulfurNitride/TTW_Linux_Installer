use anyhow::{Result, Context};
use ba2::tes4::Archive;
use ba2::{Reader, ByteSlice};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::fs;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::info;

/// LZ4 frame magic number
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];

/// In-memory MPI package store.
/// Holds all extracted files in a HashMap for instant lookups.
/// Avoids writing 27k+ files to disk and eliminates case-insensitive filesystem crawling.
pub struct MpiStore {
    /// Files stored by lowercase normalized path → data
    files: HashMap<String, Vec<u8>>,
    /// Total bytes stored
    total_bytes: usize,
}

impl MpiStore {
    /// Load an MPI file entirely into memory.
    /// Returns the store with all files decompressed and indexed by lowercase path.
    pub fn load(mpi_path: &Path) -> Result<Self> {
        if !mpi_path.exists() {
            anyhow::bail!("MPI file not found: {}", mpi_path.display());
        }

        println!("\nLoading MPI package into memory: {}",
            mpi_path.file_name().unwrap_or_default().to_string_lossy());

        let (archive, options) = Archive::read(mpi_path)
            .context("Failed to open MPI archive")?;
        let compression_options: ba2::tes4::FileCompressionOptions = (&options).into();

        // Collect all file entries with references
        struct FileEntry<'a> {
            path: String,
            file: &'a ba2::tes4::File<'a>,
        }

        let mut entries: Vec<FileEntry> = Vec::new();
        for (dir_key, folder) in archive.iter() {
            let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes())
                .replace('\\', "/");
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
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        // Decompress all files in parallel, collect into thread-safe map
        let files: Mutex<HashMap<String, Vec<u8>>> = Mutex::new(HashMap::with_capacity(total_files));
        let total_bytes = AtomicUsize::new(0);

        entries.par_iter().for_each(|entry| {
            let data = if entry.file.is_compressed() {
                let compressed = entry.file.as_bytes();
                if compressed.len() >= 4 && compressed[0..4] == LZ4_FRAME_MAGIC {
                    MpiExtractor::decompress_lz4_frame(compressed).ok()
                } else {
                    entry.file.decompress(&compression_options)
                        .map(|d| d.as_bytes().to_vec())
                        .ok()
                }
            } else {
                Some(entry.file.as_bytes().to_vec())
            };

            if let Some(data) = data {
                total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                let key = entry.path.to_lowercase();
                files.lock().unwrap().insert(key, data);
            }

            pb.inc(1);
        });

        pb.finish_with_message("Loaded into memory");

        let files = files.into_inner().unwrap();
        let total_bytes = total_bytes.load(Ordering::Relaxed);

        println!("  MPI loaded: {} files, {:.1} MB in memory",
            files.len(), total_bytes as f64 / 1024.0 / 1024.0);

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

/// Extracts .mpi files (BSA archives) to a temporary directory
pub struct MpiExtractor;

impl MpiExtractor {
    /// Check if the path is an .mpi file that needs extraction
    pub fn is_mpi_file(path: &Path) -> bool {
        path.is_file()
            && path.extension()
                .map(|e| e.eq_ignore_ascii_case("mpi"))
                .unwrap_or(false)
    }

    /// Extract .mpi file to a directory
    /// If output_dir is None, extracts to a subdirectory next to the MPI file
    /// Returns the path to the extracted directory
    pub fn extract_to_temp(mpi_path: &Path) -> Result<PathBuf> {
        // Extract next to the MPI file by default
        let default_output = mpi_path.parent()
            .unwrap_or(Path::new("."))
            .join(format!("ttw_mpi_extracted_{}", uuid_simple()));
        Self::extract_to(mpi_path, &default_output)
    }

    /// Extract .mpi file to a specific directory (parallel)
    ///
    /// Phase 1: Scan archive and collect all file entries (single-threaded, fast)
    /// Phase 2: Pre-create all directories (single-threaded, eliminates per-file create_dir_all)
    /// Phase 3: Decompress + write files in parallel across all CPU cores
    pub fn extract_to(mpi_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        if !mpi_path.exists() {
            anyhow::bail!("MPI file not found: {}", mpi_path.display());
        }

        println!("\nExtracting MPI package: {}", mpi_path.file_name().unwrap_or_default().to_string_lossy());
        println!("Extracting to: {}", output_dir.display());
        println!("This may take a few minutes...\n");

        fs::create_dir_all(output_dir)?;
        let temp_dir = output_dir.to_path_buf();

        let (archive, options) = Archive::read(mpi_path)
            .context("Failed to open MPI archive")?;
        let compression_options: ba2::tes4::FileCompressionOptions = (&options).into();

        // Phase 1: Collect all file entries and their data references
        struct FileEntry<'a> {
            relative_path: String,
            file: &'a ba2::tes4::File<'a>,
        }

        let mut entries: Vec<FileEntry> = Vec::new();
        let mut dirs_needed: HashSet<PathBuf> = HashSet::new();

        for (dir_key, folder) in archive.iter() {
            let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes())
                .replace('\\', "/");

            for (file_key, file) in folder.iter() {
                let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_string();
                let relative_path = if dir_name.is_empty() || dir_name == "." {
                    file_name
                } else {
                    format!("{}/{}", dir_name, file_name)
                };

                // Track unique parent directories
                let output_path = temp_dir.join(&relative_path);
                if let Some(parent) = output_path.parent() {
                    dirs_needed.insert(parent.to_path_buf());
                }

                entries.push(FileEntry { relative_path, file });
            }
        }

        let total_files = entries.len();
        println!("Archive opened: {} files found", total_files);

        // Phase 2: Pre-create all directories (single pass, no redundant checks)
        for dir in &dirs_needed {
            fs::create_dir_all(dir)?;
        }

        // Phase 3: Decompress + write in parallel
        let pb = ProgressBar::new(total_files as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        let extracted = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);

        entries.par_iter().for_each(|entry| {
            let output_path = temp_dir.join(&entry.relative_path);

            let data = if entry.file.is_compressed() {
                let compressed = entry.file.as_bytes();
                if compressed.len() >= 4 && compressed[0..4] == LZ4_FRAME_MAGIC {
                    Self::decompress_lz4_frame(compressed)
                } else {
                    entry.file.decompress(&compression_options)
                        .map(|d| d.as_bytes().to_vec())
                        .map_err(|e| anyhow::anyhow!("{}", e))
                }
            } else {
                Ok(entry.file.as_bytes().to_vec())
            };

            match data {
                Ok(bytes) => {
                    if fs::write(&output_path, &bytes).is_ok() {
                        extracted.fetch_add(1, Ordering::Relaxed);
                    } else {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(_) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                }
            }

            pb.inc(1);
        });

        pb.finish_with_message("Extraction complete");

        let extracted = extracted.load(Ordering::Relaxed);
        let failed = failed.load(Ordering::Relaxed);

        println!("\nMPI extraction complete: {} files extracted", extracted);
        if failed > 0 {
            println!("{} files failed to extract", failed);
        }

        Ok(temp_dir)
    }

    /// Decompress LZ4 frame format data
    fn decompress_lz4_frame(compressed: &[u8]) -> Result<Vec<u8>> {
        use lz4_flex::frame::FrameDecoder;
        use std::io::Read;

        let mut decoder = FrameDecoder::new(compressed);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)
            .context("LZ4 frame decompression failed")?;
        Ok(decompressed)
    }

    /// Clean up a temporary extraction directory
    pub fn cleanup_temp(temp_dir: &Path) -> Result<()> {
        // Safety check - only delete our temp directories
        let dir_name = temp_dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Allow various temp directory patterns we use
        let is_safe = dir_name.starts_with("ttw_mpi_")
            || dir_name.starts_with(".ttw_mpi_")
            || dir_name == ".mpi_package"
            || dir_name.starts_with("mpi_extracted_");

        if !is_safe {
            anyhow::bail!("Refusing to delete directory that doesn't match expected MPI temp pattern: {}", dir_name);
        }

        if temp_dir.exists() {
            println!("\nCleaning up temporary files...");
            fs::remove_dir_all(temp_dir)?;
        }

        Ok(())
    }
}

/// Generate a simple UUID-like string
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}{:x}", duration.as_secs(), duration.subsec_nanos())
}
