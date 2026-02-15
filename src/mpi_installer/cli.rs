use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{error, info, warn};

use ttw_installer::{
    models::InstallConfig,
    services::{
        AssetProcessor, FileVerifier, LocationResolver, Logger, ManifestLoader, MpiExtractor,
        XdeltaManager,
    },
};

#[derive(Parser)]
#[command(name = "mpi_installer")]
#[command(about = "MPI Package Installer for Linux (TTW, Oblivion Decompressor, etc.)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install from an MPI package
    Install {
        /// Path to MPI file or extracted directory
        #[arg(short, long)]
        mpi: PathBuf,

        /// Fallout 3 installation directory
        #[arg(long)]
        fo3: Option<PathBuf>,

        /// Fallout New Vegas installation directory
        #[arg(long)]
        fnv: Option<PathBuf>,

        /// Oblivion installation directory
        #[arg(long)]
        oblivion: Option<PathBuf>,

        /// Destination directory for installation output
        #[arg(short, long)]
        dest: PathBuf,

        /// Perform a dry run (don't actually write files)
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },

    /// Extract an MPI package to a directory
    Extract {
        /// Path to MPI file
        #[arg(short, long)]
        mpi: PathBuf,

        /// Output directory
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Inspect an MPI package
    Inspect {
        /// Path to MPI file or extracted directory
        #[arg(short, long)]
        mpi: PathBuf,

        /// Show detailed asset information
        #[arg(long, default_value = "false")]
        verbose: bool,
    },

    /// Verify game installations
    Verify {
        /// Fallout 3 installation directory
        #[arg(long)]
        fo3: Option<PathBuf>,

        /// Fallout New Vegas installation directory
        #[arg(long)]
        fnv: Option<PathBuf>,

        /// Oblivion installation directory
        #[arg(long)]
        oblivion: Option<PathBuf>,
    },

    /// List recent installation logs
    Logs {
        /// Number of recent logs to show
        #[arg(short, long, default_value = "10")]
        count: usize,
    },
}

fn main() -> Result<()> {
    // Set up panic handler to log crashes
    std::panic::set_hook(Box::new(|panic_info| {
        let crash_log = std::env::current_exe()
            .map(|p| {
                p.parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("crash.log")
            })
            .unwrap_or_else(|_| PathBuf::from("crash.log"));

        let message = format!(
            "MPI Installer crashed!\n{}\n\nBacktrace:\n{:?}\n\n",
            panic_info,
            std::backtrace::Backtrace::capture()
        );

        // Try to write to crash log
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log)
        {
            use std::io::Write;
            let _ = file.write_all(message.as_bytes());
        }

        // Also print to stderr
        eprintln!("{}", message);
    }));

    let cli = Cli::parse();

    match cli.command {
        Commands::Install {
            mpi,
            fo3,
            fnv,
            oblivion,
            dest,
            dry_run,
        } => run_install(
            &mpi,
            fo3.as_deref(),
            fnv.as_deref(),
            oblivion.as_deref(),
            &dest,
            dry_run,
        ),
        Commands::Extract { mpi, output } => run_extract(&mpi, &output),
        Commands::Inspect { mpi, verbose } => run_inspect(&mpi, verbose),
        Commands::Verify { fo3, fnv, oblivion } => {
            run_verify(fo3.as_deref(), fnv.as_deref(), oblivion.as_deref())
        }
        Commands::Logs { count } => run_logs(count),
    }
}

