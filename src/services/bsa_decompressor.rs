use anyhow::{Result, Context, bail};
use ba2::tes4::{Archive, ArchiveKey, ArchiveOptions, ArchiveFlags, Directory, DirectoryKey, File as BsaFile, Version};
use ba2::{ByteSlice, CompressableFrom, Reader};
use std::path::{Path, PathBuf};
use std::fs;
use std::io::BufWriter;
use tracing::info;

/// Directory data: (dir_path, [(filename, data)])
type DirData = Vec<(String, Vec<(String, Vec<u8>)>)>;

/// Supported games for BSA decompression
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompressGame {
    Fallout3,
    FalloutNV,
    Oblivion,
}

impl DecompressGame {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fo3" | "fallout3" | "fallout 3" => Some(Self::Fallout3),
            "fnv" | "falloutnv" | "fallout nv" | "new vegas" => Some(Self::FalloutNV),
            "oblivion" | "tes4" => Some(Self::Oblivion),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Fallout3 => "Fallout 3",
            Self::FalloutNV => "Fallout New Vegas",
            Self::Oblivion => "Oblivion",
        }
    }

    /// Get BSA file patterns for this game
    pub fn bsa_patterns(&self) -> Vec<&'static str> {
        match self {
            Self::Fallout3 => vec![
                "Fallout - Meshes.bsa",
                "Fallout - Misc.bsa",
                "Fallout - Textures.bsa",
            ],
            Self::FalloutNV => vec![
                "Fallout - Meshes.bsa",
                "Fallout - Misc.bsa",
                "Fallout - Sound.bsa",
                "Fallout - Textures.bsa",
                "Fallout - Textures2.bsa",
                // DLC BSAs
                "DeadMoney - Sounds.bsa",
                "HonestHearts - Sounds.bsa",
                "LonesomeRoad - Sounds.bsa",
                "OldWorldBlues - Sounds.bsa",
            ],
            Self::Oblivion => vec![
                "Oblivion - Meshes.bsa",
                "Oblivion - Misc.bsa",
                "Oblivion - Textures - Compressed.bsa",
                // DLC BSAs
                "DLCShiveringIsles - Meshes.bsa",
                "DLCShiveringIsles - Textures.bsa",
            ],
        }
    }

    /// Get BSA version for this game
    pub fn bsa_version(&self) -> Version {
        match self {
            Self::Fallout3 | Self::FalloutNV => Version::v104,
            Self::Oblivion => Version::v103,
        }
    }
}

/// Result of decompression operation
#[derive(Debug, Default)]
pub struct DecompressResult {
    pub bsas_processed: usize,
    pub files_extracted: usize,
    pub files_converted: usize,  // OGG -> WAV conversions (FNV only)
    pub errors: Vec<String>,
}

impl DecompressResult {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

/// BSA Decompressor service
pub struct BsaDecompressor {
    game: DecompressGame,
    data_path: PathBuf,
    output_path: Option<PathBuf>,
    create_backup: bool,
}

impl BsaDecompressor {
    pub fn new(game: DecompressGame, data_path: PathBuf) -> Self {
        Self {
            game,
            data_path,
            output_path: None,
            create_backup: true,
        }
    }

    /// Set output path (if different from input)
    pub fn with_output(mut self, output: PathBuf) -> Self {
        self.output_path = Some(output);
        self
    }

    /// Set whether to create backups of original BSAs
    pub fn with_backup(mut self, backup: bool) -> Self {
        self.create_backup = backup;
        self
    }

    /// Get list of BSA files to process
    pub fn find_bsas(&self) -> Result<Vec<PathBuf>> {
        let mut found = Vec::new();

        for pattern in self.game.bsa_patterns() {
            let bsa_path = self.data_path.join(pattern);
            if bsa_path.exists() {
                found.push(bsa_path);
            } else {
                // Try case-insensitive search
                if let Some(actual_path) = find_file_case_insensitive(&self.data_path, pattern) {
                    found.push(actual_path);
                }
            }
        }

        Ok(found)
    }

    /// Run decompression
    pub fn decompress(&self) -> Result<DecompressResult> {
        self.decompress_with_callback(|_, _, _| {})
    }

