use super::ManifestLoader;
use crate::models::{PostCommand, TtwManifest};
use anyhow::{Context, Result};
use std::path::{Component, PathBuf};

impl ManifestLoader {
    /// Get post-installation commands from manifest
    pub fn get_post_commands(manifest: &TtwManifest) -> Vec<PostCommand> {
        manifest.post_commands.clone().unwrap_or_default()
    }

    /// Execute post-installation commands (translated from Windows to Linux)
    /// These are typically rename/delete operations for BSA files.
    pub fn execute_post_commands(
        post_commands: &[PostCommand],
        destination: &str,
    ) -> Result<(usize, usize)> {
        let mut success = 0;
        let mut failed = 0;

        for cmd in post_commands {
            let value = match &cmd.value {
                Some(v) => v,
                None => continue,
            };

            match Self::execute_single_command(value, destination) {
                Ok(_) => success += 1,
                Err(e) => {
                    eprintln!("  PostCommand failed: {} - {}", value, e);
                    failed += 1;
                }
            }
        }

        Ok((success, failed))
    }

    /// Execute a single Windows command translated to Linux.
    fn execute_single_command(cmd: &str, destination: &str) -> Result<()> {
        let cmd = cmd.trim();
        let cmd = cmd.strip_prefix("cmd.exe /C ").unwrap_or(cmd);
        let cmd = cmd.strip_prefix("cmd /C ").unwrap_or(cmd);
        let cmd = cmd.replace("%DESTINATION%", destination);
        let cmd = cmd.replace('\\', "/");

        if cmd.starts_with("del ") || cmd.starts_with("DEL ") {
            let path = cmd[4..].trim().trim_matches('"');
            let path = post_command_path(destination, path)?;

            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("Failed to delete: {}", path.display()))?;
                println!("  Deleted: {}", path.display());
            }
        } else if cmd.starts_with("ren ") || cmd.starts_with("REN ") {
            let parts: Vec<&str> = cmd[4..]
                .trim()
                .split('"')
                .filter(|s| !s.trim().is_empty())
                .collect();

            if parts.len() >= 2 {
                let old_path = post_command_path(destination, parts[0].trim())?;
                let new_name = parts[1].trim();
                if !is_plain_filename(new_name) {
                    anyhow::bail!(
                        "Refusing rename target that is not a plain filename: {new_name}"
                    );
                }
                let new_path = old_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(new_name);

                if old_path.exists() {
                    std::fs::rename(&old_path, &new_path).with_context(|| {
                        format!(
                            "Failed to rename: {} -> {}",
                            old_path.display(),
                            new_path.display()
                        )
                    })?;
                    println!(
                        "  Renamed: {} -> {}",
                        old_path.file_name().unwrap_or_default().to_string_lossy(),
                        new_name
                    );
                } else {
                    println!("  Skip rename (not found): {}", old_path.display());
                }
            }
        }

        Ok(())
    }
}

fn post_command_path(destination: &str, raw_path: &str) -> Result<PathBuf> {
    let destination = normalize_lexical(PathBuf::from(destination))?;
    let raw = PathBuf::from(raw_path);
    let path = if raw.is_absolute() {
        normalize_lexical(raw)?
    } else {
        normalize_lexical(destination.join(raw))?
    };

    if !path.starts_with(&destination) {
        anyhow::bail!(
            "Refusing post-install path outside destination: {}",
            path.display()
        );
    }

    Ok(path)
}

fn is_plain_filename(name: &str) -> bool {
    let path = PathBuf::from(name);
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn normalize_lexical(path: PathBuf) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    anyhow::bail!("Refusing path traversal in post-install command");
                }
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::ManifestLoader;
    use crate::models::PostCommand;
    use std::fs;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ttw_installer_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn post_commands_delete_inside_destination() {
        let dir = test_dir("delete_inside");
        let file = dir.join("delete-me.txt");
        fs::write(&file, b"remove").unwrap();

        let commands = [PostCommand {
            value: Some(format!("cmd.exe /C del \"{}\"", file.display())),
            wait: false,
            hidden: false,
        }];

        let (success, failed) =
            ManifestLoader::execute_post_commands(&commands, &dir.to_string_lossy()).unwrap();

        assert_eq!((success, failed), (1, 0));
        assert!(!file.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commands_reject_delete_outside_destination() {
        let dir = test_dir("delete_outside");
        let outside = std::env::temp_dir().join(format!(
            "ttw_installer_outside_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, b"keep").unwrap();

        let commands = [PostCommand {
            value: Some(format!("cmd.exe /C del \"{}\"", outside.display())),
            wait: false,
            hidden: false,
        }];

        let (success, failed) =
            ManifestLoader::execute_post_commands(&commands, &dir.to_string_lossy()).unwrap();

        assert_eq!((success, failed), (0, 1));
        assert!(outside.exists());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commands_reject_rename_to_relative_path() {
        let dir = test_dir("rename_relative_path");
        let file = dir.join("old.txt");
        fs::write(&file, b"keep").unwrap();

        let commands = [PostCommand {
            value: Some(format!(
                "cmd.exe /C ren \"{}\" \"../outside.txt\"",
                file.display()
            )),
            wait: false,
            hidden: false,
        }];

        let (success, failed) =
            ManifestLoader::execute_post_commands(&commands, &dir.to_string_lossy()).unwrap();

        assert_eq!((success, failed), (0, 1));
        assert!(file.exists());
        assert!(!dir.join("..").join("outside.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn post_commands_reject_rename_to_current_dir() {
        let dir = test_dir("rename_current_dir");
        let file = dir.join("old.txt");
        fs::write(&file, b"keep").unwrap();

        let commands = [PostCommand {
            value: Some(format!("cmd.exe /C ren \"{}\" \".\"", file.display())),
            wait: false,
            hidden: false,
        }];

        let (success, failed) =
            ManifestLoader::execute_post_commands(&commands, &dir.to_string_lossy()).unwrap();

        assert_eq!((success, failed), (0, 1));
        assert!(file.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
