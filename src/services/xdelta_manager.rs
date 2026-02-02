use anyhow::{Result, Context, bail};
use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use tracing::info;

/// Manages the xdelta3 binary for patch operations
pub struct XdeltaManager {
    /// Path to the xdelta3 binary
    xdelta_path: PathBuf,
    /// Directory for temp files (uses output dir, not system temp)
    staging_dir: PathBuf,
}

impl XdeltaManager {
    /// Create manager with path to xdelta3 binary and staging directory
    pub fn new(xdelta_path: PathBuf, staging_dir: PathBuf) -> Self {
        Self { xdelta_path, staging_dir }
    }

    /// Test if a binary actually works
    fn test_binary(path: &Path) -> bool {
        Command::new(path)
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get default xdelta3 path (in tools directory)
    pub fn default_path() -> PathBuf {
        let exe_dir = std::env::current_exe()
            .map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf())
            .unwrap_or_else(|_| PathBuf::from("."));

        #[cfg(windows)]
        let binary_name = "xdelta3.exe";
        #[cfg(not(windows))]
        let binary_name = "xdelta3";

        exe_dir.join("tools").join(binary_name)
    }

    /// Try to make a binary executable on Unix
    #[cfg(unix)]
    fn try_make_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mode = metadata.permissions().mode();
            // Check if executable bit is set
            if mode & 0o111 == 0 {
                // Try to make it executable
                info!("Setting execute permission on: {}", path.display());
                let mut perms = metadata.permissions();
                perms.set_mode(mode | 0o755);
                if fs::set_permissions(path, perms).is_ok() {
                    return true;
                }
            }
        }
        false
    }

    /// Try to find xdelta3 in common locations
    pub fn find_xdelta3() -> Option<PathBuf> {
        #[cfg(windows)]
        let binary_name = "xdelta3.exe";
        #[cfg(not(windows))]
        let binary_name = "xdelta3";

        // First, try bundled xdelta3 in tools directory (next to executable)
        let tools_path = Self::default_path();
        if tools_path.exists() {
            #[cfg(unix)]
            Self::try_make_executable(&tools_path);

            if Self::test_binary(&tools_path) {
                info!("Using bundled xdelta3: {}", tools_path.display());
                return Some(tools_path);
            }
            info!("Bundled xdelta3 at {} doesn't work, trying system xdelta3", tools_path.display());
        }

        // Second, try system-installed xdelta3 (most reliable on Linux)
        #[cfg(not(windows))]
        {
            // Check common system locations directly first
            for path in &["/usr/bin/xdelta3", "/usr/local/bin/xdelta3"] {
                let p = PathBuf::from(path);
                if p.exists() && Self::test_binary(&p) {
                    info!("Using system xdelta3: {}", p.display());
                    return Some(p);
                }
            }

            // Try 'which' command
            if let Ok(output) = Command::new("which").arg("xdelta3").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        let p = PathBuf::from(&path);
                        if Self::test_binary(&p) {
                            info!("Using xdelta3 from PATH: {}", path);
                            return Some(p);
                        }
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            // Use 'where' command on Windows
            if let Ok(output) = Command::new("where").arg("xdelta3.exe").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !path.is_empty() {
                        let p = PathBuf::from(&path);
                        if Self::test_binary(&p) {
                            return Some(p);
                        }
                    }
                }
            }

            // Check common Windows locations
            for path in &[
                r"C:\Program Files\xdelta3\xdelta3.exe",
                r"C:\Program Files (x86)\xdelta3\xdelta3.exe",
                r"C:\xdelta3\xdelta3.exe",
                r"C:\Tools\xdelta3.exe",
            ] {
                let p = PathBuf::from(path);
                if p.exists() && Self::test_binary(&p) {
                    return Some(p);
                }
            }
        }

        // Third, try tools directory relative to cwd (for development)
        let cwd_tools = PathBuf::from("tools").join(binary_name);
        if cwd_tools.exists() {
            #[cfg(unix)]
            Self::try_make_executable(&cwd_tools);

            if Self::test_binary(&cwd_tools) {
                return Some(cwd_tools.canonicalize().unwrap_or(cwd_tools));
            }
        }

        // Fourth, check parent directories (for running from target/release/)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                for rel_path in &["../../tools", "../tools"] {
                    let p = exe_dir.join(rel_path).join(binary_name);
                    if p.exists() {
                        #[cfg(unix)]
                        Self::try_make_executable(&p);

                        if Self::test_binary(&p) {
                            return Some(p.canonicalize().unwrap_or(p));
                        }
                    }
                }
            }
        }

        None
    }

    /// Ensure xdelta3 is available
    pub fn ensure_available(staging_dir: PathBuf) -> Result<Self> {
        if let Some(path) = Self::find_xdelta3() {
            return Ok(Self::new(path, staging_dir));
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
        let result = Command::new(&self.xdelta_path)
            .arg("-d")
            .arg("-s")
            .arg(source)
            .arg(patch)
            .arg(output)
            .output()
            .with_context(|| format!("Failed to execute xdelta3 (path: {})",
                self.xdelta_path.display()))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stdout = String::from_utf8_lossy(&result.stdout);
            bail!(
                "xdelta3 patch failed (status {}): {}\nSource: {}\nPatch: {}\nstdout: {}\nstderr: {}",
                result.status,
                if stderr.is_empty() { "no error message" } else { stderr.trim() },
                source.display(),
                patch.display(),
                stdout.trim(),
                stderr.trim()
            );
        }

        Ok(())
    }

    /// Apply a delta patch from bytes (for BSA-extracted patches)
    pub fn apply_patch_from_bytes(
        &self,
        source_data: &[u8],
        patch_data: &[u8],
    ) -> Result<Vec<u8>> {
        // Create temp files in staging directory (not system temp - may be tmpfs with limited space)
        let temp_dir = tempfile::Builder::new()
            .prefix(".ttw_xdelta_")
            .tempdir_in(&self.staging_dir)
            .context("Failed to create temp directory for xdelta")?;

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