    /// Run decompression with progress callback
    /// callback(current_bsa, total_bsas, message)
    pub fn decompress_with_callback<F>(&self, callback: F) -> Result<DecompressResult>
    where
        F: Fn(usize, usize, &str) + Send + Sync,
    {
        let mut result = DecompressResult::default();

        // Find BSA files
        let bsas = self.find_bsas()?;
        if bsas.is_empty() {
            bail!("No BSA files found for {} in {}", self.game.name(), self.data_path.display());
        }

        let total = bsas.len();
        callback(0, total, &format!("Found {} BSA files to process", total));

        // Determine output directory
        let output_dir = self.output_path.as_ref().unwrap_or(&self.data_path);
        fs::create_dir_all(output_dir)?;

        // Process each BSA
        for (i, bsa_path) in bsas.iter().enumerate() {
            let bsa_name = bsa_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown.bsa".to_string());

            callback(i + 1, total, &format!("Processing: {}", bsa_name));

            match self.decompress_single_bsa(bsa_path, output_dir) {
                Ok((files, converted)) => {
                    result.bsas_processed += 1;
                    result.files_extracted += files;
                    result.files_converted += converted;
                }
                Err(e) => {
                    result.errors.push(format!("{}: {}", bsa_name, e));
                }
            }
        }

        callback(total, total, "Decompression complete");
        Ok(result)
    }

    /// Decompress a single BSA file
    fn decompress_single_bsa(&self, bsa_path: &Path, output_dir: &Path) -> Result<(usize, usize)> {
        let bsa_name = bsa_path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.bsa".to_string());

        // Check if this is FNV Meshes.bsa (needs special handling to stay under 2GB)
        // Architecture files are extracted as loose files instead of going into the BSA
        let is_fnv_meshes = self.game == DecompressGame::FalloutNV
            && bsa_name.to_lowercase() == "fallout - meshes.bsa";

        // Read the source archive with its options
        let (archive, options) = Archive::read(bsa_path)
            .with_context(|| format!("Failed to read BSA: {}", bsa_path.display()))?;

        // Collect all files with their data, organized by directory
        let mut merged: std::collections::HashMap<String, Vec<(String, Vec<u8>)>> = std::collections::HashMap::new();
        let mut converted_count = 0usize;
        let mut file_count = 0usize;
        let mut loose_file_count = 0usize;

        for (dir_key, folder) in archive.iter() {
            let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_string();

            for (file_key, file) in folder.iter() {
                let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_string();
                let full_path = if dir_name.is_empty() || dir_name == "." {
                    file_name.clone()
                } else {
                    format!("{}/{}", dir_name.replace('\\', "/"), file_name)
                };

                // Extract file data (decompress if needed)
                let data = if file.is_compressed() {
                    file.decompress(&Default::default())?.as_bytes().to_vec()
                } else {
                    file.as_bytes().to_vec()
                };

                // For FNV Meshes.bsa: extract architecture files as loose files to stay under 2GB
                if is_fnv_meshes && should_extract_as_loose(&full_path) {
                    let loose_path = output_dir.join(full_path.replace('\\', "/"));
                    if let Some(parent) = loose_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&loose_path, &data)?;
                    loose_file_count += 1;
                    continue;
                }

                // OGG -> WAV conversion for FNV ambient/emitter sounds
                let (final_path, final_data) = if self.game == DecompressGame::FalloutNV && needs_wav_conversion(&full_path) {
                    match convert_ogg_to_wav(&data) {
                        Ok(wav_data) => {
                            // Change extension from .ogg to .wav
                            let wav_path = full_path.replace(".ogg", ".wav").replace(".OGG", ".wav");
                            converted_count += 1;
                            (wav_path, wav_data)
                        }
                        Err(_) => {
                            // Keep original if conversion fails
                            (full_path, data)
                        }
                    }
                } else {
                    (full_path, data)
                };

                // Split path into directory and filename
                let normalized = final_path.replace('/', "\\");
                let (dir_path, fname) = if let Some(idx) = normalized.rfind('\\') {
                    (normalized[..idx].to_string(), normalized[idx + 1..].to_string())
                } else {
                    (".".to_string(), normalized)
                };

                merged.entry(dir_path).or_default().push((fname, final_data));
                file_count += 1;
            }
        }

        // Log loose files extracted for FNV Meshes.bsa
        if is_fnv_meshes && loose_file_count > 0 {
            info!(
                "Extracted {} architecture files as loose files from {} to keep BSA under 2GB",
                loose_file_count, bsa_name
            );
        }

        // Create backup if requested and output is same as input
        let output_bsa = output_dir.join(&bsa_name);
        if self.create_backup && output_bsa == bsa_path {
            let backup_path = bsa_path.with_extension("bsa.backup");
            if !backup_path.exists() {
                fs::rename(bsa_path, &backup_path)
                    .with_context(|| format!("Failed to create backup: {}", backup_path.display()))?;
            }
        }

        // Preserve original archive flags but remove compression
        // This is critical - games expect specific flags in the BSA header
        let mut new_flags = options.flags();
        new_flags.remove(ArchiveFlags::COMPRESSED);

        // Build new options preserving original settings (version, types) but without compression
        let new_options = ArchiveOptions::builder()
            .version(options.version())
            .flags(new_flags)
            .types(options.types())
            .build();

