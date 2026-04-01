use anyhow::{Result, Context, bail};
use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use tracing::info;

/// Manages VCDIFF/xdelta3 patch operations.
///
/// Uses oxidelta (pure Rust) for in-memory patching (no subprocess, no temp files).
/// Falls back to the xdelta3 binary for file-based operations if needed.
pub struct XdeltaManager {
    /// Path to the xdelta3 binary (for file-based patching fallback)
    xdelta_path: Option<PathBuf>,
    /// Directory for temp files (uses output dir, not system temp)
    staging_dir: PathBuf,
}

impl XdeltaManager {
    /// Create manager with staging directory. Finds xdelta3 binary as optional fallback.
    pub fn new(xdelta_path: PathBuf, staging_dir: PathBuf) -> Self {
        Self { xdelta_path: Some(xdelta_path), staging_dir }
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
            if mode & 0o111 == 0 {
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

        let tools_path = Self::default_path();
        if tools_path.exists() {
            #[cfg(unix)]
            Self::try_make_executable(&tools_path);
            if Self::test_binary(&tools_path) {
                return Some(tools_path);
            }
        }

        #[cfg(not(windows))]
        {
            for path in &["/usr/bin/xdelta3", "/usr/local/bin/xdelta3"] {
                let p = PathBuf::from(path);
                if p.exists() && Self::test_binary(&p) {
                    return Some(p);
                }
            }

            if let Ok(output) = Command::new("which").arg("xdelta3").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        let p = PathBuf::from(&path);
                        if Self::test_binary(&p) {
                            return Some(p);
                        }
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            if let Ok(output) = Command::new("where").arg("xdelta3.exe").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout)
                        .lines().next().unwrap_or("").trim().to_string();
                    if !path.is_empty() {
                        let p = PathBuf::from(&path);
                        if Self::test_binary(&p) {
                            return Some(p);
                        }
                    }
                }
            }
        }

        // Check tools relative to cwd and parent directories
        let cwd_tools = PathBuf::from("tools").join(binary_name);
        if cwd_tools.exists() {
            #[cfg(unix)]
            Self::try_make_executable(&cwd_tools);
            if Self::test_binary(&cwd_tools) {
                return Some(cwd_tools.canonicalize().unwrap_or(cwd_tools));
            }
        }

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

    /// Ensure xdelta3 capability is available.
    /// oxidelta (pure Rust) is always available for in-memory patching.
    /// The xdelta3 binary is optional (used as fallback for file-based ops).
    pub fn ensure_available(staging_dir: PathBuf) -> Result<Self> {
        let binary = Self::find_xdelta3();
        if let Some(ref path) = binary {
            info!("xdelta3 binary found: {} (fallback for file ops)", path.display());
        } else {
            info!("xdelta3 binary not found; using oxidelta (pure Rust) for all patching");
        }

        Ok(Self {
            xdelta_path: binary,
            staging_dir,
        })
    }

    /// Check if xdelta3 binary is working (optional, oxidelta doesn't need this)
    pub fn verify(&self) -> Result<String> {
        if let Some(ref path) = self.xdelta_path {
            let output = Command::new(path)
                .arg("-V")
                .output()
                .context("Failed to run xdelta3")?;
            if !output.status.success() {
                bail!("xdelta3 returned error");
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Ok("oxidelta (pure Rust VCDIFF)".to_string())
        }
    }

    /// Apply a delta patch to create output file (uses binary if available)
    pub fn apply_patch(&self, source: &Path, patch: &Path, output: &Path) -> Result<()> {
        // Try oxidelta first (pure Rust, no subprocess)
        let source_data = fs::read(source)
            .with_context(|| format!("Failed to read source: {}", source.display()))?;
        let patch_data = fs::read(patch)
            .with_context(|| format!("Failed to read patch: {}", patch.display()))?;

        let result = self.apply_patch_from_bytes(&source_data, &patch_data)?;

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, &result)?;
        Ok(())
    }

    /// Apply a delta patch from bytes (pure Rust, no subprocess, no temp files).
    /// Uses oxidelta for VCDIFF decoding directly in memory.
    pub fn apply_patch_from_bytes(
        &self,
        source_data: &[u8],
        patch_data: &[u8],
    ) -> Result<Vec<u8>> {
        oxidelta::compress::decoder::decode_all(source_data, patch_data)
            .context("VCDIFF patch failed (oxidelta)")
    }

    /// Get path to xdelta3 binary (if available)
    pub fn path(&self) -> &Path {
        self.xdelta_path.as_deref().unwrap_or(Path::new("oxidelta"))
    }
}
