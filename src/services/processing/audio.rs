use std::fs::File;
use std::io::Write;
use std::num::NonZero;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use mp3lame_encoder::{Builder, DualPcm, FlushNoGap, InterleavedPcm};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_VORBIS};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

/// Audio processing service for decoding, resampling, and encoding audio files
pub struct AudioProcessor {
    /// Target sample rate for resampling (default 44100 Hz)
    target_sample_rate: u32,
    /// OGG encoding quality (-0.1 to 1.0, default 0.5)
    ogg_quality: f32,
}

impl AudioProcessor {
    pub fn new() -> Self {
        Self {
            target_sample_rate: 44100,
            ogg_quality: 0.5,
        }
    }

    pub fn with_target_sample_rate(mut self, rate: u32) -> Self {
        self.target_sample_rate = rate;
        self
    }

    pub fn with_ogg_quality(mut self, quality: f32) -> Self {
        self.ogg_quality = quality.clamp(-0.1, 1.0);
        self
    }

    /// Configure from MPI manifest params string (e.g. "-f:24000 -q:5")
    /// -f:<hz> sets target sample rate, -q:<0-10> sets OGG quality (mapped to -0.1..1.0)
    pub fn with_params(mut self, params: &str) -> Self {
        for token in params.split_whitespace() {
            if let Some(freq_str) = token.strip_prefix("-f:") {
                if let Ok(freq) = freq_str.parse::<u32>() {
                    if freq > 0 {
                        self.target_sample_rate = freq;
                    }
                }
            } else if let Some(q_str) = token.strip_prefix("-q:") {
                if let Ok(q) = q_str.parse::<f32>() {
                    // OggEnc2 quality scale: 0-10 maps to vorbis -0.1 to 1.0
                    self.ogg_quality = (q / 10.0).clamp(-0.1, 1.0);
                }
            }
        }
        self
    }

    /// Decode an audio file to raw PCM samples
    /// Returns (samples, channels, sample_rate)
    pub fn decode_file(&self, path: &Path) -> Result<DecodedAudio> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open audio file: {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio format")?;

        let mut format = probed.format;
        let track = format.default_track().context("No audio tracks found")?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate.context("Unknown sample rate")?;
        let channels = codec_params
            .channels
            .context("Unknown channel count")?
            .count();

        let decoder_opts = DecoderOptions::default();
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &decoder_opts)
            .context("Failed to create decoder")?;

