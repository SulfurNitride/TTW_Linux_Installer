mod extractor;
mod store;

pub use extractor::*;
use indicatif::ProgressStyle;
pub use store::*;

fn archive_progress_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .expect("valid progress template")
        .progress_chars("#>-")
}

/// LZ4 frame magic number.
#[cfg(not(feature = "dream-reader"))]
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];
