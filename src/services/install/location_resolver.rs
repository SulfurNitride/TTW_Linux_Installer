use crate::models::{InstallConfig, Location, Variable};
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// Resolves location indices to actual file paths
pub struct LocationResolver {
    locations: Vec<Location>,
    config: InstallConfig,
    /// Variables read from manifest, keyed by name
    variables: HashMap<String, String>,
}

impl LocationResolver {
    pub fn new(locations: Vec<Location>, config: InstallConfig) -> Self {
        Self {
            locations,
            config,
            variables: HashMap::new(),
        }
    }

    /// Create resolver with variables from manifest
    pub fn with_variables(mut self, manifest_vars: &[Variable]) -> Self {
        // First pass: add all variables with their raw values
        // BUT skip registry-based variables (Type 4 = registry lookup) on Linux
        // as these are Windows-specific and CLI paths should be used instead
        for var in manifest_vars {
            if let (Some(name), Some(value)) = (&var.name, &var.value) {
                // Skip registry-based variables (they contain HKLM paths)
                // Variable Type 4 = Registry lookup (Windows-only)
                if var.var_type == 4 || value.contains("HKLM\\") || value.contains("HKCU\\") {
                    continue;
                }
                // Skip DESTINATION - we always use the CLI-provided destination
                // The manifest may contain a hardcoded Windows path
                if name.to_uppercase() == "DESTINATION" {
                    continue;
                }
                self.variables.insert(name.clone(), value.clone());
            }
        }

        // Second pass: resolve variable references within variables
        // (e.g., %TES4DATA% = %TES4ROOT%\Data)
        let var_names: Vec<String> = self.variables.keys().cloned().collect();
        for _ in 0..5 {
            // Max 5 iterations to resolve nested refs
            let mut changed = false;
            for name in &var_names {
                if let Some(value) = self.variables.get(name).cloned() {
                    let resolved = self.resolve_variable_refs(&value);
                    if resolved != value {
                        self.variables.insert(name.clone(), resolved);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        if !self.variables.is_empty() {
            println!("\nResolved variables from manifest:");
            for (name, value) in &self.variables {
                println!("  %{}% = {}", name, value);
            }
        }

        self
    }

    /// Resolve variable references in a string (without game path substitution)
    fn resolve_variable_refs(&self, path: &str) -> String {
        let mut resolved = path.to_string();

        for (name, value) in &self.variables {
            let var_pattern = format!("%{}%", name);
            resolved = resolved.replace(&var_pattern, value);
        }

        resolved
    }

    /// Resolve a location to its actual path
    pub fn resolve_path(&self, location_index: i32) -> Result<PathBuf> {
        let idx = location_index as usize;
        if idx >= self.locations.len() {
            bail!(
                "Location index {} is out of range (0-{})",
                location_index,
                self.locations.len().saturating_sub(1)
            );
        }

        let location = &self.locations[idx];
        let value = location.value.as_deref().unwrap_or("");
        let resolved = self.resolve_variables(value);

        Ok(PathBuf::from(resolved))
    }

    /// Get location by index
    pub fn get_location(&self, location_index: i32) -> Result<&Location> {
        let idx = location_index as usize;
        if idx >= self.locations.len() {
            bail!("Location index {} is out of range", location_index);
        }
        Ok(&self.locations[idx])
    }

    /// Check if location is a BSA source file
    pub fn is_bsa_location(&self, location_index: i32) -> bool {
        self.get_location(location_index)
            .map(|loc| loc.is_bsa_source())
            .unwrap_or(false)
    }

    /// Check if location is a BSA creation target
    pub fn is_bsa_creation_location(&self, location_index: i32) -> bool {
        self.get_location(location_index)
            .map(|loc| loc.is_bsa_creation())
            .unwrap_or(false)
    }

    /// Get BSA path for a location
    pub fn get_bsa_path(&self, location_index: i32) -> Result<PathBuf> {
        if !self.is_bsa_location(location_index) {
            bail!("Location {} is not a BSA location", location_index);
        }
        self.resolve_path(location_index)
    }

    /// Get directory path for a location
    pub fn get_directory_path(&self, location_index: i32) -> Result<PathBuf> {
        let location = self.get_location(location_index)?;

        if location.is_directory() {
            self.resolve_path(location_index)
        } else if location.is_bsa_creation() {
            // Return the directory containing the BSA
            let bsa_path = self.resolve_path(location_index)?;
            bsa_path
                .parent()
                .map(|p| p.to_path_buf())
                .ok_or_else(|| anyhow::anyhow!("Invalid BSA path"))
        } else {
            bail!("Cannot get directory path for BSA source location type");
        }
    }

    /// Resolve variables in a path string
    fn resolve_variables(&self, path: &str) -> String {
        let mut resolved = path.to_string();

        // First: resolve manifest-defined variables
        for (name, value) in &self.variables {
            let var_pattern = format!("%{}%", name);
            resolved = resolved.replace(&var_pattern, value);
        }

        // Then: resolve game root paths from CLI config
        // These override or complement manifest variables with actual paths
        // Common variable names used across different MPI packages:

        // Fallout 3
        if !self.config.fallout3_root.is_empty() {
            resolved = resolved.replace("%FO3ROOT%", &self.config.fallout3_root);
            resolved =
                resolved.replace("%FO3DATA%", &self.config.fallout3_data().to_string_lossy());
            // Also handle alternative naming conventions
            resolved = resolved.replace("%FALLOUT3ROOT%", &self.config.fallout3_root);
            resolved = resolved.replace(
                "%FALLOUT3DATA%",
                &self.config.fallout3_data().to_string_lossy(),
            );
        }

        // Fallout New Vegas
        if !self.config.falloutnv_root.is_empty() {
            resolved = resolved.replace("%FNVROOT%", &self.config.falloutnv_root);
            resolved =
                resolved.replace("%FNVDATA%", &self.config.falloutnv_data().to_string_lossy());
            resolved = resolved.replace("%FALLOUTNVROOT%", &self.config.falloutnv_root);
            resolved = resolved.replace(
                "%FALLOUTNVDATA%",
                &self.config.falloutnv_data().to_string_lossy(),
            );
        }

        // Oblivion (TES4)
        if !self.config.oblivion_root.is_empty() {
            resolved = resolved.replace("%TES4ROOT%", &self.config.oblivion_root);
            resolved =
                resolved.replace("%TES4DATA%", &self.config.oblivion_data().to_string_lossy());
            resolved = resolved.replace("%OBLIVIONROOT%", &self.config.oblivion_root);
            resolved = resolved.replace(
                "%OBLIVIONDATA%",
                &self.config.oblivion_data().to_string_lossy(),
            );
        }

        // Destination is always needed
        resolved = resolved.replace("%DESTINATION%", &self.config.destination_path);

        // Convert Windows paths to Unix paths
        if std::path::MAIN_SEPARATOR == '/' {
            resolved = resolved.replace('\\', "/");
        }

        // Safety net for Linux/Wine installs: if an MPI manifest leaves a
        // hardcoded Windows absolute path behind, route it to the output
        // directory instead of creating a C: tree.
        #[cfg(not(windows))]
        if is_windows_absolute_path(&resolved) {
            tracing::warn!(
                "Resolved path is a Windows absolute path: {} — replacing with destination: {}",
                resolved,
                self.config.destination_path
            );
            resolved = self.config.destination_path.clone();
        }

        resolved
    }

    /// Print location summary for debugging
    pub fn print_locations(&self) {
        println!("\nLocations:");
        for (i, loc) in self.locations.iter().enumerate() {
            let type_name = match loc.loc_type {
                0 => "Directory",
                1 => "BSA Source",
                2 => "BSA Target",
                _ => "Unknown",
            };
            let resolved = self.resolve_path(i as i32).unwrap_or_default();
            println!(
                "  [{}] {} ({}): {}",
                i,
                loc.name.as_deref().unwrap_or("?"),
                type_name,
                resolved.display()
            );
        }
    }
}

#[cfg(not(windows))]
fn is_windows_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

#[cfg(test)]
mod tests {
    use super::LocationResolver;
    use crate::models::{InstallConfig, Location};
    use std::path::PathBuf;

    fn config() -> InstallConfig {
        InstallConfig {
            fallout3_root: r"D:\Games\Fallout 3".to_string(),
            falloutnv_root: r"C:\Steam\steamapps\common\Fallout New Vegas".to_string(),
            oblivion_root: String::new(),
            destination_path: r"D:\Games\TTW Output".to_string(),
            mpi_package_path: String::new(),
        }
    }

    fn resolver() -> LocationResolver {
        LocationResolver::new(
            vec![
                Location {
                    name: Some("FO3 Root".to_string()),
                    loc_type: 0,
                    value: Some("%FO3ROOT%".to_string()),
                    create_folder: None,
                    archive_type: None,
                    archive_flags: None,
                    files_flags: None,
                    archive_compressed: None,
                },
                Location {
                    name: Some("FNV Root".to_string()),
                    loc_type: 0,
                    value: Some("%FNVROOT%".to_string()),
                    create_folder: None,
                    archive_type: None,
                    archive_flags: None,
                    files_flags: None,
                    archive_compressed: None,
                },
                Location {
                    name: Some("Destination".to_string()),
                    loc_type: 0,
                    value: Some("%DESTINATION%".to_string()),
                    create_folder: None,
                    archive_type: None,
                    archive_flags: None,
                    files_flags: None,
                    archive_compressed: None,
                },
            ],
            config(),
        )
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_stay_on_game_roots_on_windows() {
        assert_eq!(
            resolver().resolve_path(0).unwrap(),
            PathBuf::from(r"D:\Games\Fallout 3")
        );
        assert_eq!(
            resolver().resolve_path(1).unwrap(),
            PathBuf::from(r"C:\Steam\steamapps\common\Fallout New Vegas")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_paths_fall_back_to_destination_on_non_windows() {
        assert_eq!(
            resolver().resolve_path(0).unwrap(),
            PathBuf::from(r"D:\Games\TTW Output")
        );
        assert_eq!(
            resolver().resolve_path(1).unwrap(),
            PathBuf::from(r"D:\Games\TTW Output")
        );
    }
}
