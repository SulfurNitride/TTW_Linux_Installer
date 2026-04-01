//! Audio Pipeline Micro-Benchmark
//!
//! Profiles each sub-step of OggEnc2 independently:
//! 1. Decode (Symphonia OGG Vorbis → PCM f32)
//! 2. Resample (Rubato SincFixedIn 44100→24000 Hz)
//! 3. Encode (vorbis_rs PCM f32 → OGG Vorbis)
//!
//! Uses real audio files from FO3 BSAs.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ba2::tes4::{Archive, FileCompressionOptions};
use ba2::{ByteSlice, Reader};
use rayon::prelude::*;

use ttw_installer::services::AudioProcessor;

fn main() {
    println!("\n{}", "=".repeat(80));
    println!("  OggEnc2 Pipeline Micro-Benchmark (decode → resample → encode)");
    println!("{}\n", "=".repeat(80));

    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    println!("  CPUs: {}", cpus);

    // Collect audio files from FO3 voice/sound BSAs
    let bsa_paths = [
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Voices.bsa",
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Sound.bsa",
    ];

    // Extract OGG files from BSAs
    println!("\n  Extracting OGG files from BSAs...");
    let mut ogg_files: Vec<Vec<u8>> = Vec::new();

    for bsa_path in &bsa_paths {
        let path = Path::new(bsa_path);
        if !path.exists() {
            println!("    SKIP: {}", bsa_path);
            continue;
        }

        let (archive, options): (Archive, _) = match Archive::read(path) {
            Ok(a) => a,
            Err(e) => { println!("    FAIL: {} - {}", bsa_path, e); continue; }
        };
        let comp_opts = FileCompressionOptions::from(&options);

        let mut count = 0;
        for (_, folder) in archive.iter() {
            for (file_key, file) in folder.iter() {
                let name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
                if !name.ends_with(".ogg") { continue; }

                let data = if file.is_decompressed() {
                    file.as_bytes().to_vec()
                } else {
                    match file.decompress(&comp_opts) {
                        Ok(d) => d.as_bytes().to_vec(),
                        Err(_) => continue,
                    }
                };

                ogg_files.push(data);
                count += 1;
                if count >= 2000 { break; } // Enough for a good benchmark
            }
            if count >= 2000 { break; }
        }
        println!("    {} → {} OGG files", path.file_name().unwrap().to_string_lossy(), count);
    }

    if ogg_files.is_empty() {
        println!("  No OGG files found!");
        return;
    }

    let total_input_bytes: usize = ogg_files.iter().map(|f| f.len()).sum();
    println!("\n  Total: {} files, {:.1} MB input data\n",
        ogg_files.len(), total_input_bytes as f64 / 1024.0 / 1024.0);

    // ─── Benchmark 1: Full pipeline (baseline) ──────────────────────────
    println!("  {:40} {:>7} {:>8} {:>8} {:>8}", "Test", "Time", "Files", "MB/s", "ms/file");
    println!("  {}", "─".repeat(75));

    let start = Instant::now();
    let processed = AtomicUsize::new(0);
    let out_bytes = AtomicUsize::new(0);

    ogg_files.par_iter().for_each(|data| {
        let ap = AudioProcessor::new().with_params("-f:24000 -q:5");
        if let Ok(result) = ap.process_ogg_resample(data) {
            processed.fetch_add(1, Ordering::Relaxed);
            out_bytes.fetch_add(result.len(), Ordering::Relaxed);
        }
    });

    let full_elapsed = start.elapsed();
    let full_count = processed.load(Ordering::Relaxed);
    print_result("Full pipeline (decode+resample+encode)", full_elapsed, full_count, total_input_bytes);

    // ─── Benchmark 2: Decode only ───────────────────────────────────────

    let decode_ns = AtomicU64::new(0);
    let decode_count = AtomicUsize::new(0);
    let decode_samples = AtomicUsize::new(0);

    let start = Instant::now();
    ogg_files.par_iter().for_each(|data| {
        let ap = AudioProcessor::new();
        let t = Instant::now();
        if let Ok(decoded) = ap.decode_bytes(data, Some("ogg")) {
            decode_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            decode_count.fetch_add(1, Ordering::Relaxed);
            decode_samples.fetch_add(decoded.samples.len(), Ordering::Relaxed);
        }
    });
    let decode_elapsed = start.elapsed();
    print_result("Decode only (OGG → PCM)", decode_elapsed, decode_count.load(Ordering::Relaxed), total_input_bytes);

    let decode_thread_time = decode_ns.load(Ordering::Relaxed) as f64 / 1e9;
    println!("    Thread-time: {:.1}s, Avg: {:.2}ms/file",
        decode_thread_time, decode_thread_time * 1000.0 / decode_count.load(Ordering::Relaxed) as f64);

    // ─── Benchmark 3: Resample only (pre-decoded data) ──────────────────

    // Pre-decode all files first
    println!("\n    Pre-decoding {} files for resample/encode benchmarks...", ogg_files.len());
    let decoded_files: Vec<_> = ogg_files.par_iter().filter_map(|data| {
        let ap = AudioProcessor::new();
        ap.decode_bytes(data, Some("ogg")).ok()
    }).collect();
    println!("    Decoded {} files\n", decoded_files.len());

    let total_samples: usize = decoded_files.iter().map(|d| d.samples.len()).sum();
    let total_pcm_bytes = total_samples * 4; // f32 = 4 bytes

    let resample_ns = AtomicU64::new(0);
    let resample_count = AtomicUsize::new(0);

    let start = Instant::now();
    decoded_files.par_iter().for_each(|decoded| {
        let ap = AudioProcessor::new().with_params("-f:24000 -q:5");
        let t = Instant::now();
        if let Ok(_resampled) = ap.resample(decoded.clone()) {
            resample_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            resample_count.fetch_add(1, Ordering::Relaxed);
        }
    });
    let resample_elapsed = start.elapsed();
    print_result("Resample only (44100→24000 Hz)", resample_elapsed, resample_count.load(Ordering::Relaxed), total_pcm_bytes);

    let resample_thread_time = resample_ns.load(Ordering::Relaxed) as f64 / 1e9;
    println!("    Thread-time: {:.1}s, Avg: {:.2}ms/file",
        resample_thread_time, resample_thread_time * 1000.0 / resample_count.load(Ordering::Relaxed) as f64);

    // ─── Benchmark 4: Encode only (pre-resampled data) ──────────────────

    // Pre-resample all files
    println!("\n    Pre-resampling {} files for encode benchmark...", decoded_files.len());
    let resampled_files: Vec<_> = decoded_files.par_iter().filter_map(|decoded| {
        let ap = AudioProcessor::new().with_params("-f:24000 -q:5");
        ap.resample(decoded.clone()).ok()
    }).collect();
    println!("    Resampled {} files\n", resampled_files.len());

    let total_resampled_bytes: usize = resampled_files.iter().map(|d| d.samples.len() * 4).sum();

    let encode_ns = AtomicU64::new(0);
    let encode_count = AtomicUsize::new(0);
    let encode_out_bytes = AtomicUsize::new(0);

    let start = Instant::now();
    resampled_files.par_iter().for_each(|resampled| {
        let ap = AudioProcessor::new().with_params("-f:24000 -q:5");
        let t = Instant::now();
        if let Ok(encoded) = ap.encode_ogg(resampled) {
            encode_ns.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            encode_count.fetch_add(1, Ordering::Relaxed);
            encode_out_bytes.fetch_add(encoded.len(), Ordering::Relaxed);
        }
    });
    let encode_elapsed = start.elapsed();
    print_result("Encode only (PCM → OGG)", encode_elapsed, encode_count.load(Ordering::Relaxed), total_resampled_bytes);

    let encode_thread_time = encode_ns.load(Ordering::Relaxed) as f64 / 1e9;
    println!("    Thread-time: {:.1}s, Avg: {:.2}ms/file",
        encode_thread_time, encode_thread_time * 1000.0 / encode_count.load(Ordering::Relaxed) as f64);

    // ─── Benchmark 5: Resample with lower quality settings ──────────────

    println!("\n  {:40} {:>7} {:>8} {:>8} {:>8}", "Resample Quality Comparison", "Time", "Files", "MB/s", "ms/file");
    println!("  {}", "─".repeat(75));

    // Current: sinc_len=256, oversampling=256
    // Test with lower settings
    for (label, sinc_len, oversample) in [
        ("Current (sinc=256, over=256)", 256, 256),
        ("Medium  (sinc=128, over=128)", 128, 128),
        ("Fast    (sinc=64,  over=64)",   64,  64),
        ("Faster  (sinc=32,  over=32)",   32,  32),
    ] {
        let count = AtomicUsize::new(0);
        let start = Instant::now();

        decoded_files.par_iter().for_each(|decoded| {
            use rubato::{Resampler, SincFixedIn, SincInterpolationType, SincInterpolationParameters, WindowFunction};

            let channels = decoded.channels;
            let ratio = 24000.0 / decoded.sample_rate as f64;
            let samples_per_channel = decoded.samples.len() / channels;

            let mut channel_data: Vec<Vec<f32>> = vec![Vec::with_capacity(samples_per_channel); channels];
            for (i, sample) in decoded.samples.iter().enumerate() {
                channel_data[i % channels].push(*sample);
            }

            let params = SincInterpolationParameters {
                sinc_len: sinc_len,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: oversample,
                window: WindowFunction::BlackmanHarris2,
            };

            if let Ok(mut resampler) = SincFixedIn::<f32>::new(ratio, 2.0, params, samples_per_channel, channels) {
                if resampler.process(&channel_data, None).is_ok() {
                    count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let elapsed = start.elapsed();
        print_result(label, elapsed, count.load(Ordering::Relaxed), total_pcm_bytes);
    }

    // ─── Summary ────────────────────────────────────────────────────────

    println!("\n{}", "─".repeat(80));
    println!("  SUMMARY - Time breakdown for full pipeline ({} files)", full_count);
    println!("{}", "─".repeat(80));

    let total_sub = decode_elapsed + resample_elapsed + encode_elapsed;
    println!("\n  {:<25} {:>6.1}s  {:>5.1}%   {}",
        "Decode (OGG→PCM)", decode_elapsed.as_secs_f64(),
        decode_elapsed.as_secs_f64() / total_sub.as_secs_f64() * 100.0,
        bar(decode_elapsed.as_secs_f64() / total_sub.as_secs_f64()));
    println!("  {:<25} {:>6.1}s  {:>5.1}%   {}",
        "Resample (44.1k→24k)", resample_elapsed.as_secs_f64(),
        resample_elapsed.as_secs_f64() / total_sub.as_secs_f64() * 100.0,
        bar(resample_elapsed.as_secs_f64() / total_sub.as_secs_f64()));
    println!("  {:<25} {:>6.1}s  {:>5.1}%   {}",
        "Encode (PCM→OGG)", encode_elapsed.as_secs_f64(),
        encode_elapsed.as_secs_f64() / total_sub.as_secs_f64() * 100.0,
        bar(encode_elapsed.as_secs_f64() / total_sub.as_secs_f64()));
    println!("  {:<25} {:>6.1}s", "Sum of parts", total_sub.as_secs_f64());
    println!("  {:<25} {:>6.1}s", "Full pipeline", full_elapsed.as_secs_f64());
}

fn print_result(label: &str, elapsed: Duration, count: usize, bytes: usize) {
    let secs = elapsed.as_secs_f64();
    let mb = bytes as f64 / 1024.0 / 1024.0;
    let throughput = if secs > 0.0 { mb / secs } else { 0.0 };
    let per_file_ms = if count > 0 { secs * 1000.0 / count as f64 } else { 0.0 };
    println!("  {:<40} {:>6.1}s  {:>7}  {:>6.0} MB/s  {:>5.2}ms",
        label, secs, count, throughput, per_file_ms);
}

fn bar(fraction: f64) -> String {
    "█".repeat((fraction * 50.0) as usize)
}
