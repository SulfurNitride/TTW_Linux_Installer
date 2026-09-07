mod extractor;
mod index;
mod store;

pub use extractor::*;
pub use index::*;
use indicatif::ProgressStyle;
pub use store::*;

/// Normalize an MPI path for Windows-style, case-insensitive lookup.
pub(crate) fn normalize_mpi_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_lowercase()
}

fn archive_progress_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .expect("valid progress template")
        .progress_chars("#>-")
}

/// LZ4 frame magic number.
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];

#[cfg(test)]
mod tests {
    use super::normalize_mpi_path;

    #[test]
    fn normalizes_manifest_paths_like_windows() {
        assert_eq!(
            normalize_mpi_path(r"textures\characters\FaceMods\FalloutNV.esm\00030A41_0.dds"),
            "textures/characters/facemods/falloutnv.esm/00030a41_0.dds"
        );
        assert_eq!(
            normalize_mpi_path("./Video/B30.bik.xd3"),
            "video/b30.bik.xd3"
        );
    }
}
