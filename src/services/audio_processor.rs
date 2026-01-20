use anyhow::{Result, Context, bail};
use std::path::Path;
use std::fs::File;
use std::num::NonZero;
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use rubato::{Resampler, SincFixedIn, SincInterpolationType, SincInterpolationParameters, WindowFunction};
use hound::{WavWriter, WavSpec, SampleFormat};
use vorbis_rs::VorbisEncoderBuilder;
use mp3lame_encoder::{Builder, FlushNoGap, InterleavedPcm, DualPcm};

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
        let track = format.default_track()
            .context("No audio tracks found")?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate
            .context("Unknown sample rate")?;
        let channels = codec_params.channels
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
                    if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
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
        let track = format.default_track()
            .context("No audio tracks found")?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate
            .context("Unknown sample rate")?;
        let channels = codec_params.channels
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
                    if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
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

    fn append_samples(buffer: &AudioBufferRef, output: &mut Vec<f32>) {
        match buffer {
            AudioBufferRef::F32(buf) => {
                for plane in buf.planes().planes() {
                    output.extend_from_slice(plane);
                }
            }
            AudioBufferRef::S16(buf) => {
                for plane in buf.planes().planes() {
                    output.extend(plane.iter().map(|&s| s as f32 / 32768.0));
                }
            }
            AudioBufferRef::S32(buf) => {
                for plane in buf.planes().planes() {
                    output.extend(plane.iter().map(|&s| s as f32 / 2147483648.0));
                }
            }
            AudioBufferRef::U8(buf) => {
                for plane in buf.planes().planes() {
                    output.extend(plane.iter().map(|&s| (s as f32 - 128.0) / 128.0));
                }
            }
            _ => {}
        }
    }

    /// Resample audio to target sample rate
    pub fn resample(&self, audio: DecodedAudio) -> Result<DecodedAudio> {
        if audio.sample_rate == self.target_sample_rate {
            return Ok(audio);
        }

        let channels = audio.channels;
        let ratio = self.target_sample_rate as f64 / audio.sample_rate as f64;

        // Deinterleave samples into per-channel vectors
        let samples_per_channel = audio.samples.len() / channels;
        let mut channel_data: Vec<Vec<f32>> = vec![Vec::with_capacity(samples_per_channel); channels];

        for (i, sample) in audio.samples.iter().enumerate() {
            channel_data[i % channels].push(*sample);
        }

        // Create resampler
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        let mut resampler = SincFixedIn::<f32>::new(
            ratio,
            2.0,
            params,
            samples_per_channel,
            channels,
        ).context("Failed to create resampler")?;

        let resampled = resampler.process(&channel_data, None)
            .context("Failed to resample audio")?;

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
            sample_rate: self.target_sample_rate,
        })
    }

    /// Encode audio to OGG Vorbis format
    pub fn encode_ogg(&self, audio: &DecodedAudio) -> Result<Vec<u8>> {
        let mut output = Vec::new();

        // Group f32 samples by channel (vorbis_rs expects f32)
        let samples_per_channel = audio.samples.len() / audio.channels;
        let mut channel_samples: Vec<Vec<f32>> = vec![Vec::with_capacity(samples_per_channel); audio.channels];

        for (i, &sample) in audio.samples.iter().enumerate() {
            channel_samples[i % audio.channels].push(sample.clamp(-1.0, 1.0));
        }

        // Create encoder with NonZero types
        let sample_rate = NonZero::new(audio.sample_rate)
            .ok_or_else(|| anyhow::anyhow!("Sample rate cannot be zero"))?;
        let channels = NonZero::new(audio.channels as u8)
            .ok_or_else(|| anyhow::anyhow!("Channel count cannot be zero"))?;

        let mut encoder = VorbisEncoderBuilder::new(
            sample_rate,
            channels,
            &mut output,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create Vorbis encoder: {:?}", e))?
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build Vorbis encoder: {:?}", e))?;

        // Convert to slices for the encoder
        let channel_slices: Vec<&[f32]> = channel_samples.iter()
            .map(|v| v.as_slice())
            .collect();

        // Encode
        encoder.encode_audio_block(&channel_slices)
            .map_err(|e| anyhow::anyhow!("Failed to encode audio: {:?}", e))?;

        encoder.finish()
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
        let mut writer = WavWriter::new(cursor, spec)
            .context("Failed to create WAV writer")?;

        for &sample in &audio.samples {
            let sample_i16 = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            writer.write_sample(sample_i16)
                .context("Failed to write sample")?;
        }

        writer.finalize().context("Failed to finalize WAV")?;

        Ok(buffer)
    }

    /// Process OggEnc2 operation: decode OGG, resample, re-encode to OGG
    pub fn process_ogg_resample(&self, input_data: &[u8]) -> Result<Vec<u8>> {
        let decoded = self.decode_bytes(input_data, Some("ogg"))
            .context("Failed to decode OGG")?;

        let resampled = self.resample(decoded)
            .context("Failed to resample")?;

        self.encode_ogg(&resampled)
            .context("Failed to encode OGG")
    }

    /// Process AudioEnc operation: convert to specified format
    pub fn process_audio_conversion(
        &self,
        input_data: &[u8],
        input_format: Option<&str>,
        output_format: AudioFormat,
    ) -> Result<Vec<u8>> {
        let decoded = self.decode_bytes(input_data, input_format)
            .context("Failed to decode audio")?;

        let resampled = self.resample(decoded)
            .context("Failed to resample")?;

        match output_format {
            AudioFormat::Ogg => self.encode_ogg(&resampled),
            AudioFormat::Wav => self.encode_wav(&resampled),
            AudioFormat::Mp3 => self.encode_mp3(&resampled),
        }
    }

    /// Encode audio to MP3 using LAME
    fn encode_mp3(&self, audio: &DecodedAudio) -> Result<Vec<u8>> {
        use std::mem::MaybeUninit;

        let mut builder = Builder::new()
            .ok_or_else(|| anyhow::anyhow!("Failed to create LAME encoder"))?;

        builder.set_num_channels(audio.channels as u8)
            .map_err(|e| anyhow::anyhow!("Failed to set channels: {:?}", e))?;
        builder.set_sample_rate(audio.sample_rate)
            .map_err(|e| anyhow::anyhow!("Failed to set sample rate: {:?}", e))?;
        builder.set_quality(mp3lame_encoder::Quality::Best)
            .map_err(|e| anyhow::anyhow!("Failed to set quality: {:?}", e))?;
        builder.set_brate(mp3lame_encoder::Bitrate::Kbps192)
            .map_err(|e| anyhow::anyhow!("Failed to set bitrate: {:?}", e))?;

        let mut encoder = builder.build()
            .map_err(|e| anyhow::anyhow!("Failed to build LAME encoder: {:?}", e))?;

        // Convert f32 samples to i16
        let samples_i16: Vec<i16> = audio.samples.iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        let mut mp3_data = Vec::new();

        if audio.channels == 1 {
            // Mono - use interleaved with 1 channel
            let input = InterleavedPcm(&samples_i16);
            let mut output: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(samples_i16.len())];
            let encoded = encoder.encode(input, &mut output)
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

            let input = DualPcm { left: &left, right: &right };
            let mut output: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(frame_count)];
            let encoded = encoder.encode(input, &mut output)
                .map_err(|e| anyhow::anyhow!("Failed to encode stereo MP3: {:?}", e))?;
            // SAFETY: encode() initializes the first `encoded` bytes
            mp3_data.extend(output[..encoded].iter().map(|b| unsafe { b.assume_init() }));
        } else {
            bail!("MP3 encoding only supports mono or stereo audio");
        }

        // Flush encoder
        let mut output: Vec<MaybeUninit<u8>> = vec![MaybeUninit::uninit(); mp3lame_encoder::max_required_buffer_size(1024)];
        let flushed = encoder.flush::<FlushNoGap>(&mut output)
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
