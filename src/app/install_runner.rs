use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::InstallEvent;
use crate::models::{Check, InstallConfig};
use crate::services::{
    compute_md5, compute_sha1, AssetProcessor, FileVerifier, GameDetection, LocationResolver,
    ManifestLoader, MpiExtractor, MpiStore, XdeltaManager,
};

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub mpi_path: PathBuf,
    pub fallout3_path: Option<PathBuf>,
    pub falloutnv_path: Option<PathBuf>,
    pub oblivion_path: Option<PathBuf>,
    pub destination_path: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub package_name: String,
    pub package_version: String,
    pub assets_success: usize,
    pub assets_failed: usize,
    pub bsa_success: usize,
    pub bsa_failed: usize,
    pub post_success: usize,
    pub post_failed: usize,
    pub elapsed: Duration,
}

pub fn run_install<F>(request: InstallRequest, emit: F) -> Result<InstallReport>
where
    F: Fn(InstallEvent) + Sync,
{
    let install_start = Instant::now();

    log_requested_paths(&request, &emit);
    validate_request(&request)?;
    let game_paths = resolve_game_paths(&request, &emit);
    log_resolved_game_paths(&game_paths, &emit);

    emit(InstallEvent::log("Loading MPI package..."));
    emit(InstallEvent::progress(0, 10_000, "Preparing MPI package"));
    let (mpi_dir, cleanup_needed, mpi_store) = load_mpi_package(&request, &emit)?;
    emit(InstallEvent::progress(500, 10_000, "MPI package ready"));

    let manifest_path = find_manifest(&mpi_dir)?;
    emit(InstallEvent::log(format!(
        "Loading manifest: {}",
        manifest_path.display()
    )));
    let manifest = ManifestLoader::load_from_file(&manifest_path)?;

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
    emit(InstallEvent::log(format!(
        "Package: {} v{}",
        package_name, package_version
    )));

    let assets = ManifestLoader::parse_assets(&manifest)?;
    emit(InstallEvent::log(format!("Parsed {} assets", assets.len())));

    let profile_index = ManifestLoader::select_best_profile(&manifest);
    emit(InstallEvent::log(format!(
        "Using profile {}",
        profile_index
    )));

    let locations = ManifestLoader::get_locations(&manifest, profile_index)?;
    let bsa_targets = ManifestLoader::get_bsa_target_locations(&manifest)?;
    emit(InstallEvent::log(format!(
        "Found {} BSA targets",
        bsa_targets.len()
    )));

    let variables = ManifestLoader::get_variables(&manifest, profile_index)
        .or_else(|_| ManifestLoader::get_variables(&manifest, 0))
        .unwrap_or_default();
    emit(InstallEvent::progress(1_000, 10_000, "Manifest loaded"));

    let destination = request.destination_path.to_string_lossy().to_string();
    let config = InstallConfig {
        fallout3_root: game_paths
            .fallout3
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        falloutnv_root: game_paths
            .falloutnv
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        oblivion_root: game_paths
            .oblivion
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        destination_path: destination.clone(),
        mpi_package_path: mpi_dir.to_string_lossy().to_string(),
    };

    let resolver = LocationResolver::new(locations.clone(), config).with_variables(&variables);

    let checks = ManifestLoader::get_checks(&manifest);
    if !checks.is_empty() {
        log_check_file_diagnostics(&FileVerifier::new(&resolver), &checks, &emit);
        emit(InstallEvent::log(format!(
            "Running {} pre-installation checks...",
            checks.len()
        )));
        let verifier = FileVerifier::new(&resolver);
        let verification_result = verifier.run_checks(&checks)?;

        if !verification_result.is_success() {
            for err in &verification_result.errors {
                emit(InstallEvent::log(format!("CHECK FAILED: {}", err)));
            }
            bail!(
                "Verification failed: {} checks failed. Please ensure you have valid, unmodified game files.",
                verification_result.failed
            );
        }
        emit(InstallEvent::log(format!(
            "All {} checks passed",
            verification_result.passed
        )));
    }
    emit(InstallEvent::progress(1_200, 10_000, "Checks complete"));

    emit(InstallEvent::log("Checking xdelta3..."));
    let xdelta = XdeltaManager::ensure_available(request.destination_path.clone())?;

    let mut processor = AssetProcessor::new(
        resolver,
        xdelta,
        mpi_dir.clone(),
        request.destination_path.clone(),
        &locations,
        &bsa_targets,
    )?
    .with_dry_run(request.dry_run);

    if let Some(store) = mpi_store {
        processor = processor.with_mpi_store(store);
    }

    if !request.dry_run {
        std::fs::create_dir_all(&request.destination_path)?;
    }

    emit(InstallEvent::log("Processing assets (streaming mode)..."));
    let stats =
        processor.process_assets_streaming_with_callback(&assets, |current, total, msg| {
            let pct = 1_000 + ((current as u32 * 7_000) / total.max(1) as u32);
            emit(InstallEvent::progress(pct, 10_000, msg));

            if current.is_multiple_of(1000) || current == total {
                emit(InstallEvent::log(format!(
                    "Assets: {}/{} - {}",
                    current, total, msg
                )));
            }
        })?;

    emit(InstallEvent::log(format!(
        "Processed: {} success, {} failed",
        stats.success, stats.failed
    )));
    for err in &stats.errors {
        emit(InstallEvent::log(format!("Asset error: {}", err)));
    }
    emit(InstallEvent::progress(8_000, 10_000, "Assets processed"));

    let (bsa_success, bsa_failed) = if stats.bsa_success > 0 || stats.bsa_failed > 0 {
        emit(InstallEvent::log(format!(
            "BSA archives built during asset processing: {} created, {} failed",
            stats.bsa_success, stats.bsa_failed
        )));
        (stats.bsa_success, stats.bsa_failed)
    } else {
        emit(InstallEvent::log("Finalizing BSA archives..."));
        processor.finalize_bsas_with_callback(|current, total, bsa_name| {
            let pct = 8_000 + ((current as u32 * 1_500) / total.max(1) as u32);
            emit(InstallEvent::progress(pct, 10_000, bsa_name));
            emit(InstallEvent::log(format!(
                "Writing BSA {}/{}: {}",
                current, total, bsa_name
            )));
        })?
    };

    emit(InstallEvent::log(format!(
        "BSA archives: {} created, {} failed",
        bsa_success, bsa_failed
    )));
    emit(InstallEvent::progress(
        9_000,
        10_000,
        "BSA archives written",
    ));

    let post_commands = ManifestLoader::get_post_commands(&manifest);
    let (post_success, post_failed) = if post_commands.is_empty() {
        (0, 0)
    } else {
        emit(InstallEvent::log("Executing post-installation commands..."));
        let result = ManifestLoader::execute_post_commands(&post_commands, &destination)?;
        emit(InstallEvent::log(format!(
            "Post-commands: {} success, {} failed",
            result.0, result.1
        )));
        result
    };
    emit(InstallEvent::progress(
        9_500,
        10_000,
        "Post-install commands complete",
    ));

    let missing_patch_errors = stats
        .errors
        .iter()
        .filter(|e| e.contains("Patch file not found"))
        .count();
    let failure_message =
        failure_message(stats.failed, bsa_failed, post_failed, missing_patch_errors);

    if cleanup_needed {
        emit(InstallEvent::log("Cleaning up..."));
        MpiExtractor::cleanup_temp(&mpi_dir)?;
    }

    if let Some(message) = failure_message {
        bail!("Installation failed: {}", message);
    }

    emit(InstallEvent::progress(
        10_000,
        10_000,
        "Installation complete",
    ));

    Ok(InstallReport {
        package_name,
        package_version,
        assets_success: stats.success,
        assets_failed: stats.failed,
        bsa_success,
        bsa_failed,
        post_success,
        post_failed,
        elapsed: install_start.elapsed(),
    })
}

