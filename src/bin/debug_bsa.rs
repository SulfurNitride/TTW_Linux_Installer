use anyhow::Result;
use ba2::tes4::Archive;
use ba2::{Reader, ByteSlice};
use std::path::Path;
use std::io::Read;

fn main() -> Result<()> {
    let mpi_path = std::env::args().nth(1).expect("Usage: debug_bsa <path>");

    println!("Opening: {}", mpi_path);

    // Read the first few bytes manually to check format
    let data = std::fs::read(&mpi_path)?;
    println!("Magic: {:?}", &data[0..4]);
    println!("Version: 0x{:02x}", data[4]);
    println!("Folder offset: 0x{:08x}", u32::from_le_bytes([data[8], data[9], data[10], data[11]]));
    println!("Archive flags: 0x{:08x}", u32::from_le_bytes([data[12], data[13], data[14], data[15]]));

    let flags = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    println!("  - Has directory names: {}", (flags & 0x01) != 0);
    println!("  - Has file names: {}", (flags & 0x02) != 0);
    println!("  - Compressed: {}", (flags & 0x04) != 0);
    println!("  - Retain directory names: {}", (flags & 0x08) != 0);
    println!("  - Retain file names: {}", (flags & 0x10) != 0);
    println!("  - Retain file name offsets: {}", (flags & 0x20) != 0);
    println!("  - Xbox archive: {}", (flags & 0x40) != 0);
    println!("  - Retain strings during startup: {}", (flags & 0x80) != 0);
    println!("  - Embed file names: {}", (flags & 0x100) != 0);
    println!("  - XMem codec: {}", (flags & 0x200) != 0);

    // Open with ba2
    let (archive, _meta) = Archive::read(Path::new(&mpi_path))?;

    println!("\nArchive info from ba2:");
    println!("  Directories: {}", archive.len());

    let mut total = 0;
    let mut compressed = 0;

    for (dir_key, folder) in archive.iter().take(3) {
        let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes());
        println!("\nDirectory: '{}'", dir_name);

        for (file_key, file) in folder.iter().take(2) {
            let file_name = String::from_utf8_lossy(file_key.name().as_bytes());
            let raw = file.as_bytes();
            println!("  File: '{}' compressed={} raw_size={}",
                file_name, file.is_compressed(), raw.len());

            total += 1;
            if file.is_compressed() {
                compressed += 1;

                // Check first 16 bytes of compressed data
                if raw.len() >= 16 {
                    println!("    First 16 bytes: {:02x?}", &raw[..16]);

                    // Check if there's an original size prefix (BSA stores 4-byte original size before compressed data)
                    let potential_orig_size = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
                    println!("    First 4 bytes as u32 (potential orig size): {}", potential_orig_size);

                    // Check bytes 4-8 for LZ4 magic
                    if raw.len() >= 8 && raw[4..8] == [0x04, 0x22, 0x4D, 0x18] {
                        println!("    LZ4 magic at offset 4!");

                        // Try LZ4 decompression starting at offset 4
                        let lz4_data = &raw[4..];
                        match decompress_lz4(lz4_data) {
                            Ok(decompressed) => {
                                println!("    LZ4 decompressed (offset 4): {} bytes", decompressed.len());
                            }
                            Err(e) => {
                                println!("    LZ4 decompress failed: {}", e);
                            }
                        }
                    }

                    // Check bytes 0-4 for LZ4 magic
                    if raw[0..4] == [0x04, 0x22, 0x4D, 0x18] {
                        println!("    LZ4 magic at offset 0!");

                        match decompress_lz4(raw) {
                            Ok(decompressed) => {
                                println!("    LZ4 decompressed (offset 0): {} bytes", decompressed.len());
                            }
                            Err(e) => {
                                println!("    LZ4 decompress failed: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    println!("\nTotal sampled: {} files, {} compressed", total, compressed);

    Ok(())
}

fn decompress_lz4(data: &[u8]) -> Result<Vec<u8>> {
    use lz4_flex::frame::FrameDecoder;

    let mut decoder = FrameDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}
