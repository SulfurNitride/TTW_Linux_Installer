use ba2::tes4::Archive;
use ba2::Reader;
use std::path::Path;

fn analyze_bsa(path: &Path) -> (usize, usize, usize) {
    let (archive, _): (Archive, _) = match Archive::read(path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to open {}: {}", path.display(), e);
            return (0, 0, 0);
        }
    };

    let mut file_count = 0;
    let mut compressed_size = 0;
    let mut estimated_decompressed = 0;

    for (_, folder) in archive.iter() {
        for (_, file) in folder.iter() {
            file_count += 1;
            let size = file.as_bytes().len();
            compressed_size += size;
            if file.is_compressed() {
                estimated_decompressed += size * 3; // ~3x compression ratio estimate
            } else {
                estimated_decompressed += size;
            }
        }
    }

    (file_count, compressed_size, estimated_decompressed)
}

fn main() {
    let bsa_dirs = [
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data",
        "/home/luke/.local/share/Steam/steamapps/common/Fallout New Vegas/Data",
        "/home/luke/.local/share/Steam/steamapps/common/Oblivion/Data",
    ];

    println!("\n=== BSA File Analysis ===\n");
    println!("{:<50} {:>8} {:>12} {:>12}", "BSA File", "Files", "On Disk", "Est. RAM");
    println!("{}", "-".repeat(85));

    let mut total_files = 0;
    let mut total_disk = 0;
    let mut total_ram = 0;

    for dir in &bsa_dirs {
        let path = Path::new(dir);
        if !path.exists() {
            continue;
        }

        println!("\n{}:", dir.rsplit('/').next().unwrap_or(dir));

        if let Ok(entries) = std::fs::read_dir(path) {
            let mut bsas: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "bsa").unwrap_or(false))
                .collect();
            bsas.sort_by_key(|e| e.path());

            for entry in bsas {
                let bsa_path = entry.path();
                let name = bsa_path.file_name().unwrap().to_string_lossy();

                let (files, disk, ram) = analyze_bsa(&bsa_path);
                total_files += files;
                total_disk += disk;
                total_ram += ram;

                println!(
                    "  {:<48} {:>8} {:>10.1}MB {:>10.1}MB",
                    name,
                    files,
                    disk as f64 / 1024.0 / 1024.0,
                    ram as f64 / 1024.0 / 1024.0
                );
            }
        }
    }

    println!("\n{}", "=".repeat(85));
    println!(
        "{:<50} {:>8} {:>10.1}GB {:>10.1}GB",
        "TOTAL",
        total_files,
        total_disk as f64 / 1024.0 / 1024.0 / 1024.0,
        total_ram as f64 / 1024.0 / 1024.0 / 1024.0
    );

    println!("\nNote: 'Est. RAM' assumes ~3x decompression ratio for compressed files.");
    println!("Actual RAM usage per chunk will be limited to 50% of your available RAM.\n");
}