fn validate_request(request: &InstallRequest) -> Result<()> {
    if let Some(path) = &request.fallout3_path {
        if !path.exists() {
            bail!("Fallout 3 directory not found: {}", path.display());
        }
    }
    if let Some(path) = &request.falloutnv_path {
        if !path.exists() {
            bail!("Fallout New Vegas directory not found: {}", path.display());
        }
    }
    if let Some(path) = &request.oblivion_path {
        if !path.exists() {
            bail!("Oblivion directory not found: {}", path.display());
        }
    }
    Ok(())
}

fn log_requested_paths<F>(request: &InstallRequest, emit: &F)
where
    F: Fn(InstallEvent) + Sync,
{
    emit(InstallEvent::log("Install request paths:"));
    emit(InstallEvent::log(format!(
        "  MPI package: {}",
        request.mpi_path.display()
    )));
    emit(InstallEvent::log(format!(
        "  Fallout 3: {}",
        optional_path(request.fallout3_path.as_ref())
    )));
    emit(InstallEvent::log(format!(
        "  Fallout New Vegas: {}",
        optional_path(request.falloutnv_path.as_ref())
    )));
    emit(InstallEvent::log(format!(
        "  Oblivion: {}",
        optional_path(request.oblivion_path.as_ref())
    )));
    emit(InstallEvent::log(format!(
        "  Output: {}",
        request.destination_path.display()
    )));
}