        // Build and write the archive
        // We need to build it in a way that doesn't have lifetime issues
        // The ba2 crate requires the data to outlive the archive, so we build directories
        // one at a time and write immediately

        // First, build the complete archive structure with owned data
        let dirs_data: DirData = merged.into_iter().collect();

        // Build archive - files need to be kept alive while building
        let archive: Archive = dirs_data.iter().map(|(dir_path, files)| {
            let directory: Directory = files.iter().map(|(file_name, data)| {
                let file = BsaFile::from_decompressed(&data[..]);
                (DirectoryKey::from(file_name.as_bytes()), file)
            }).collect();
            (ArchiveKey::from(dir_path.as_bytes()), directory)
        }).collect();

        // Write the archive
        let file = fs::File::create(&output_bsa)
            .with_context(|| format!("Failed to create output BSA: {}", output_bsa.display()))?;
        let mut writer = BufWriter::new(file);

        archive.write(&mut writer, &new_options)
            .with_context(|| format!("Failed to write BSA: {}", output_bsa.display()))?;

        Ok((file_count, converted_count))
    }
}

/// Check if a file path needs OGG -> WAV conversion (FNV ambient/emitter sounds)
fn needs_wav_conversion(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".ogg") && (
        lower.contains("/fx/amb/") || lower.contains("\\fx\\amb\\") ||
        lower.contains("/fx/emt/") || lower.contains("\\fx\\emt\\")
    )
}

/// Check if a file should be extracted as a loose file instead of going into the BSA
///
/// For FNV Meshes.bsa, base game architecture files (`meshes\architecture\*`) are
/// extracted as loose files to keep the BSA under 2GB (game crashes with larger BSAs).
/// DLC architecture files (`meshes\dlc*\architecture\*`) stay in the BSA.
fn should_extract_as_loose(path: &str) -> bool {
    let lower = path.to_lowercase();
    let normalized = lower.replace('/', "\\");

    // Extract base game architecture as loose files: meshes\architecture\*
    // Keep DLC architecture in BSA: meshes\dlc*\architecture\*
    normalized.starts_with("meshes\\architecture\\")
}

/// Convert OGG audio to WAV format
fn convert_ogg_to_wav(ogg_data: &[u8]) -> Result<Vec<u8>> {
    use symphonia::core::audio::{AudioBufferRef, Signal};
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    use std::io::Cursor;

    // Create a cursor over the OGG data
    let cursor = Cursor::new(ogg_data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    // Probe the format
    let mut hint = Hint::new();
    hint.with_extension("ogg");

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Failed to probe OGG format")?;

    let mut format = probed.format;

    // Get the default track
    let track = format.default_track()
        .ok_or_else(|| anyhow::anyhow!("No audio track found"))?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u16;

    // Create decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create decoder")?;

    // Decode all samples
    let mut all_samples: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };

        // Convert samples based on the actual format
        let num_channels = decoded.spec().channels.count();
        let num_frames = decoded.frames();

        match decoded {
            AudioBufferRef::F32(buf) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        let sample_f32 = buf.chan(ch)[frame];
                        // Convert f32 [-1.0, 1.0] to i16
                        let sample_i16 = (sample_f32.clamp(-1.0, 1.0) * 32767.0) as i16;
                        all_samples.push(sample_i16);
                    }
                }
            }
            AudioBufferRef::S16(buf) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        all_samples.push(buf.chan(ch)[frame]);
                    }
                }
            }
            AudioBufferRef::S32(buf) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        // Convert i32 to i16
                        let sample_i16 = (buf.chan(ch)[frame] >> 16) as i16;
                        all_samples.push(sample_i16);
                    }
                }
            }
            AudioBufferRef::U8(buf) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        // Convert u8 [0, 255] to i16
                        let sample_i16 = ((buf.chan(ch)[frame] as i16) - 128) * 256;
                        all_samples.push(sample_i16);
                    }
                }
            }
            _ => {
                // For other formats, try using SampleBuffer
                use symphonia::core::audio::SampleBuffer;
                let spec = *decoded.spec();
                let mut sample_buf = SampleBuffer::<i16>::new(num_frames as u64, spec);
                sample_buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(sample_buf.samples());
            }
        }
    }

    // Write WAV using hound
    let mut wav_buffer = Vec::new();
    {
        let cursor = Cursor::new(&mut wav_buffer);
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::new(cursor, spec)?;
        for sample in all_samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
    }

    Ok(wav_buffer)
}

/// Find a file with case-insensitive matching
fn find_file_case_insensitive(dir: &Path, filename: &str) -> Option<PathBuf> {
    let target_lower = filename.to_lowercase();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_lowercase() == target_lower {
                return Some(entry.path());
            }
        }
    }

    None
}
