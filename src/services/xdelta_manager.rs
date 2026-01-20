use anyhow::{Result, Context, bail};
use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;

/// Manages the xdelta3 binary for patch operations
pub struct XdeltaManager {
    /// Path to the xdelta3 binary
    xdelta_path: PathBuf,
}

impl XdeltaManager {
    /// Create manager with path to xdelta3 binary
    pub fn new(xdelta_path: PathBuf) -> Self {
        Self { xdelta_path }
    }

    /// Get default xdelta3 path (in tools directory)
    pub fn default_path() -> PathBuf {
        let exe_dir = std::env::current_exe()
            .map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf())
            .unwrap_or_else(|_| PathBuf::from("."));

        exe_dir.join("tools").join("xdelta3")
    }

    /// Try to find xdelta3 in common locations
    pub fn find_xdelta3() -> Option<PathBuf> {
        // Check our tools directory (next to executable)
        let tools_path = Self::default_path();
        if tools_path.exists() {
            return Some(tools_path);
        }

        // Check tools directory relative to cwd
        let cwd_tools = PathBuf::from("tools/xdelta3");
        if cwd_tools.exists() {
            return Some(cwd_tools.canonicalize().unwrap_or(cwd_tools));
        }

        // Check parent directories (for running from target/release/)
        if let Ok(exe_path) = std::env::current_exe() {
            // Go up from target/release/ to project root
            if let Some(exe_dir) = exe_path.parent() {
                // Check ../../tools/xdelta3 (project root when in target/release/)
                let project_tools = exe_dir.join("../../tools/xdelta3");
                if project_tools.exists() {
                    return Some(project_tools.canonicalize().unwrap_or(project_tools));
                }
                // Check ../tools/xdelta3 (one level up)
                let parent_tools = exe_dir.join("../tools/xdelta3");
                if parent_tools.exists() {
                    return Some(parent_tools.canonicalize().unwrap_or(parent_tools));
                }
            }
        }

        // Check system PATH
        if let Ok(output) = Command::new("which").arg("xdelta3").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        // Check common locations
        let common_paths = [
            "/usr/bin/xdelta3",
            "/usr/local/bin/xdelta3",
            "/opt/xdelta3/xdelta3",
        ];

        for path in common_paths {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }

        None
    }

    /// Ensure xdelta3 is available
    pub fn ensure_available() -> Result<Self> {
        if let Some(path) = Self::find_xdelta3() {
            return Ok(Self::new(path));
        }

        // Provide helpful installation instructions
        #[cfg(target_os = "linux")]
        bail!(
            "xdelta3 not found. Please install it using your package manager:\n\
            \n\
            Arch/CachyOS:  sudo pacman -S xdelta3\n\
            Ubuntu/Debian: sudo apt install xdelta3\n\
            Fedora:        sudo dnf install xdelta\n\
            \n\
            Or place the xdelta3 binary in the 'tools' folder next to this application."
        );

        #[cfg(target_os = "windows")]
        bail!(
            "xdelta3.exe not found. Please download it from:\n\
            https://github.com/jmacd/xdelta/releases\n\
            \n\
            Place xdelta3.exe in the 'tools' folder next to this application."
        );

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        bail!("xdelta3 not found. Please install xdelta3 and ensure it's in your PATH.");
    }

    /// Check if xdelta3 is working
    pub fn verify(&self) -> Result<String> {
        let output = Command::new(&self.xdelta_path)
            .arg("-V")
            .output()
            .context("Failed to run xdelta3")?;

        if !output.status.success() {
            bail!("xdelta3 returned error");
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Apply a delta patch to create output file
    /// xdelta3 -d -s <source> <patch> <output>
    pub fn apply_patch(&self, source: &Path, patch: &Path, output: &Path) -> Result<()> {
        let status = Command::new(&self.xdelta_path)
            .arg("-d")
            .arg("-s")
            .arg(source)
            .arg(patch)
            .arg(output)
            .status()
            .with_context(|| format!("Failed to run xdelta3 patch: {} -> {}",
                patch.display(), output.display()))?;

        if !status.success() {
            bail!("xdelta3 patch failed with status: {}", status);
        }

        Ok(())
    }

    /// Apply a delta patch from bytes (for BSA-extracted patches)
    pub fn apply_patch_from_bytes(
        &self,
        source_data: &[u8],
        patch_data: &[u8],
    ) -> Result<Vec<u8>> {
        // Create temp files for the operation
        let temp_dir = tempfile::tempdir()
            .context("Failed to create temp directory")?;

        let source_path = temp_dir.path().join("source");
        let patch_path = temp_dir.path().join("patch.xdelta");
        let output_path = temp_dir.path().join("output");

        fs::write(&source_path, source_data)?;
        fs::write(&patch_path, patch_data)?;

        self.apply_patch(&source_path, &patch_path, &output_path)?;

        let result = fs::read(&output_path)
            .context("Failed to read patched output")?;

        Ok(result)
    }

    /// Get path to xdelta3 binary
    pub fn path(&self) -> &Path {
        &self.xdelta_path
    }
}
