use std::fs;
use std::path::{Path, PathBuf};
use sysinfo::System;
use tracing::info;

/// Get number of chunks based on available RAM.
///
/// Streaming BSA builders write to disk instead of holding all files in RAM, so
/// chunking can be more aggressive than the original all-in-memory flow.
pub(super) fn get_chunk_count_for_ram() -> usize {
    let mut sys = System::new_all();
    sys.refresh_memory();
    let available_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    let chunks = if available_gb >= 6.0 {
        1
    } else if available_gb >= 4.0 {
        2
    } else {
        4
    };

    info!(
        "System RAM: {:.1}GB available -> {} chunk(s) (streaming mode)",
        available_gb, chunks
    );

    chunks
}

/// Find a file with case-insensitive matching for Linux compatibility.
pub(super) fn find_file_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }

    let parent = path.parent()?;
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();
    let actual_parent = find_dir_case_insensitive(parent)?;

    if let Ok(entries) = fs::read_dir(&actual_parent) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().to_lowercase() == file_name {
                return Some(entry.path());
            }
        }
    }

    None
}

fn find_dir_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }

    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();

    for (i, component) in components.iter().enumerate() {
        let comp_str = component.as_os_str().to_string_lossy();

        if i == 0 {
            current.push(component);
            if !current.exists() {
                return None;
            }
            continue;
        }

        let target = comp_str.to_lowercase();
        let mut found = false;

        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_name().to_string_lossy().to_lowercase() == target {
                    current = entry.path();
                    found = true;
                    break;
                }
            }
        }

        if !found {
            current.push(component);
            if !current.exists() {
                return None;
            }
        }
    }

    current.exists().then_some(current)
}
