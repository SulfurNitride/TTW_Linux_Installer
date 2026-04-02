use anyhow::{Result, Context};
use std::path::Path;
use std::fs;
use crate::models::{Asset, TtwManifest, Location, Variable};

/// Loads and parses TTW installation manifest
pub struct ManifestLoader;

impl ManifestLoader {
    /// Load manifest from JSON file
    pub fn load_from_file(manifest_path: &Path) -> Result<TtwManifest> {
        if !manifest_path.exists() {
            anyhow::bail!("Manifest not found: {}", manifest_path.display());
        }

        println!("Loading manifest from: {}", manifest_path.display());

        let json = fs::read_to_string(manifest_path)
            .context("Failed to read manifest file")?;

        let manifest: TtwManifest = serde_json::from_str(&json)
            .context("Failed to parse manifest JSON")?;

        // Print summary
        if let Some(pkg) = &manifest.package {
            println!("  Package: {} v{}",
                pkg.title.as_deref().unwrap_or("Unknown"),
                pkg.version.as_deref().unwrap_or("?"));
        }

        let var_count = manifest.variables
            .as_ref()
            .and_then(|v| v.first())
            .map(|v| v.len())
            .unwrap_or(0);

        let loc_count = manifest.locations
            .as_ref()
            .and_then(|l| l.first())
            .map(|l| l.len())
            .unwrap_or(0);

        let asset_count = manifest.assets
            .as_ref()
            .map(|a| a.len())
            .unwrap_or(0);

        println!("  Variables: {} defined", var_count);
        println!("  Locations: {} defined", loc_count);
        println!("  Assets: {} operations", asset_count);

        Ok(manifest)
    }

    /// Parse assets from manifest
    pub fn parse_assets(manifest: &TtwManifest) -> Result<Vec<Asset>> {
        let asset_arrays = match &manifest.assets {
            Some(assets) if !assets.is_empty() => assets,
            _ => return Ok(Vec::new()),
        };

        println!("\nParsing {} assets...", asset_arrays.len());

        let mut assets = Vec::with_capacity(asset_arrays.len());
        let mut _success_count = 0;
        let mut fail_count = 0;

        for asset_value in asset_arrays {
            if let Some(array) = asset_value.as_array() {
                match Asset::from_json_array(array) {
                    Ok(asset) => {
                        assets.push(asset);
                        _success_count += 1;
                    }
                    Err(e) => {
                        fail_count += 1;
                        if fail_count <= 3 {
                            eprintln!("Warning: Failed to parse asset: {}", e);
                        }
                    }
                }
            } else {
                fail_count += 1;
                if fail_count <= 3 {
                    eprintln!("Warning: Asset is not a JSON array");
                }
            }
        }

        if fail_count > 3 {
            eprintln!("Warning: {} more assets failed to parse (messages suppressed)", fail_count - 3);
        }

        println!("Successfully parsed {} assets", assets.len());

        // Print operation type summary
        Self::print_operation_summary(&assets);

        Ok(assets)
    }

    fn print_operation_summary(assets: &[Asset]) {
        use std::collections::HashMap;

        let mut counts: HashMap<i32, usize> = HashMap::new();
        for asset in assets {
            *counts.entry(asset.op_type).or_insert(0) += 1;
        }

        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by_key(|(op_type, _)| *op_type);

        println!("\nOperation summary:");
        for (op_type, count) in sorted {
            let name = match op_type {
                0 => "Copy",
                1 => "New",
                2 => "Patch",
                4 => "OggEnc2",
                5 => "AudioEnc",
                _ => "Unknown",
            };
            println!("  OpType {} ({}): {} operations", op_type, name, count);
        }
    }

    /// Get locations for a specific profile
    pub fn get_locations(manifest: &TtwManifest, profile_index: usize) -> Result<Vec<Location>> {
        let locations = match &manifest.locations {
            Some(locs) if !locs.is_empty() => locs,
            _ => return Ok(Vec::new()),
        };

        if profile_index >= locations.len() {
            anyhow::bail!("Invalid profile index: {}", profile_index);
        }

        Ok(locations[profile_index].clone())
    }

    /// Get variables for a specific profile
    pub fn get_variables(manifest: &TtwManifest, profile_index: usize) -> Result<Vec<Variable>> {
        let variables = match &manifest.variables {
            Some(vars) if !vars.is_empty() => vars,
            _ => return Ok(Vec::new()),
        };

        if profile_index >= variables.len() {
            anyhow::bail!("Invalid profile index: {}", profile_index);
        }

        Ok(variables[profile_index].clone())
    }

    /// Select the best profile index for Linux installation.
    /// Some MPI files have Profile 0 with hardcoded Windows paths (e.g., C:\Users\...)
    /// while Profile 1 uses proper %DESTINATION% variables. On Linux we prefer the
    /// profile that uses variables, since we substitute them with CLI-provided paths.
    pub fn select_best_profile(manifest: &TtwManifest) -> usize {
        let locations = match &manifest.locations {
            Some(locs) if !locs.is_empty() => locs,
            _ => return 0,
        };

        // Check if Profile 0 has hardcoded Windows paths in directory locations
        if let Some(profile0) = locations.first() {
            let has_hardcoded_windows_path = profile0.iter().any(|loc| {
                if loc.loc_type != 0 {
                    return false; // Only check directory locations
                }
                let value = loc.value.as_deref().unwrap_or("");
                // Detect Windows absolute paths: C:\..., D:\..., etc.
                Self::is_windows_absolute_path(value)
            });

            if has_hardcoded_windows_path && locations.len() > 1 {
                // Check if another profile uses %DESTINATION% or other variables instead
                for (idx, profile) in locations.iter().enumerate().skip(1) {
                    let uses_variables = profile.iter().all(|loc| {
                        let value = loc.value.as_deref().unwrap_or("");
                        // Either uses variables, or is a BSA source path (Type 1) which is fine
                        loc.loc_type != 0 || value.contains('%') || value.is_empty()
                    });

                    if uses_variables {
                        println!("  Profile 0 has hardcoded Windows paths, using Profile {} instead", idx);
                        return idx;
                    }
                }
            }
        }

        0
    }

