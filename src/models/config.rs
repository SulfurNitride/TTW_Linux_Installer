use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Result, bail};

/// Installation configuration and paths
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallConfig {
    /// Fallout 3 root directory
    #[serde(default)]
    pub fallout3_root: String,
    /// Fallout New Vegas root directory
    #[serde(default)]
    pub falloutnv_root: String,
    /// Oblivion root directory
    #[serde(default)]
    pub oblivion_root: String,
    /// Destination directory for installation output
    #[serde(default)]
    pub destination_path: String,
    /// Path to MPI package file or extracted directory
    #[serde(default)]
    pub mpi_package_path: String,
}

impl InstallConfig {
    /// Get Fallout 3 Data directory
    pub fn fallout3_data(&self) -> PathBuf {
        Path::new(&self.fallout3_root).join("Data")
    }

    /// Get Fallout New Vegas Data directory
    pub fn falloutnv_data(&self) -> PathBuf {
        Path::new(&self.falloutnv_root).join("Data")
    }

    /// Get Oblivion Data directory
    pub fn oblivion_data(&self) -> PathBuf {
        Path::new(&self.oblivion_root).join("Data")
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        // Validate Fallout 3 path if provided
        if !self.fallout3_root.is_empty() {
            let root = Path::new(&self.fallout3_root);
            if !root.exists() {
                bail!("Fallout 3 directory not found: {}", self.fallout3_root);
            }
            if !root.join("Fallout3.exe").exists() && !root.join("Fallout3").exists() {
                bail!("Fallout3.exe not found in: {}", self.fallout3_root);
            }
        }

        // Validate Fallout New Vegas path if provided
        if !self.falloutnv_root.is_empty() {
            let root = Path::new(&self.falloutnv_root);
            if !root.exists() {
                bail!("Fallout New Vegas directory not found: {}", self.falloutnv_root);
            }
            if !root.join("FalloutNV.exe").exists() && !root.join("FalloutNV").exists() {
                bail!("FalloutNV.exe not found in: {}", self.falloutnv_root);
            }
        }

        // Validate Oblivion path if provided
        if !self.oblivion_root.is_empty() {
            let root = Path::new(&self.oblivion_root);
            if !root.exists() {
                bail!("Oblivion directory not found: {}", self.oblivion_root);
            }
            if !root.join("Oblivion.exe").exists() && !root.join("Oblivion").exists() {
                bail!("Oblivion.exe not found in: {}", self.oblivion_root);
            }
        }

        // Destination path is always required
        if self.destination_path.is_empty() {
            bail!("Destination path is required");
        }

        // MPI package path is always required
        if self.mpi_package_path.is_empty() {
            bail!("MPI package path is required");
        }

        let mpi_path = Path::new(&self.mpi_package_path);
        let is_mpi_file = mpi_path.is_file()
            && mpi_path.extension().map(|e| e.eq_ignore_ascii_case("mpi")).unwrap_or(false);
        let is_directory = mpi_path.is_dir();

        if !is_mpi_file && !is_directory {
            bail!("MPI package not found: {}", self.mpi_package_path);
        }

        // If it's a directory, validate index.json exists
        if is_directory && !mpi_path.join("_package").join("index.json").exists() {
            bail!("index.json not found in MPI package directory (_package/index.json)");
        }

        Ok(())
    }

    /// Load configuration from a JSON file
    pub fn from_file(config_path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(config_path)?;
        let config: InstallConfig = serde_json::from_str(&json)?;
        Ok(config)
    }

    /// Save configuration to a JSON file
    pub fn save_to_file(&self, config_path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, json)?;
        Ok(())
    }

    /// Print configuration summary
    pub fn print_summary(&self) {
        println!("Installation Configuration:");
        if !self.fallout3_root.is_empty() {
            println!("  Fallout 3:   {}", self.fallout3_root);
        }
        if !self.falloutnv_root.is_empty() {
            println!("  Fallout NV:  {}", self.falloutnv_root);
        }
        if !self.oblivion_root.is_empty() {
            println!("  Oblivion:    {}", self.oblivion_root);
        }
        println!("  Output:      {}", self.destination_path);
        println!("  MPI Package: {}", self.mpi_package_path);
    }
}
