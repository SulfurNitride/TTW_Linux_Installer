use anyhow::{bail, Result};
use std::path::{Component, Path, PathBuf};

/// Join a manifest/archive-relative path to a trusted base directory.
///
/// The MPI manifest and archive names are package-controlled input. Keep them
/// relative so they cannot escape the user-selected game/output directories.
pub(crate) fn safe_join(base: &Path, relative_path: &str) -> Result<PathBuf> {
    let normalized = relative_path.replace('\\', "/");
    let relative = Path::new(normalized.trim_start_matches("./"));

    if relative.as_os_str().is_empty() {
        bail!("Refusing empty relative path");
    }

    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                bail!("Refusing path traversal outside base directory: {relative_path}");
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("Refusing absolute path: {relative_path}");
            }
        }
    }

    if clean.as_os_str().is_empty() {
        bail!("Refusing empty relative path");
    }

    Ok(base.join(clean))
}

#[cfg(test)]
mod tests {
    use super::safe_join;
    use std::path::Path;

    #[test]
    fn safe_join_allows_normal_relative_paths() {
        let joined = safe_join(Path::new("/tmp/install"), r"Data\Meshes\file.nif").unwrap();
        assert_eq!(joined, Path::new("/tmp/install/Data/Meshes/file.nif"));
    }

    #[test]
    fn safe_join_rejects_parent_segments() {
        let err = safe_join(Path::new("/tmp/install"), "../outside.txt").unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn safe_join_rejects_absolute_paths() {
        let err = safe_join(Path::new("/tmp/install"), "/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn safe_join_rejects_empty_paths() {
        let err = safe_join(Path::new("/tmp/install"), "./").unwrap_err();
        assert!(err.to_string().contains("empty relative path"));
    }
}
