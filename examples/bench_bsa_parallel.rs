use std::time::Instant;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::Path;
use ba2::tes4::Archive;
use ba2::Reader;

fn extract_files_rayon(bsa_path: &Path, max_files: usize) -> (usize, usize, std::time::Duration) {
    use rayon::prelude::*;

    let start = Instant::now();
    let (archive, _): (Archive, _) = Archive::read(bsa_path).expect("Failed to open BSA");

    // Collect file references first
    let mut files: Vec<_> = Vec::new();
    for (_, folder) in archive.iter() {
        for (_, file) in folder.iter() {
            files.push(file);
            if files.len() >= max_files {
                break;
            }
        }
        if files.len() >= max_files {
            break;
        }
    }

    let total_bytes = AtomicUsize::new(0);
    let file_count = AtomicUsize::new(0);

    files.par_iter().for_each(|file| {
        let data = if file.is_compressed() {
            file.decompress(&Default::default())
                .map(|d| d.as_bytes().to_vec())
                .unwrap_or_default()
        } else {
            file.as_bytes().to_vec()
        };
        total_bytes.fetch_add(data.len(), Ordering::Relaxed);
        file_count.fetch_add(1, Ordering::Relaxed);
    });

    (file_count.load(Ordering::Relaxed), total_bytes.load(Ordering::Relaxed), start.elapsed())
}

fn main() {
    println!("\n=== BSA Decompression Benchmark (with zlib-ng) ===\n");

    let bsa_paths = [
        ("/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Textures.bsa", "FO3 Textures"),
        ("/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data/Fallout - Meshes.bsa", "FO3 Meshes"),
        ("/home/luke/.local/share/Steam/steamapps/common/Fallout New Vegas/Data/Fallout - Textures.bsa", "FNV Textures"),
        ("/home/luke/.local/share/Steam/steamapps/common/Oblivion/Data/Oblivion - Textures - Compressed.bsa", "Oblivion Textures"),
    ];

    println!("{:<20} {:>8} {:>12} {:>12} {:>12}", "BSA", "Files", "Data", "Time", "Speed");
    println!("{}", "=".repeat(70));

    for (bsa_path, name) in &bsa_paths {
        let path = Path::new(bsa_path);
        if !path.exists() {
            println!("{:<20} SKIPPED (not found)", name);
            continue;
        }

        // Warm up
        let _ = extract_files_rayon(path, 10);

        // Benchmark with 2000 files
        let (count, bytes, time) = extract_files_rayon(path, 2000);
        let speed = bytes as f64 / 1024.0 / 1024.0 / time.as_secs_f64();

        println!(
            "{:<20} {:>8} {:>10.1} MB {:>10.2?} {:>8.0} MB/s",
            name, count, bytes as f64 / 1024.0 / 1024.0, time, speed
        );
    }

    println!("\nNote: Run with default flate2 (miniz_oxide) and with zlib-ng to compare.");
}
