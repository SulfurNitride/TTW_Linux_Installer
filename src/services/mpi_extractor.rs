use anyhow::{Result, Context};
use ba2::tes4::Archive;
use ba2::{Reader, ByteSlice};
use std::path::{Path, PathBuf};
use std::fs;
use indicatif::{ProgressBar, ProgressStyle};

/// LZ4 frame magic number
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];

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

    /// Extract .mpi file to a specific directory
    pub fn extract_to(mpi_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        if !mpi_path.exists() {
            anyhow::bail!("MPI file not found: {}", mpi_path.display());
        }

        println!("\nExtracting MPI package: {}", mpi_path.file_name().unwrap_or_default().to_string_lossy());
        println!("Extracting to: {}", output_dir.display());
        println!("This may take a few minutes...\n");

        // Create output directory for extraction
        fs::create_dir_all(output_dir)?;
        let temp_dir = output_dir.to_path_buf();

        // Open the BSA/MPI archive using ba2
        let (archive, _) = Archive::read(mpi_path)
            .context("Failed to open MPI archive")?;

        // Count total files
        let total_files: usize = archive.iter()
            .map(|(_, folder)| folder.iter().count())
            .sum();

        println!("Archive opened: {} files found", total_files);

        let pb = ProgressBar::new(total_files as u64);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        let mut extracted = 0;
        let mut failed = 0;

        for (dir_key, folder) in archive.iter() {
            let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes())
                .replace('\\', "/");

            for (file_key, file) in folder.iter() {
                let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_string();
                let relative_path = if dir_name.is_empty() {
                    file_name.clone()
                } else {
                    format!("{}/{}", dir_name, file_name)
                };

                let output_path = temp_dir.join(&relative_path);

                // Create parent directories
                if let Some(parent) = output_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Extract file
                match Self::extract_file(file, &output_path) {
                    Ok(_) => extracted += 1,
                    Err(e) => {
                        if failed < 3 {
                            eprintln!("  Warning: Failed to extract {}: {}", relative_path, e);
                        }
                        failed += 1;
                    }
                }

                pb.inc(1);
            }
        }

        pb.finish_with_message("Extraction complete");

        println!("\nMPI extraction complete: {} files extracted", extracted);
        if failed > 0 {
            println!("{} files failed to extract", failed);
        }

        Ok(temp_dir)
    }

    fn extract_file(file: &ba2::tes4::File, output_path: &Path) -> Result<()> {
        let data = if file.is_compressed() {
            let compressed = file.as_bytes();

            // Check if this is LZ4 compressed (BSA v105/SSE format)
            if compressed.len() >= 4 && compressed[0..4] == LZ4_FRAME_MAGIC {
                // Use LZ4 frame decompression
                Self::decompress_lz4_frame(compressed)?
            } else {
                // Try standard ba2 decompression (zlib)
                file.decompress(&Default::default())?.as_bytes().to_vec()
            }
        } else {
            file.as_bytes().to_vec()
        };

        fs::write(output_path, &data)?;
        Ok(())
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