    /// Check if a path looks like a Windows absolute path
    fn is_windows_absolute_path(path: &str) -> bool {
        let path = path.trim();
        // Match patterns like C:\, D:\, C:/, D:/, etc.
        if path.len() >= 3 {
            let bytes = path.as_bytes();
            bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/')
        } else {
            false
        }
    }

    /// Get BSA target locations from the best available profile
    /// Some MPI files have BSA targets in Profile 1 (Windows) but not Profile 0 (Linux)
    /// This method searches all profiles for Type 2 BSA targets with proper flags
    pub fn get_bsa_target_locations(manifest: &TtwManifest) -> Result<Vec<Location>> {
        let locations = match &manifest.locations {
            Some(locs) if !locs.is_empty() => locs,
            _ => return Ok(Vec::new()),
        };

        // Search all profiles for BSA targets (Type 2 with .bsa in Value and flags set)
        for (profile_idx, profile) in locations.iter().enumerate() {
            let bsa_targets: Vec<Location> = profile
                .iter()
                .filter(|loc| {
                    let value = loc.value.as_deref().unwrap_or("");
                    // Type 2 = BSA creation target with proper flags
                    loc.loc_type == 2
                        && value.to_lowercase().ends_with(".bsa")
                        && loc.archive_flags.is_some()
                })
                .cloned()
                .collect();

            if !bsa_targets.is_empty() {
                println!("  Found {} BSA targets in profile {}", bsa_targets.len(), profile_idx);
                return Ok(bsa_targets);
            }
        }

        // Fallback: look for Type 0 with .bsa in Name (TTW 3.4 style)
        for (profile_idx, profile) in locations.iter().enumerate() {
            let bsa_targets: Vec<Location> = profile
                .iter()
                .filter(|loc| {
                    let name = loc.name.as_deref().unwrap_or("");
                    loc.loc_type == 0 && name.to_lowercase().ends_with(".bsa")
                })
                .cloned()
                .collect();

            if !bsa_targets.is_empty() {
                println!("  Found {} BSA targets (Type 0) in profile {}", bsa_targets.len(), profile_idx);
                return Ok(bsa_targets);
            }
        }

        Ok(Vec::new())
    }

    /// Get pre-installation checks from manifest
    pub fn get_checks(manifest: &TtwManifest) -> Vec<crate::models::Check> {
        manifest.checks.clone().unwrap_or_default()
    }

    /// Get post-installation commands from manifest
    pub fn get_post_commands(manifest: &TtwManifest) -> Vec<crate::models::PostCommand> {
        manifest.post_commands.clone().unwrap_or_default()
    }

    /// Execute post-installation commands (translated from Windows to Linux)
    /// These are typically rename/delete operations for BSA files
    pub fn execute_post_commands(
        post_commands: &[crate::models::PostCommand],
        destination: &str,
    ) -> Result<(usize, usize)> {
        let mut success = 0;
        let mut failed = 0;

        for cmd in post_commands {
            let value = match &cmd.value {
                Some(v) => v,
                None => continue,
            };

            // Parse Windows command and translate to Linux operation
            // Format: cmd.exe /C del "path" or cmd.exe /C ren "old" "new"
            let result = Self::execute_single_command(value, destination);

            match result {
                Ok(_) => success += 1,
                Err(e) => {
                    eprintln!("  PostCommand failed: {} - {}", value, e);
                    failed += 1;
                }
            }
        }

        Ok((success, failed))
    }

    /// Execute a single Windows command translated to Linux
    fn execute_single_command(cmd: &str, destination: &str) -> Result<()> {
        use std::fs;
        use std::path::PathBuf;

        // Remove cmd.exe /C prefix
        let cmd = cmd.trim();
        let cmd = cmd.strip_prefix("cmd.exe /C ").unwrap_or(cmd);
        let cmd = cmd.strip_prefix("cmd /C ").unwrap_or(cmd);

        // Replace %DESTINATION% with actual path
        let cmd = cmd.replace("%DESTINATION%", destination);
        // Convert backslashes to forward slashes
        let cmd = cmd.replace('\\', "/");

        if cmd.starts_with("del ") || cmd.starts_with("DEL ") {
            // Delete command: del "path"
            let path = cmd[4..].trim().trim_matches('"');
            let path = PathBuf::from(path);

            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to delete: {}", path.display()))?;
                println!("  Deleted: {}", path.display());
            }
        } else if cmd.starts_with("ren ") || cmd.starts_with("REN ") {
            // Rename command: ren "old" "new"
            let parts: Vec<&str> = cmd[4..].trim().split('"').filter(|s| !s.trim().is_empty()).collect();

            if parts.len() >= 2 {
                let old_path = PathBuf::from(parts[0].trim());
                // New name is just the filename, not full path
                let new_name = parts[1].trim();
                let new_path = old_path.parent().unwrap_or(std::path::Path::new(".")).join(new_name);

                if old_path.exists() {
                    fs::rename(&old_path, &new_path)
                        .with_context(|| format!("Failed to rename: {} -> {}", old_path.display(), new_path.display()))?;
                    println!("  Renamed: {} -> {}", old_path.file_name().unwrap_or_default().to_string_lossy(), new_name);
                } else {
                    // Not an error if source doesn't exist (might not have been created)
                    println!("  Skip rename (not found): {}", old_path.display());
                }
            }
        }

        Ok(())
    }
}