fn log_resolved_game_paths<F>(paths: &ResolvedGamePaths, emit: &F)
where
    F: Fn(InstallEvent) + Sync,
{
    emit(InstallEvent::log("Resolved game paths:"));
    emit(InstallEvent::log(format!(
        "  Fallout 3: {}",
        optional_path(paths.fallout3.as_ref())
    )));
    emit(InstallEvent::log(format!(
        "  Fallout New Vegas: {}",
        optional_path(paths.falloutnv.as_ref())
    )));
    emit(InstallEvent::log(format!(
        "  Oblivion: {}",
        optional_path(paths.oblivion.as_ref())
    )));
}

fn log_check_file_diagnostics<F>(verifier: &FileVerifier<'_>, checks: &[Check], emit: &F)
where
    F: Fn(InstallEvent) + Sync,
{
    let file_checks: Vec<(usize, &Check)> = checks
        .iter()
        .enumerate()
        .filter(|(_, check)| check.check_type == 0 && check.file.is_some())
        .collect();

    if file_checks.is_empty() {
        return;
    }

    emit(InstallEvent::log("Pre-check file diagnostics:"));
    for (index, check) in file_checks {
        match verifier.check_file_path(check) {
            Ok(path) => {
                let metadata = fs::metadata(&path);
                match metadata {
                    Ok(metadata) => emit(InstallEvent::log(format!(
                        "  [{}] {} exists, size {}",
                        index + 1,
                        path.display(),
                        format_bytes(metadata.len())
                    ))),
                    Err(err) => emit(InstallEvent::log(format!(
                        "  [{}] {} missing/unreadable: {}",
                        index + 1,
                        path.display(),
                        err
                    ))),
                }

                if check
                    .checksums
                    .as_ref()
                    .is_some_and(|checksums| !checksums.trim().is_empty())
                {
                    match fs::read(&path) {
                        Ok(data) => emit(InstallEvent::log(format!(
                            "  [{}] hashes: sha1={}, md5={}",
                            index + 1,
                            compute_sha1(&data),
                            compute_md5(&data)
                        ))),
                        Err(err) => emit(InstallEvent::log(format!(
                            "  [{}] hashes unavailable: {}",
                            index + 1,
                            err
                        ))),
                    }
                }
            }
            Err(err) => emit(InstallEvent::log(format!(
                "  [{}] could not resolve check file path: {}",
                index + 1,
                err
            ))),
        }
    }
}

fn optional_path(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "<not provided>".to_string())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", value, UNITS[unit])
    }
}

#[derive(Debug, Clone, Default)]
struct ResolvedGamePaths {
    fallout3: Option<PathBuf>,
    falloutnv: Option<PathBuf>,
    oblivion: Option<PathBuf>,
}

fn resolve_game_paths<F>(request: &InstallRequest, emit: &F) -> ResolvedGamePaths
where
    F: Fn(InstallEvent) + Sync,
{
    let mut paths = ResolvedGamePaths {
        fallout3: request.fallout3_path.clone(),
        falloutnv: request.falloutnv_path.clone(),
        oblivion: request.oblivion_path.clone(),
    };

    if paths.fallout3.is_some() && paths.falloutnv.is_some() && paths.oblivion.is_some() {
        return paths;
    }

    emit(InstallEvent::log(
        "Auto-detecting missing game installation paths...",
    ));
    let detected = GameDetection::detect();

    if paths.fallout3.is_none() {
        if let Some(game) = detected.fallout3 {
            emit(InstallEvent::log(format!(
                "Detected {} via {}: {}",
                game.kind.display_name(),
                game.source,
                game.path.display()
            )));
            paths.fallout3 = Some(game.path);
        }
    }
    if paths.falloutnv.is_none() {
        if let Some(game) = detected.falloutnv {
            emit(InstallEvent::log(format!(
                "Detected {} via {}: {}",
                game.kind.display_name(),
                game.source,
                game.path.display()
            )));
            paths.falloutnv = Some(game.path);
        }
    }
    if paths.oblivion.is_none() {
        if let Some(game) = detected.oblivion {
            emit(InstallEvent::log(format!(
                "Detected {} via {}: {}",
                game.kind.display_name(),
                game.source,
                game.path.display()
            )));
            paths.oblivion = Some(game.path);
        }
    }

    paths
}

