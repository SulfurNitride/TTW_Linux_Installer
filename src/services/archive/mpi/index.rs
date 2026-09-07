use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::normalize_mpi_path;

/// Case-insensitive index of files extracted from an MPI package.
///
/// MPI manifests use Windows paths, while Linux filesystems are normally
/// case-sensitive. Indexing once avoids walking the extracted directory tree
/// for every asset lookup and ensures manifest casing does not affect installs.
pub struct MpiPathIndex {
    files: HashMap<String, PathBuf>,
}

impl MpiPathIndex {
    pub fn build(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!("MPI extraction directory not found: {}", root.display());
        }

        let mut files = HashMap::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.with_context(|| {
                format!(
                    "Failed to index extracted MPI directory: {}",
                    root.display()
                )
            })?;
            if !entry.file_type().is_file() {
                continue;
            }

            let relative = entry.path().strip_prefix(root).with_context(|| {
                format!(
                    "Failed to make MPI path relative: {}",
                    entry.path().display()
                )
            })?;
            let key = normalize_mpi_path(&relative.to_string_lossy());

            if let Some(existing) = files.insert(key.clone(), entry.path().to_path_buf()) {
                bail!(
                    "MPI contains paths that differ only by case: {} and {} (normalized: {})",
                    existing.display(),
                    entry.path().display(),
                    key
                );
            }
        }

        println!(
            "Indexed {} MPI files for case-insensitive lookup",
            files.len()
        );
        Ok(Self { files })
    }

    pub fn get(&self, path: &str) -> Option<&Path> {
        self.files
            .get(&normalize_mpi_path(path))
            .map(PathBuf::as_path)
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::MpiPathIndex;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_reported_ttw_paths_case_insensitively() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ttw-mpi-case-index-{}-{unique}",
            std::process::id()
        ));
        let face = root.join("textures/characters/facemods/falloutnv.esm/00030a41_0.dds");
        let video = root.join("video/b30.bik.xd3");
        fs::create_dir_all(face.parent().unwrap()).unwrap();
        fs::create_dir_all(video.parent().unwrap()).unwrap();
        fs::write(&face, b"face").unwrap();
        fs::write(&video, b"video").unwrap();

        let index = MpiPathIndex::build(&root).unwrap();
        assert_eq!(index.file_count(), 2);
        assert_eq!(
            index
                .get("textures/characters/FaceMods/FalloutNV.esm/00030A41_0.dds")
                .unwrap(),
            face
        );
        assert_eq!(index.get(r"Video\B30.bik.xd3").unwrap(), video);

        fs::remove_dir_all(root).unwrap();
    }
}