fn run_install(
    mpi_path: &Path,
    fo3: Option<&Path>,
    fnv: Option<&Path>,
    oblivion: Option<&Path>,
    dest: &Path,
    dry_run: bool,
) -> Result<()> {
    let install_start = Instant::now();
    println!("=== MPI Linux Installer ===\n");

    // Validate provided paths
    if let Some(fo3_path) = fo3 {
        if !fo3_path.exists() {
            bail!("Fallout 3 directory not found: {}", fo3_path.display());
        }
    }
    if let Some(fnv_path) = fnv {
        if !fnv_path.exists() {
            bail!(
                "Fallout New Vegas directory not found: {}",
                fnv_path.display()
            );
        }
    }
    if let Some(oblivion_path) = oblivion {
        if !oblivion_path.exists() {
            bail!("Oblivion directory not found: {}", oblivion_path.display());
        }
    }

    // Handle MPI extraction if needed
    let (mpi_dir, cleanup_needed) = if MpiExtractor::is_mpi_file(mpi_path) {
        println!("Extracting MPI package...");
        let extract_dir = dest.join(".mpi_package");
        let extracted = MpiExtractor::extract_to(mpi_path, &extract_dir)?;
        (extracted, true)
    } else if mpi_path.is_dir() {
        (mpi_path.to_path_buf(), false)
    } else {
        bail!("Invalid MPI path: {}", mpi_path.display());
    };

    // Find manifest
    let manifest_path = find_manifest(&mpi_dir)?;
    println!("\nLoading manifest: {}", manifest_path.display());

    // Load manifest
    let manifest = ManifestLoader::load_from_file(&manifest_path)?;

    // Get package name for logging
    let package_name = manifest
        .package
        .as_ref()
        .and_then(|p| p.title.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let package_version = manifest
        .package
        .as_ref()
        .and_then(|p| p.version.clone())
        .unwrap_or_else(|| "?".to_string());

    // Initialize logging with package name
    let logger = Logger::init(&package_name)?;

    info!("Package: {} v{}", package_name, package_version);
    info!("MPI path: {}", mpi_path.display());
    info!("Destination: {}", dest.display());
    if let Some(p) = fo3 {
        info!("Fallout 3: {}", p.display());
    }
    if let Some(p) = fnv {
        info!("Fallout NV: {}", p.display());
    }
    if let Some(p) = oblivion {
        info!("Oblivion: {}", p.display());
    }
    if dry_run {
        warn!("DRY RUN MODE - No files will be written");
    }

    // Parse assets
    let assets = ManifestLoader::parse_assets(&manifest)?;
    info!("Parsed {} assets", assets.len());

    // Get locations (profile 0 for Linux)
    let locations = ManifestLoader::get_locations(&manifest, 0)?;
    info!("Loaded {} locations", locations.len());

    // Get BSA targets from the best profile (may be Profile 1 with proper flags)
    let bsa_targets = ManifestLoader::get_bsa_target_locations(&manifest)?;
    info!("Found {} BSA targets", bsa_targets.len());

    // Get variables from manifest (profile 0 for Linux)
    let variables = ManifestLoader::get_variables(&manifest, 0).unwrap_or_default();

    // Create config with provided paths (empty string if not provided)
    let destination_path = dest.to_string_lossy().to_string();
    let config = InstallConfig {
        fallout3_root: fo3
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        falloutnv_root: fnv
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        oblivion_root: oblivion
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        destination_path: destination_path.clone(),
        mpi_package_path: mpi_dir.to_string_lossy().to_string(),
    };

    // Create location resolver with manifest variables
    let resolver = LocationResolver::new(locations.clone(), config).with_variables(&variables);
    resolver.print_locations();

    // Run pre-installation checks (hash verification, file existence, etc.)
    let checks = ManifestLoader::get_checks(&manifest);
    if !checks.is_empty() {
        info!("Running {} pre-installation checks...", checks.len());
        let verifier = FileVerifier::new(&resolver);
        let verification_result = verifier.run_checks(&checks)?;

        if !verification_result.is_success() {
            error!("Pre-installation verification failed!");
            error!("{} checks failed:", verification_result.failed);
            for err in &verification_result.errors {
                error!("  - {}", err);
            }
            bail!(
                "Installation aborted: {} verification checks failed. \
                Please ensure you have valid, unmodified game files.",
                verification_result.failed
            );
        }
        info!("All {} checks passed", verification_result.passed);
    }

    // Ensure xdelta3 is available
    info!("Checking xdelta3...");
    let xdelta = XdeltaManager::ensure_available(dest.to_path_buf())?;
    info!("xdelta3: {}", xdelta.path().display());

    // Create asset processor
    let processor = AssetProcessor::new(
        resolver,
        xdelta,
        mpi_dir.to_path_buf(),
        dest.to_path_buf(),
        &locations,
        &bsa_targets,
    )?
    .with_dry_run(dry_run);

    // Create destination directory
    if !dry_run {
        std::fs::create_dir_all(dest)?;
    }

    // Process assets using streaming mode (parallel BSA processing, minimal RAM)
    info!("=== Processing Assets (Streaming Mode) ===");
    let stats = processor.process_assets_streaming(&assets)?;

    info!(
        "Processing complete: {} success, {} failed",
        stats.success, stats.failed
    );
    if !stats.errors.is_empty() {
        println!("\n=== Errors ({}) ===", stats.errors.len());
        let show_count = std::cmp::min(10, stats.errors.len());
        for err in stats.errors.iter().take(show_count) {
            println!("  {}", err);
            error!("Asset error: {}", err);
        }
        if stats.errors.len() > show_count {
            println!(
                "  ... and {} more errors (see log file for full list)",
                stats.errors.len() - show_count
            );
        }
    }

    // Write BSA archives
    let (bsa_success, bsa_fail) = processor.finalize_bsas()?;
    info!("BSA archives: {} created, {} failed", bsa_success, bsa_fail);

    // Execute post-installation commands (renames, etc.)
    let mut post_success = 0usize;
    let mut post_fail = 0usize;
    let post_commands = ManifestLoader::get_post_commands(&manifest);
    if !post_commands.is_empty() {
        println!("\n=== Executing Post-Installation Commands ===");
        (post_success, post_fail) =
            ManifestLoader::execute_post_commands(&post_commands, &destination_path)?;
        info!(
            "Post-commands: {} success, {} failed",
            post_success, post_fail
        );
    }

    // Log summary
    logger.log_summary(stats.success, stats.failed, bsa_success, bsa_fail);

    let missing_patch_errors = stats
        .errors
        .iter()
        .filter(|e| e.contains("Patch file not found"))
        .count();

    let mut failure_reasons: Vec<String> = Vec::new();
    if stats.failed > 0 {
        failure_reasons.push(format!("{} asset operations failed", stats.failed));
    }
    if bsa_fail > 0 {
        failure_reasons.push(format!("{} BSA archives failed to write", bsa_fail));
    }
    if post_fail > 0 {
        failure_reasons.push(format!("{} post-install commands failed", post_fail));
    }

    let failure_message = if failure_reasons.is_empty() {
        None
    } else if missing_patch_errors > 0 {
        Some(format!(
            "{}. Detected {} missing patch files (.xd3). This usually means the extracted MPI package is incomplete or mismatched for this manifest.",
            failure_reasons.join("; "),
            missing_patch_errors
        ))
    } else {
        Some(failure_reasons.join("; "))
    };

    // Cleanup
    if cleanup_needed {
        MpiExtractor::cleanup_temp(&mpi_dir)?;
    }

    if let Some(message) = failure_message {
        bail!("Installation failed: {}", message);
    }

    let elapsed = install_start.elapsed();
    let minutes = elapsed.as_secs() / 60;
    let seconds = elapsed.as_secs() % 60;

    println!("\n=== Installation Complete ===");
    println!("Total time: {}m {}s", minutes, seconds);
    println!("Log saved to: {}", logger.log_path().display());
    Ok(())
}

fn run_extract(mpi_path: &Path, output: &Path) -> Result<()> {
    println!("=== TTW MPI Extractor ===\n");

    if !MpiExtractor::is_mpi_file(mpi_path) {
        bail!("Not an MPI file: {}", mpi_path.display());
    }

    // Extract to output directory
    let extracted = MpiExtractor::extract_to_temp(mpi_path)?;

    // Move to desired location
    if output.exists() {
        bail!("Output directory already exists: {}", output.display());
    }

    if std::fs::rename(&extracted, output).is_err() {
        // Cross-device move, need to copy
        copy_dir_recursive(&extracted, output)?;
        std::fs::remove_dir_all(&extracted)?;
    }

    println!("\nExtracted to: {}", output.display());
    Ok(())
}

fn run_inspect(mpi_path: &Path, verbose: bool) -> Result<()> {
    println!("=== TTW MPI Inspector ===\n");

    // Handle MPI extraction if needed
    let (mpi_dir, cleanup_needed) = if MpiExtractor::is_mpi_file(mpi_path) {
        let extracted = MpiExtractor::extract_to_temp(mpi_path)?;
        (extracted, true)
    } else if mpi_path.is_dir() {
        (mpi_path.to_path_buf(), false)
    } else {
        bail!("Invalid MPI path: {}", mpi_path.display());
    };

    // Find and load manifest
    let manifest_path = find_manifest(&mpi_dir)?;
    let manifest = ManifestLoader::load_from_file(&manifest_path)?;

    // Print package info
    if let Some(pkg) = &manifest.package {
        println!("\nPackage Information:");
        println!("  Title: {}", pkg.title.as_deref().unwrap_or("Unknown"));
        println!("  Version: {}", pkg.version.as_deref().unwrap_or("?"));
        println!("  Author: {}", pkg.author.as_deref().unwrap_or("Unknown"));
    }

    // Parse and summarize assets
    let assets = ManifestLoader::parse_assets(&manifest)?;

    if verbose {
        println!("\nAssets (first 20):");
        for (i, asset) in assets.iter().take(20).enumerate() {
            println!(
                "  [{}] OpType:{} {} -> loc:{}",
                i, asset.op_type, asset.source_path, asset.target_loc
            );
        }
        if assets.len() > 20 {
            println!("  ... and {} more assets", assets.len() - 20);
        }
    }

    // Cleanup
    if cleanup_needed {
        MpiExtractor::cleanup_temp(&mpi_dir)?;
    }

    Ok(())
}

fn run_verify(fo3: Option<&Path>, fnv: Option<&Path>, oblivion: Option<&Path>) -> Result<()> {
    println!("=== Game Installation Verifier ===\n");

    let mut all_valid = true;

    if let Some(fo3_path) = fo3 {
        println!("Checking Fallout 3: {}", fo3_path.display());
        if verify_fo3_install(fo3_path) {
            println!("  [OK] Valid Fallout 3 installation");
        } else {
            println!("  [FAIL] Invalid or incomplete Fallout 3 installation");
            all_valid = false;
        }
    }

    if let Some(fnv_path) = fnv {
        println!("Checking Fallout New Vegas: {}", fnv_path.display());
        if verify_fnv_install(fnv_path) {
            println!("  [OK] Valid Fallout New Vegas installation");
        } else {
            println!("  [FAIL] Invalid or incomplete Fallout New Vegas installation");
            all_valid = false;
        }
    }

    if let Some(oblivion_path) = oblivion {
        println!("Checking Oblivion: {}", oblivion_path.display());
        if verify_oblivion_install(oblivion_path) {
            println!("  [OK] Valid Oblivion installation");
        } else {
            println!("  [FAIL] Invalid or incomplete Oblivion installation");
            all_valid = false;
        }
    }

    if all_valid {
        println!("\nAll specified installations are valid!");
        Ok(())
    } else {
        bail!("Some installations are invalid")
    }
}

fn find_manifest(mpi_dir: &Path) -> Result<PathBuf> {
    // Look for common manifest locations
    let candidates = [
        "_package/index.json", // TTW MPI format
        "manifest.json",
        "Manifest.json",
        "TTW.manifest.json",
        "ttw.manifest.json",
        "index.json",
    ];

    for name in candidates {
        let path = mpi_dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }

    // Search recursively
    for entry in walkdir::WalkDir::new(mpi_dir).max_depth(3) {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if (name.contains("manifest") || name == "index.json")
            && entry
                .path()
                .extension()
                .map(|e| e == "json")
                .unwrap_or(false)
        {
            return Ok(entry.path().to_path_buf());
        }
    }

    bail!("Manifest not found in: {}", mpi_dir.display())
}

fn verify_fo3_install(path: &Path) -> bool {
    // Check for essential Fallout 3 files
    let required = [
        "Data/Fallout3.esm",
        "Data/Fallout - Meshes.bsa",
        "Data/Fallout - Textures.bsa",
        "Data/Fallout - Voices.bsa",
    ];

    for file in required {
        let file_path = path.join(file);
        if !file_path.exists() {
            // Try lowercase
            let lower_path = path.join(file.to_lowercase());
            if !lower_path.exists() {
                return false;
            }
        }
    }

    true
}

fn verify_fnv_install(path: &Path) -> bool {
    // Check for essential FNV files
    let required = [
        "Data/FalloutNV.esm",
        "Data/Fallout - Meshes.bsa",
        "Data/Fallout - Textures.bsa",
        "Data/Fallout - Voices1.bsa",
    ];

    for file in required {
        let file_path = path.join(file);
        if !file_path.exists() {
            // Try lowercase
            let lower_path = path.join(file.to_lowercase());
            if !lower_path.exists() {
                return false;
            }
        }
    }

    true
}

fn verify_oblivion_install(path: &Path) -> bool {
    // Check for essential Oblivion files
    let required = [
        "Data/Oblivion.esm",
        "Data/Oblivion - Meshes.bsa",
        "Data/Oblivion - Textures - Compressed.bsa",
    ];

    for file in required {
        let file_path = path.join(file);
        if !file_path.exists() {
            // Try lowercase
            let lower_path = path.join(file.to_lowercase());
            if !lower_path.exists() {
                return false;
            }
        }
    }

    true
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}

fn run_logs(count: usize) -> Result<()> {
    println!("=== Recent Installation Logs ===\n");

    let logs = Logger::list_recent_logs(count)?;

    if logs.is_empty() {
        println!("No log files found.");
        println!("\nLogs are stored in: ~/.local/share/mpi_installer/logs/");
        return Ok(());
    }

    for (i, log_path) in logs.iter().enumerate() {
        let filename = log_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "?".to_string());

        // Get file size
        let size = std::fs::metadata(log_path)
            .map(|m| format_size(m.len()))
            .unwrap_or_else(|_| "?".to_string());

        println!("  [{}] {} ({})", i + 1, filename, size);
    }

    println!("\nTo view a log: cat ~/.local/share/mpi_installer/logs/<filename>");
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