fn load_mpi_package<F>(
    request: &InstallRequest,
    emit: &F,
) -> Result<(PathBuf, bool, Option<MpiStore>)>
where
    F: Fn(InstallEvent) + Sync,
{
    if MpiExtractor::is_mpi_file(&request.mpi_path) {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let available_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
        let use_inmemory = available_gb >= 10.0;
        let extract_dir = mpi_work_dir(request);

        if use_inmemory {
            let store = MpiStore::load(&request.mpi_path)?;
            std::fs::create_dir_all(&extract_dir)?;
            let manifest_dir = extract_dir.join("_package");
            std::fs::create_dir_all(&manifest_dir)?;

            if let Some(manifest_data) = store.get_manifest() {
                std::fs::write(manifest_dir.join("index.json"), manifest_data)?;
            } else {
                bail!("No manifest found in MPI package");
            }

            Ok((extract_dir, true, Some(store)))
        } else {
            emit(InstallEvent::log(format!(
                "System has {:.1} GB available RAM, using disk extraction (need 10+ GB for in-memory mode)",
                available_gb
            )));
            let extracted = MpiExtractor::extract_to(&request.mpi_path, &extract_dir)?;
            Ok((extracted, true, None))
        }
    } else if request.mpi_path.is_dir() {
        Ok((request.mpi_path.clone(), false, None))
    } else {
        bail!("Invalid MPI path: {}", request.mpi_path.display());
    }
}

fn mpi_work_dir(request: &InstallRequest) -> PathBuf {
    if request.dry_run {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ttw_mpi_package_{}_{}",
            std::process::id(),
            timestamp
        ))
    } else {
        request.destination_path.join(".mpi_package")
    }
}

pub fn find_manifest(mpi_dir: impl AsRef<std::path::Path>) -> Result<PathBuf> {
    let mpi_dir = mpi_dir.as_ref();
    let candidates = [
        "_package/index.json",
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

fn failure_message(
    asset_failed: usize,
    bsa_failed: usize,
    post_failed: usize,
    missing_patch_errors: usize,
) -> Option<String> {
    let mut failure_reasons = Vec::new();
    if asset_failed > 0 {
        failure_reasons.push(format!("{} asset operations failed", asset_failed));
    }
    if bsa_failed > 0 {
        failure_reasons.push(format!("{} BSA archives failed to write", bsa_failed));
    }
    if post_failed > 0 {
        failure_reasons.push(format!("{} post-install commands failed", post_failed));
    }

    if failure_reasons.is_empty() {
        None
    } else if missing_patch_errors > 0 {
        Some(format!(
            "{}. Detected {} missing patch files (.xd3). This usually means the extracted MPI package is incomplete or mismatched for this manifest.",
            failure_reasons.join("; "),
            missing_patch_errors
        ))
    } else {
        Some(failure_reasons.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::{failure_message, mpi_work_dir, InstallRequest};
    use std::path::PathBuf;

    fn request(dry_run: bool) -> InstallRequest {
        InstallRequest {
            mpi_path: PathBuf::from("/tmp/package.mpi"),
            fallout3_path: None,
            falloutnv_path: None,
            oblivion_path: None,
            destination_path: PathBuf::from("/tmp/ttw-output"),
            dry_run,
        }
    }

    #[test]
    fn dry_run_mpi_work_dir_uses_temp_dir() {
        let dir = mpi_work_dir(&request(true));
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ttw_mpi_"));
    }

    #[test]
    fn real_install_mpi_work_dir_uses_destination_package_dir() {
        assert_eq!(
            mpi_work_dir(&request(false)),
            PathBuf::from("/tmp/ttw-output/.mpi_package")
        );
    }

    #[test]
    fn failure_message_mentions_missing_patches() {
        let message = failure_message(2, 0, 0, 2).unwrap();
        assert!(message.contains("2 asset operations failed"));
        assert!(message.contains("missing patch files"));
    }
}
