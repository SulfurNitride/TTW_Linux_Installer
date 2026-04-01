use ba2::tes4::{Archive, FileCompressionOptions};
use ba2::{ByteSlice, Reader};
use std::collections::HashMap;
use std::path::Path;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn check_bsa(bsa_path: &str, label: &str, max_files: usize) {
    let path = Path::new(bsa_path);
    if !path.exists() { println!("{}: NOT FOUND", label); return; }

    let (archive, options): (Archive, _) = Archive::read(path).unwrap();
    let comp_opts = FileCompressionOptions::from(&options);

    let mut rates: HashMap<u32, usize> = HashMap::new();
    let mut channels_map: HashMap<usize, usize> = HashMap::new();
    let mut total_ogg = 0usize;
    let mut total_wav = 0usize;
    let mut checked = 0;

    for (_, folder) in archive.iter() {
        for (file_key, file) in folder.iter() {
            let name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
            if name.ends_with(".ogg") { total_ogg += 1; }
            else if name.ends_with(".wav") { total_wav += 1; }
            else { continue; }

            if checked >= max_files { continue; } // Still count totals

            let data = if file.is_decompressed() {
                file.as_bytes().to_vec()
            } else {
                match file.decompress(&comp_opts) {
                    Ok(d) => d.as_bytes().to_vec(),
                    Err(_) => continue,
                }
            };

            let ext = if name.ends_with(".ogg") { "ogg" } else { "wav" };
            let cursor = std::io::Cursor::new(data);
            let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
            let mut hint = Hint::new();
            hint.with_extension(ext);

            if let Ok(probed) = symphonia::default::get_probe().format(
                &hint, mss, &FormatOptions::default(), &MetadataOptions::default()
            ) {
                if let Some(track) = probed.format.default_track() {
                    if let Some(rate) = track.codec_params.sample_rate {
                        *rates.entry(rate).or_insert(0) += 1;
                    }
                    if let Some(ch) = track.codec_params.channels {
                        *channels_map.entry(ch.count()).or_insert(0) += 1;
                    }
                }
            }
            checked += 1;
        }
    }

    println!("\n  {} ({} OGG, {} WAV total, sampled {}):", label, total_ogg, total_wav, checked);
    println!("    Sample rates:");
    let mut rate_vec: Vec<_> = rates.into_iter().collect();
    rate_vec.sort_by_key(|(r, _)| *r);
    for (rate, count) in &rate_vec {
        let pct = *count as f64 / checked as f64 * 100.0;
        println!("      {:>6} Hz: {:>5} files ({:.1}%)", rate, count, pct);
    }
    println!("    Channels:");
    for (ch, count) in &channels_map {
        println!("      {} ch: {} files", ch, count);
    }
}

fn main() {
    println!("\n  Audio File Sample Rate Survey");
    println!("  {}\n", "=".repeat(60));

    // FO3 BSAs that contain audio files targeted by OggEnc2
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Voices.bsa",
        "FO3 Voices", 1000
    );
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Sound.bsa",
        "FO3 Sound", 500
    );
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - MenuVoices.bsa",
        "FO3 MenuVoices", 1000
    );
    // DLC BSAs
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Anchorage - Sounds.bsa",
        "FO3 Anchorage Sounds", 200
    );
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/BrokenSteel - Sounds.bsa",
        "FO3 BrokenSteel Sounds", 200
    );
    // FNV for comparison
    check_bsa(
        "/home/luke/.local/share/Steam/steamapps/common/Fallout New Vegas/Data/Fallout - Voices1.bsa",
        "FNV Voices", 500
    );
}
