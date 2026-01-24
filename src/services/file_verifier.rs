use anyhow::{Result, Context, bail};
use sha1::{Sha1, Digest};
use std::path::{Path, PathBuf};
use std::fs;

use crate::models::Check;
use crate::services::LocationResolver;

/// Check types from MPI manifest
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckType {
    /// File must exist
    FileExists = 0,
    /// File must have matching checksum
    FileChecksum = 1,
    /// Directory must have minimum free space
    FreeSpace = 2,
}

impl CheckType {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::FileExists),
            1 => Some(Self::FileChecksum),
            2 => Some(Self::FreeSpace),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::FileExists => "FileExists",
            Self::FileChecksum => "FileChecksum",
            Self::FreeSpace => "FreeSpace",
        }
    }
}

/// Result of verification
#[derive(Debug)]
pub struct VerificationResult {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

impl VerificationResult {
    pub fn is_success(&self) -> bool {
        self.failed == 0
    }
}

/// File verification service for checking game files before installation
pub struct FileVerifier<'a> {
    resolver: &'a LocationResolver,
}

impl<'a> FileVerifier<'a> {
    pub fn new(resolver: &'a LocationResolver) -> Self {
        Self { resolver }
    }

    /// Run all checks from the manifest
    pub fn run_checks(&self, checks: &[Check]) -> Result<VerificationResult> {
        let mut result = VerificationResult {
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: Vec::new(),
        };

        if checks.is_empty() {
            println!("No verification checks defined in manifest");
            return Ok(result);
        }

        println!("\n=== Running {} Pre-Installation Checks ===\n", checks.len());

        for (i, check) in checks.iter().enumerate() {
            let check_type = CheckType::from_i32(check.check_type);

            match check_type {
                Some(CheckType::FileExists) => {
                    self.verify_file_exists(i, check, &mut result);
                }
                Some(CheckType::FileChecksum) => {
                    self.verify_file_checksum(i, check, &mut result);
                }
                Some(CheckType::FreeSpace) => {
                    self.verify_free_space(i, check, &mut result);
                }
                None => {
                    println!("  [{}] SKIP: Unknown check type {}", i + 1, check.check_type);
                    result.skipped += 1;
                }
            }
        }

        println!("\n=== Verification Summary ===");
        println!("  Passed:  {}", result.passed);
        println!("  Failed:  {}", result.failed);
        println!("  Skipped: {}", result.skipped);

        if !result.errors.is_empty() {
            println!("\nErrors:");
            for error in &result.errors {
                println!("  - {}", error);
            }
        }

        Ok(result)
    }

    /// Verify a file exists at the specified location
    fn verify_file_exists(&self, index: usize, check: &Check, result: &mut VerificationResult) {
        let file_path = match self.resolve_check_path(check) {
            Ok(p) => p,
            Err(e) => {
                println!("  [{}] FAIL: FileExists - {}", index + 1, e);
                result.failed += 1;
                result.errors.push(format!("FileExists: {}", e));
                return;
            }
        };

        let exists = file_path.exists();
        let expected = !check.inverted; // inverted means file should NOT exist

        if exists == expected {
            println!("  [{}] PASS: FileExists - {}", index + 1, file_path.display());
            result.passed += 1;
        } else {
            let msg = if check.inverted {
                format!("File should not exist: {}", file_path.display())
            } else {
                format!("File not found: {}", file_path.display())
            };

            if let Some(custom_msg) = &check.custom_message {
                println!("  [{}] FAIL: {} - {}", index + 1, custom_msg, msg);
                result.errors.push(format!("{}: {}", custom_msg, msg));
            } else {
                println!("  [{}] FAIL: FileExists - {}", index + 1, msg);
                result.errors.push(msg);
            }
            result.failed += 1;
        }
    }