        let mut all_samples: Vec<f32> = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(e) => return Err(e.into()),
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = decoder.decode(&packet)?;
            Self::append_samples(&decoded, &mut all_samples);
        }

        Ok(DecodedAudio {
            samples: all_samples,
            channels,
            sample_rate,
        })
    }

    /// Decode audio from raw bytes (in-memory)
    pub fn decode_bytes(&self, data: &[u8], format_hint: Option<&str>) -> Result<DecodedAudio> {
        let cursor = std::io::Cursor::new(data.to_vec());
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = format_hint {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio format")?;

        let mut format = probed.format;
        let track = format.default_track().context("No audio tracks found")?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate.context("Unknown sample rate")?;
        let channels = codec_params
            .channels
            .context("Unknown channel count")?
            .count();

        let decoder_opts = DecoderOptions::default();
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &decoder_opts)
            .context("Failed to create decoder")?;

        let mut all_samples: Vec<f32> = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(e) => return Err(e.into()),
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = decoder.decode(&packet)?;
            Self::append_samples(&decoded, &mut all_samples);
        }

        Ok(DecodedAudio {
            samples: all_samples,
            channels,
            sample_rate,
        })
    }

    fn probe_audio_info(data: &[u8], format_hint: Option<&str>) -> Result<AudioInfo> {
        let cursor = std::io::Cursor::new(data.to_vec());
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = format_hint {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .context("Failed to probe audio format")?;

        let track = probed
            .format
            .default_track()
            .context("No audio tracks found")?;
        let codec_params = &track.codec_params;

        Ok(AudioInfo {
            codec_is_vorbis: codec_params.codec == CODEC_TYPE_VORBIS,
            sample_rate: codec_params.sample_rate,
            channels: codec_params.channels.map(|channels| channels.count()),
        })
    }

    /// Append samples from a Symphonia audio buffer in interleaved format.
    /// Symphonia returns planar data (one plane per channel), so we must
    /// interleave them: [L1, R1, L2, R2, ...] not [L1..Ln, R1..Rn].
    fn append_samples(buffer: &AudioBufferRef, output: &mut Vec<f32>) {
        match buffer {
            AudioBufferRef::F32(buf) => {
                let signal = buf.planes();
                let planes = signal.planes();
                if planes.len() <= 1 {
                    for plane in planes {
                        output.extend_from_slice(plane);
                    }
                } else {
                    let num_frames = planes[0].len();
                    output.reserve(num_frames * planes.len());
                    for i in 0..num_frames {
                        for plane in planes {
                            output.push(plane[i]);
                        }
                    }
                }
            }
            AudioBufferRef::S16(buf) => {
                let signal = buf.planes();
                let planes = signal.planes();
                if planes.len() <= 1 {
                    for plane in planes {
                        output.extend(plane.iter().map(|&s| s as f32 / 32768.0));
                    }
                } else {
                    let num_frames = planes[0].len();
                    output.reserve(num_frames * planes.len());
                    for i in 0..num_frames {
                        for plane in planes {
                            output.push(plane[i] as f32 / 32768.0);
                        }
                    }
                }
            }
            AudioBufferRef::S32(buf) => {
                let signal = buf.planes();
                let planes = signal.planes();
                if planes.len() <= 1 {
                    for plane in planes {
                        output.extend(plane.iter().map(|&s| s as f32 / 2147483648.0));
                    }
                } else {
                    let num_frames = planes[0].len();
                    output.reserve(num_frames * planes.len());
                    for i in 0..num_frames {
                        for plane in planes {
                            output.push(plane[i] as f32 / 2147483648.0);
                        }
                    }
                }
            }
            AudioBufferRef::U8(buf) => {
                let signal = buf.planes();
                let planes = signal.planes();
                if planes.len() <= 1 {
                    for plane in planes {
                        output.extend(plane.iter().map(|&s| (s as f32 - 128.0) / 128.0));
                    }
                } else {
                    let num_frames = planes[0].len();
                    output.reserve(num_frames * planes.len());
                    for i in 0..num_frames {
                        for plane in planes {
                            output.push((plane[i] as f32 - 128.0) / 128.0);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Resample audio to target sample rate
    /// Uses settings matching oggenc2's SRC_SINC_FASTEST (libsamplerate)
    pub fn resample(&self, audio: DecodedAudio) -> Result<DecodedAudio> {
        self.resample_to_rate(&audio, self.target_sample_rate)
    }

    fn resample_to_rate(
        &self,
        audio: &DecodedAudio,
        target_sample_rate: u32,
    ) -> Result<DecodedAudio> {
        if audio.sample_rate == target_sample_rate {
            return Ok(audio.clone());
        }

        let channels = audio.channels;
        if channels == 0 {
            bail!("Cannot resample audio with zero channels");
        }
        let ratio = target_sample_rate as f64 / audio.sample_rate as f64;

        // Deinterleave samples into per-channel vectors
        let samples_per_channel = audio.samples.len() / channels;
        let mut channel_data: Vec<Vec<f32>> =
            vec![Vec::with_capacity(samples_per_channel); channels];

        for (i, sample) in audio.samples.iter().enumerate() {
            channel_data[i % channels].push(*sample);
        }

        let resampled =
            Self::resample_channels(channel_data, samples_per_channel, channels, ratio)?;

        // Interleave samples back together
        let output_len = resampled[0].len();
        let mut output = Vec::with_capacity(output_len * channels);
        for i in 0..output_len {
            for channel_samples in resampled.iter().take(channels) {
                output.push(channel_samples[i]);
            }
        }

        Ok(DecodedAudio {
            samples: output,
            channels,
            sample_rate: target_sample_rate,
        })
    }

    /// Resample per-channel data without interleaving the output.
    /// Returns Vec<Vec<f32>> (one vec per channel) - avoids redundant
    /// interleave+deinterleave when the next step (OGG encode) also wants per-channel data.
    ///
    /// Resampler params match oggenc2's default: libsamplerate SRC_SINC_FASTEST
    fn resample_channels(
        channel_data: Vec<Vec<f32>>,
        samples_per_channel: usize,
        channels: usize,
        ratio: f64,
    ) -> Result<Vec<Vec<f32>>> {
        // Match oggenc2's SRC_SINC_FASTEST quality
        let params = SincInterpolationParameters {
            sinc_len: 64,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 64,
            window: WindowFunction::BlackmanHarris2,
        };

        let mut resampler =
            SincFixedIn::<f32>::new(ratio, 2.0, params, samples_per_channel, channels)
                .context("Failed to create resampler")?;

        resampler
            .process(&channel_data, None)
            .context("Failed to resample audio")
    }

    /// Encode audio to OGG Vorbis format
    pub fn encode_ogg(&self, audio: &DecodedAudio) -> Result<Vec<u8>> {
        // Deinterleave into per-channel data for the encoder
        let samples_per_channel = audio.samples.len() / audio.channels;
        let mut channel_samples: Vec<Vec<f32>> =
            vec![Vec::with_capacity(samples_per_channel); audio.channels];

        for (i, &sample) in audio.samples.iter().enumerate() {
            channel_samples[i % audio.channels].push(sample.clamp(-1.0, 1.0));
        }

        Self::encode_ogg_channels(&channel_samples, audio.sample_rate, self.ogg_quality)
    }

    /// Encode per-channel data directly to OGG without interleaving first.
    /// Used by the optimized pipeline to skip the interleave -> deinterleave roundtrip.
    fn encode_ogg_channels(
        channel_data: &[Vec<f32>],
        sample_rate: u32,
        quality: f32,
    ) -> Result<Vec<u8>> {
        let channels = channel_data.len();
        let mut output = Vec::new();

        let sr = NonZero::new(sample_rate)
            .ok_or_else(|| anyhow::anyhow!("Sample rate cannot be zero"))?;
        let ch = NonZero::new(channels as u8)
            .ok_or_else(|| anyhow::anyhow!("Channel count cannot be zero"))?;

        let mut encoder = VorbisEncoderBuilder::new(sr, ch, &mut output)
            .map_err(|e| anyhow::anyhow!("Failed to create Vorbis encoder: {:?}", e))?;

        encoder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: quality,
        });

        let mut encoder = encoder
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build Vorbis encoder: {:?}", e))?;

        let clamped: Vec<Vec<f32>> = channel_data
            .iter()
            .map(|ch| ch.iter().map(|s| s.clamp(-1.0, 1.0)).collect())
            .collect();
        let channel_slices: Vec<&[f32]> = clamped.iter().map(|v| v.as_slice()).collect();

        encoder
            .encode_audio_block(&channel_slices)
            .map_err(|e| anyhow::anyhow!("Failed to encode audio: {:?}", e))?;

        encoder
            .finish()
            .map_err(|e| anyhow::anyhow!("Failed to finish encoding: {:?}", e))?;

        Ok(output)
    }

    /// Encode audio to WAV format
    pub fn encode_wav(&self, audio: &DecodedAudio) -> Result<Vec<u8>> {
        let spec = WavSpec {
            channels: audio.channels as u16,
            sample_rate: audio.sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        let mut buffer = Vec::new();
        let cursor = std::io::Cursor::new(&mut buffer);
        let mut writer = WavWriter::new(cursor, spec).context("Failed to create WAV writer")?;

        for &sample in &audio.samples {
            let sample_i16 = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            writer
                .write_sample(sample_i16)
                .context("Failed to write sample")?;
        }

        writer.finalize().context("Failed to finalize WAV")?;

        Ok(buffer)
    }

    /// Process OggEnc2 operation: decode OGG, resample, re-encode to OGG.
    ///
    /// If the source is already Ogg Vorbis at the manifest target sample rate,
    /// copy it through unchanged. Vorbis files do not expose the original
    /// encoder quality setting reliably, so quality cannot be safely compared.
    pub fn process_ogg_resample(&self, input_data: &[u8]) -> Result<Vec<u8>> {
        if self.can_copy_ogg_resample_input(input_data)? {
            return Ok(input_data.to_vec());
        }

        if Self::use_ffmpeg_audio_backend() {
            return self.process_ogg_resample_ffmpeg(input_data);
        }

        let decoded = self
            .decode_bytes(input_data, Some("ogg"))
            .context("Failed to decode OGG")?;

        let channels = decoded.channels;
        let samples_per_channel = decoded.samples.len() / channels;

        if decoded.sample_rate == self.target_sample_rate {
            return self.encode_ogg(&decoded);
        }

        // Deinterleave once (decode produces interleaved samples).
        let mut channel_data: Vec<Vec<f32>> =
            vec![Vec::with_capacity(samples_per_channel); channels];
        for (i, &sample) in decoded.samples.iter().enumerate() {
            channel_data[i % channels].push(sample);
        }

        let ratio = self.target_sample_rate as f64 / decoded.sample_rate as f64;
        let resampled = Self::resample_channels(channel_data, samples_per_channel, channels, ratio)
            .context("Failed to resample")?;

        Self::encode_ogg_channels(&resampled, self.target_sample_rate, self.ogg_quality)
            .context("Failed to encode OGG")
    }

    fn can_copy_ogg_resample_input(&self, input_data: &[u8]) -> Result<bool> {
        let info = Self::probe_audio_info(input_data, Some("ogg"))?;

        Ok(info.codec_is_vorbis
            && info.sample_rate == Some(self.target_sample_rate)
            && info.channels.is_some_and(|channels| channels > 0))
    }

    fn use_ffmpeg_audio_backend() -> bool {
        std::env::var("TTW_AUDIO_BACKEND")
            .is_ok_and(|backend| backend.eq_ignore_ascii_case("ffmpeg"))
    }

    fn process_ogg_resample_ffmpeg(&self, input_data: &[u8]) -> Result<Vec<u8>> {
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "ogg",
                "-i",
                "pipe:0",
                "-map",
                "0:a:0",
                "-vn",
                "-ar",
                &self.target_sample_rate.to_string(),
                "-c:a",
                "libvorbis",
                "-q:a",
                &self.ffmpeg_vorbis_quality().to_string(),
                "-f",
                "ogg",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start ffmpeg for OggEnc2")?;

        {
            let stdin = child
                .stdin
                .as_mut()
                .context("Failed to open ffmpeg stdin")?;
            stdin
                .write_all(input_data)
                .context("Failed to write input audio to ffmpeg")?;
        }

        let output = child
            .wait_with_output()
            .context("Failed to wait for ffmpeg")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("ffmpeg OggEnc2 failed: {}", stderr.trim());
        }

        Ok(output.stdout)
    }

    fn ffmpeg_vorbis_quality(&self) -> String {
        let quality = (self.ogg_quality * 10.0).clamp(-1.0, 10.0);
        format!("{quality:.2}")
    }

    /// Process AudioEnc operation: convert to specified format
    pub fn process_audio_conversion(
        &self,
        input_data: &[u8],
        input_format: Option<&str>,
        output_format: AudioFormat,
    ) -> Result<Vec<u8>> {
        let decoded = self
            .decode_bytes(input_data, input_format)
            .context("Failed to decode audio")?;

        match output_format {
            AudioFormat::Ogg => {
                let resampled = self.resample(decoded).context("Failed to resample")?;
                self.encode_ogg(&resampled)
            }
            AudioFormat::Wav => {
                let resampled = self.resample(decoded).context("Failed to resample")?;
                self.encode_wav(&resampled)
            }
            AudioFormat::Mp3 => {
                let resampled = self.resample(decoded).context("Failed to resample")?;
                self.encode_mp3(&resampled)
            }
        }
    }

    /// Encode audio to MP3 using LAME
    fn encode_mp3(&self, audio: &DecodedAudio) -> Result<Vec<u8>> {
        use std::mem::MaybeUninit;

        let mut builder =
            Builder::new().ok_or_else(|| anyhow::anyhow!("Failed to create LAME encoder"))?;

        builder
            .set_num_channels(audio.channels as u8)
            .map_err(|e| anyhow::anyhow!("Failed to set channels: {:?}", e))?;
        builder
            .set_sample_rate(audio.sample_rate)
            .map_err(|e| anyhow::anyhow!("Failed to set sample rate: {:?}", e))?;
        builder
            .set_quality(mp3lame_encoder::Quality::Best)
            .map_err(|e| anyhow::anyhow!("Failed to set quality: {:?}", e))?;
        builder
            .set_brate(mp3lame_encoder::Bitrate::Kbps192)
            .map_err(|e| anyhow::anyhow!("Failed to set bitrate: {:?}", e))?;

        let mut encoder = builder
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build LAME encoder: {:?}", e))?;

        // Convert f32 samples to i16
        let samples_i16: Vec<i16> = audio
            .samples
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        let mut mp3_data = Vec::new();

        if audio.channels == 1 {
            // Mono - use interleaved with 1 channel
            let input = InterleavedPcm(&samples_i16);
            let mut output: Vec<MaybeUninit<u8>> =
                vec![
                    MaybeUninit::uninit();
                    mp3lame_encoder::max_required_buffer_size(samples_i16.len())
                ];
            let encoded = encoder
                .encode(input, &mut output)
                .map_err(|e| anyhow::anyhow!("Failed to encode MP3: {:?}", e))?;
            // SAFETY: encode() initializes the first `encoded` bytes
            mp3_data.extend(output[..encoded].iter().map(|b| unsafe { b.assume_init() }));
        } else if audio.channels == 2 {
            // Stereo - deinterleave into dual PCM
            let frame_count = samples_i16.len() / 2;
            let mut left = Vec::with_capacity(frame_count);
            let mut right = Vec::with_capacity(frame_count);

            for chunk in samples_i16.chunks(2) {
                left.push(chunk[0]);
                right.push(chunk.get(1).copied().unwrap_or(chunk[0]));
            }

            let input = DualPcm {
                left: &left,
                right: &right,
            };
            let mut output: Vec<MaybeUninit<u8>> =
                vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(frame_count)];
            let encoded = encoder
                .encode(input, &mut output)
                .map_err(|e| anyhow::anyhow!("Failed to encode stereo MP3: {:?}", e))?;
            // SAFETY: encode() initializes the first `encoded` bytes
            mp3_data.extend(output[..encoded].iter().map(|b| unsafe { b.assume_init() }));
        } else {
            bail!("MP3 encoding only supports mono or stereo audio");
        }

        // Flush encoder
        let mut output: Vec<MaybeUninit<u8>> =
            vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(1024)];
        let flushed = encoder
            .flush::<FlushNoGap>(&mut output)
            .map_err(|e| anyhow::anyhow!("Failed to flush MP3 encoder: {:?}", e))?;
        // SAFETY: flush() initializes the first `flushed` bytes
        mp3_data.extend(output[..flushed].iter().map(|b| unsafe { b.assume_init() }));

        Ok(mp3_data)
    }
}

impl Default for AudioProcessor {
    fn default() -> Self {
        Self::new()
    }
}

struct AudioInfo {
    codec_is_vorbis: bool,
    sample_rate: Option<u32>,
    channels: Option<usize>,
}

/// Decoded audio data
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Interleaved PCM samples as f32 (-1.0 to 1.0)
    pub samples: Vec<f32>,
    /// Number of channels
    pub channels: usize,
    /// Sample rate in Hz
    pub sample_rate: u32,
}

/// Output audio formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Ogg,
    Wav,
    Mp3,
}

impl AudioFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "ogg" => Some(Self::Ogg),
            "wav" => Some(Self::Wav),
            "mp3" => Some(Self::Mp3),
            _ => None,
        }
    }
}