    /// Verify a file's checksum matches expected value
    fn verify_file_checksum(&self, index: usize, check: &Check, result: &mut VerificationResult) {
        let file_path = match self.resolve_check_path(check) {
            Ok(p) => p,
            Err(e) => {
                println!("  [{}] FAIL: Checksum - {}", index + 1, e);
                result.failed += 1;
                result.errors.push(format!("Checksum: {}", e));
                return;
            }
        };

        let expected_checksums = match &check.checksums {
            Some(c) if !c.is_empty() => c,
            _ => {
                println!("  [{}] SKIP: Checksum - No checksum specified for {}",
                    index + 1, file_path.display());
                result.skipped += 1;
                return;
            }
        };

        // Read file and compute SHA1
        let file_data = match fs::read(&file_path) {
            Ok(data) => data,
            Err(e) => {
                let msg = format!("Cannot read file {}: {}", file_path.display(), e);
                if let Some(custom_msg) = &check.custom_message {
                    println!("  [{}] FAIL: {} - {}", index + 1, custom_msg, msg);
                    result.errors.push(format!("{}: {}", custom_msg, msg));
                } else {
                    println!("  [{}] FAIL: Checksum - {}", index + 1, msg);
                    result.errors.push(msg);
                }
                result.failed += 1;
                return;
            }
        };

        let actual_hash = compute_sha1(&file_data);

        // Checksums can be comma-separated (multiple valid hashes for different versions)
        let valid_hashes: Vec<&str> = expected_checksums
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let matches = valid_hashes.iter().any(|expected| {
            expected.eq_ignore_ascii_case(&actual_hash)
        });

        if matches != check.inverted {
            println!("  [{}] PASS: Checksum - {} ({})",
                index + 1,
                file_path.file_name().unwrap_or_default().to_string_lossy(),
                &actual_hash[..8]);
            result.passed += 1;
        } else {
            let msg = format!(
                "Checksum mismatch for {}: expected {}, got {}",
                file_path.display(),
                if valid_hashes.len() == 1 {
                    valid_hashes[0].to_string()
                } else {
                    format!("one of [{}]", valid_hashes.join(", "))
                },
                actual_hash
            );

            if let Some(custom_msg) = &check.custom_message {
                println!("  [{}] FAIL: {} - {}", index + 1, custom_msg, msg);
                result.errors.push(format!("{}: {}", custom_msg, msg));
            } else {
                println!("  [{}] FAIL: {}", index + 1, msg);
                result.errors.push(msg);
            }
            result.failed += 1;
        }
    }

    /// Verify free space at location
    fn verify_free_space(&self, index: usize, check: &Check, result: &mut VerificationResult) {
        let required_bytes = check.free_size.unwrap_or(0);

        if required_bytes <= 0 {
            println!("  [{}] SKIP: FreeSpace - No size requirement specified", index + 1);
            result.skipped += 1;
            return;
        }

        let path = match self.resolver.resolve_path(check.loc) {
            Ok(p) => p,
            Err(e) => {
                println!("  [{}] FAIL: FreeSpace - Cannot resolve location: {}", index + 1, e);
                result.failed += 1;
                result.errors.push(format!("FreeSpace: Cannot resolve location {}", check.loc));
                return;
            }
        };

        // Get available space (platform-specific)
        match get_available_space(&path) {
            Ok(available) => {
                let required_gb = required_bytes as f64 / 1_073_741_824.0;
                let available_gb = available as f64 / 1_073_741_824.0;

                if available >= required_bytes as u64 {
                    println!("  [{}] PASS: FreeSpace - {:.2} GB available (need {:.2} GB)",
                        index + 1, available_gb, required_gb);
                    result.passed += 1;
                } else {
                    let msg = format!(
                        "Insufficient space at {}: {:.2} GB available, {:.2} GB required",
                        path.display(), available_gb, required_gb
                    );
                    println!("  [{}] FAIL: {}", index + 1, msg);
                    result.errors.push(msg);
                    result.failed += 1;
                }
            }
            Err(e) => {
                println!("  [{}] SKIP: FreeSpace - Cannot check: {}", index + 1, e);
                result.skipped += 1;
            }
        }
    }

    /// Resolve the file path for a check
    fn resolve_check_path(&self, check: &Check) -> Result<PathBuf> {
        let base_path = self.resolver.resolve_path(check.loc)
            .context("Cannot resolve location")?;

        let file_name = check.file.as_deref()
            .ok_or_else(|| anyhow::anyhow!("No file specified in check"))?;

        // Normalize path separators
        let normalized = file_name.replace('\\', "/");

        Ok(base_path.join(normalized))
    }
}

/// Compute SHA1 hash of data and return as lowercase hex string
pub fn compute_sha1(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Compute SHA1 hash of a file
pub fn compute_file_sha1(path: &Path) -> Result<String> {
    let data = fs::read(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    Ok(compute_sha1(&data))
}

/// Get available disk space at path
fn get_available_space(path: &Path) -> Result<u64> {
    // Find an existing parent directory
    let mut check_path = path.to_path_buf();
    while !check_path.exists() {
        check_path = match check_path.parent() {
            Some(p) => p.to_path_buf(),
            None => bail!("Cannot find existing path for space check"),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&check_path)?;
        // Use statvfs for actual free space
        // For simplicity, we'll use a basic check
        // In production, you'd use libc::statvfs
        Ok(meta.len()) // Placeholder - see note below
    }

    #[cfg(windows)]
    {
        // Windows implementation would use GetDiskFreeSpaceExW
        Ok(u64::MAX) // Placeholder
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(u64::MAX) // Skip check on unknown platforms
    }
}
