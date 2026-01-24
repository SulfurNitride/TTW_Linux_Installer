use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::fs;
use std::io::Write;
use serde::{Deserialize, Serialize};
use chrono::Local;

use ttw_installer::{
    models::InstallConfig,
    services::{
        MpiExtractor, ManifestLoader, LocationResolver,
        AssetProcessor, XdeltaManager, FileVerifier,
        BsaDecompressor, DecompressGame,
    },
};

/// User configuration that persists between sessions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UserConfig {
    fo3_path: String,
    fnv_path: String,
    oblivion_path: String,
    output_path: String,
}

impl UserConfig {
    /// Get config file path (uses platform-appropriate config directory)
    fn config_path() -> PathBuf {
        // Linux: ~/.config/mpi_installer/config.json
        // Windows: %APPDATA%\mpi_installer\config.json
        if let Some(config_dir) = dirs::config_dir() {
            let app_config_dir = config_dir.join("mpi_installer");
            // Ensure the directory exists
            let _ = fs::create_dir_all(&app_config_dir);
            return app_config_dir.join("config.json");
        }

        // Fallback: next to executable (for portable mode)
        std::env::current_exe()
            .map(|p| p.parent().unwrap_or(std::path::Path::new(".")).join("mpi_installer.json"))
            .unwrap_or_else(|_| PathBuf::from("mpi_installer.json"))
    }

    /// Load config from file
    fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str(&data) {
                    return config;
                }
            }
        }
        Self::default()
    }

    /// Save config to file
    fn save(&self) {
        let path = Self::config_path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&path, data);
        }
    }
}

/// Get log file path (uses platform-appropriate data directory)
fn get_log_path(package_name: &str) -> PathBuf {
    // Linux: ~/.local/share/mpi_installer/logs/
    // Windows: %LOCALAPPDATA%\mpi_installer\logs\
    let logs_dir = if let Some(data_dir) = dirs::data_local_dir() {
        data_dir.join("mpi_installer").join("logs")
    } else {
        // Fallback: next to executable
        std::env::current_exe()
            .map(|p| p.parent().unwrap_or(std::path::Path::new(".")).join("logs"))
            .unwrap_or_else(|_| PathBuf::from("logs"))
    };

    let _ = fs::create_dir_all(&logs_dir);

    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let safe_name: String = package_name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();

    logs_dir.join(format!("{}_{}.log", timestamp, safe_name))
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MPI Installer",
        options,
        Box::new(|cc| {
            // Set dark theme
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(MpiInstallerApp::default()))
        }),
    )
}

#[derive(Debug, Clone, PartialEq)]
enum InstallStatus {
    Ready,
    Running,
    Complete,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Installer,
    Decompressor,
}

struct MpiInstallerApp {
    // Active tab
    active_tab: ActiveTab,

    // Path inputs (Installer)
    fo3_path: String,
    fnv_path: String,
    oblivion_path: String,
    mpi_path: String,
    output_path: String,

    // Decompressor settings
    decompress_game: usize,  // 0=FO3, 1=FNV, 2=Oblivion
    decompress_path: String,
    decompress_output: String,
    decompress_backup: bool,

    // State
    status: InstallStatus,
    progress: Arc<AtomicU32>,  // Progress as percentage * 100 (0-10000 for 0.00%-100.00%)
    log_messages: Arc<Mutex<Vec<String>>>,

    // Installation/Decompression thread handle
    install_thread: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Default for MpiInstallerApp {
    fn default() -> Self {
        // Load saved config
        let config = UserConfig::load();

        Self {
            active_tab: ActiveTab::Installer,
            fo3_path: config.fo3_path.clone(),
            fnv_path: config.fnv_path.clone(),
            oblivion_path: config.oblivion_path.clone(),
            mpi_path: String::new(), // Don't save MPI path - it changes per install
            output_path: config.output_path,
            decompress_game: 0,
            decompress_path: String::new(),
            decompress_output: String::new(),
            decompress_backup: true,
            status: InstallStatus::Ready,
            progress: Arc::new(AtomicU32::new(0)),
            log_messages: Arc::new(Mutex::new(vec!["Ready".to_string()])),
            install_thread: None,
        }
    }
}

impl eframe::App for MpiInstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check if thread finished
        if let Some(handle) = self.install_thread.take() {
            if handle.is_finished() {
                match handle.join() {
                    Ok(Ok(())) => {
                        self.status = InstallStatus::Complete;
                        self.progress.store(10000, Ordering::Relaxed);
                        self.add_log("Operation complete!");
                    }
                    Ok(Err(e)) => {
                        self.status = InstallStatus::Failed(e.clone());
                        self.add_log(&format!("Operation failed: {}", e));
                    }
                    Err(panic_info) => {
                        let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic".to_string()
                        };
                        self.status = InstallStatus::Failed(format!("Thread panicked: {}", panic_msg));
                        self.add_log(&format!("Thread panicked: {}", panic_msg));
                    }
                }
            } else {
                self.install_thread = Some(handle);
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            // Title
            ui.heading(egui::RichText::new("MPI Installer & BSA Tools").size(24.0));
            ui.add_space(5.0);

            // Tab bar
            ui.horizontal(|ui| {
                if ui.selectable_label(self.active_tab == ActiveTab::Installer, "MPI Installer").clicked() {
                    self.active_tab = ActiveTab::Installer;
                }
                ui.separator();
                if ui.selectable_label(self.active_tab == ActiveTab::Decompressor, "BSA Decompressor").clicked() {
                    self.active_tab = ActiveTab::Decompressor;
                }
            });
            ui.separator();
            ui.add_space(5.0);

            match self.active_tab {
                ActiveTab::Installer => self.show_installer_tab(ui),
                ActiveTab::Decompressor => self.show_decompressor_tab(ui),
            }

            ui.add_space(10.0);

            // Progress bar (shared)
            ui.horizontal(|ui| {
                let progress_value = self.progress.load(Ordering::Relaxed) as f32 / 10000.0;
                let progress_bar = egui::ProgressBar::new(progress_value)
                    .show_percentage();
                ui.add_sized([680.0, 20.0], progress_bar);

                let status_text = match &self.status {
                    InstallStatus::Ready => "Ready",
                    InstallStatus::Running => "Running...",
                    InstallStatus::Complete => "Complete",
                    InstallStatus::Failed(_) => "Failed",
                };
                ui.label(status_text);
            });

            ui.add_space(10.0);

            // Log output panel (shared)
            ui.group(|ui| {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let logs = self.log_messages.lock().unwrap();
                        for msg in logs.iter() {
                            ui.label(msg);
                        }
                    });
            });
        });

        // Request repaint while running
        if self.status == InstallStatus::Running {
            ctx.request_repaint();
        }
    }
}

impl MpiInstallerApp {
    fn add_log(&self, msg: &str) {
        let mut logs = self.log_messages.lock().unwrap();
        logs.push(msg.to_string());
        // Keep last 100 messages
        if logs.len() > 100 {
            logs.remove(0);
        }
    }

    /// Save current paths to config
    fn save_config(&self) {
        let config = UserConfig {
            fo3_path: self.fo3_path.clone(),
            fnv_path: self.fnv_path.clone(),
            oblivion_path: self.oblivion_path.clone(),
            output_path: self.output_path.clone(),
        };
        config.save();
    }

    fn show_installer_tab(&mut self, ui: &mut egui::Ui) {
        // Path inputs grid
        egui::Grid::new("paths_grid")
            .num_columns(3)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                // Fallout 3
                ui.label("Fallout 3:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.fo3_path)
                        .hint_text("Path to Fallout 3 installation (optional)")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.fo3_path = path;
                        self.save_config();
                    }
                }
                ui.end_row();

                // Fallout New Vegas
                ui.label("Fallout New Vegas:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.fnv_path)
                        .hint_text("Path to Fallout New Vegas installation (optional)")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.fnv_path = path;
                        self.save_config();
                    }
                }
                ui.end_row();

                // Oblivion
                ui.label("Oblivion:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.oblivion_path)
                        .hint_text("Path to Oblivion installation (optional)")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.oblivion_path = path;
                        self.save_config();
                    }
                }
                ui.end_row();

                // MPI Package
                ui.label("MPI Package:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.mpi_path)
                        .hint_text("Path to .mpi file or extracted package folder")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_mpi_file() {
                        self.mpi_path = path;
                    }
                }
                ui.end_row();

                // Output Directory
                ui.label("Output Directory:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.output_path)
                        .hint_text("Where to install output files")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.output_path = path;
                        self.save_config();
                    }
                }
                ui.end_row();
            });

        ui.add_space(10.0);

        // Start Installation button
        let can_install = self.status != InstallStatus::Running
            && !self.mpi_path.is_empty()
            && !self.output_path.is_empty();

        let button = egui::Button::new(
            egui::RichText::new("Start Installation").size(16.0)
        ).min_size(egui::vec2(760.0, 30.0));

        if ui.add_enabled(can_install, button).clicked() {
            self.start_installation();
        }
    }

    fn show_decompressor_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Decompress game BSA archives to improve mod compatibility and loading times.");
        ui.add_space(5.0);

        // TTW Warning - only show for FO3 (0) and FNV (1)
        if self.decompress_game == 0 || self.decompress_game == 1 {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(80, 40, 40))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::YELLOW, "WARNING:");
                        ui.label("Do NOT decompress BSAs if you plan to install Tale of Two Wastelands.");
                    });
                    ui.label("TTW requires the original compressed game files. Only use this for standalone modding.");
                });
            ui.add_space(5.0);
        }

        egui::Grid::new("decompress_grid")
            .num_columns(3)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                // Game selector
                ui.label("Game:");
                egui::ComboBox::from_id_salt("game_select")
                    .width(500.0)
                    .selected_text(match self.decompress_game {
                        0 => "Fallout 3",
                        1 => "Fallout New Vegas",
                        2 => "Oblivion",
                        _ => "Select game...",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.decompress_game, 0, "Fallout 3");
                        ui.selectable_value(&mut self.decompress_game, 1, "Fallout New Vegas");
                        ui.selectable_value(&mut self.decompress_game, 2, "Oblivion");
                    });
                ui.label(""); // Empty cell
                ui.end_row();

                // Data folder path
                ui.label("Data Folder:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.decompress_path)
                        .hint_text("Path to game's Data folder (e.g., .../Fallout 3/Data)")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.decompress_path = path;
                    }
                }
                ui.end_row();

                // Output folder (optional)
                ui.label("Output Folder:");
                ui.add_sized(
                    [500.0, 20.0],
                    egui::TextEdit::singleline(&mut self.decompress_output)
                        .hint_text("Leave empty to replace in-place (with backup)")
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = pick_folder() {
                        self.decompress_output = path;
                    }
                }
                ui.end_row();
            });

        ui.add_space(5.0);

        // Backup checkbox
        ui.checkbox(&mut self.decompress_backup, "Create backup of original BSA files (.bsa.backup)");

        ui.add_space(5.0);

        // Info about FNV
        if self.decompress_game == 1 {
            ui.colored_label(egui::Color32::YELLOW,
                "Note: FNV decompression also converts ambient/emitter OGG sounds to WAV format.");
        }

        ui.add_space(10.0);

        // Start Decompression button
        let can_decompress = self.status != InstallStatus::Running
            && !self.decompress_path.is_empty();

        let button = egui::Button::new(
            egui::RichText::new("Start Decompression").size(16.0)
        ).min_size(egui::vec2(760.0, 30.0));

        if ui.add_enabled(can_decompress, button).clicked() {
            self.start_decompression();
        }
    }

    fn start_decompression(&mut self) {
        self.status = InstallStatus::Running;
        self.progress.store(0, Ordering::Relaxed);

        // Clear logs
        {
            let mut logs = self.log_messages.lock().unwrap();
            logs.clear();
            logs.push("Starting BSA decompression...".to_string());
        }

        let game = match self.decompress_game {
            0 => DecompressGame::Fallout3,
            1 => DecompressGame::FalloutNV,
            2 => DecompressGame::Oblivion,
            _ => DecompressGame::Fallout3,
        };

        let data_path = PathBuf::from(&self.decompress_path);
        let output_path = if self.decompress_output.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.decompress_output))
        };
        let backup = self.decompress_backup;

        let log_messages = Arc::clone(&self.log_messages);
        let progress = Arc::clone(&self.progress);

        let handle = thread::spawn(move || {
            run_decompression(game, data_path, output_path, backup, log_messages, progress)
        });

        self.install_thread = Some(handle);
    }

    fn start_installation(&mut self) {
        self.status = InstallStatus::Running;
        self.progress.store(0, Ordering::Relaxed);

        // Clear logs
        {
            let mut logs = self.log_messages.lock().unwrap();
            logs.clear();
            logs.push("Starting installation...".to_string());
        }

        // Clone values for the thread
        let fo3_path = if self.fo3_path.is_empty() { None } else { Some(PathBuf::from(&self.fo3_path)) };
        let fnv_path = if self.fnv_path.is_empty() { None } else { Some(PathBuf::from(&self.fnv_path)) };
        let oblivion_path = if self.oblivion_path.is_empty() { None } else { Some(PathBuf::from(&self.oblivion_path)) };
        let mpi_path = PathBuf::from(&self.mpi_path);
        let output_path = PathBuf::from(&self.output_path);
        let log_messages = Arc::clone(&self.log_messages);
        let progress = Arc::clone(&self.progress);

        let handle = thread::spawn(move || {
            run_installation(
                mpi_path,
                fo3_path,
                fnv_path,
                oblivion_path,
                output_path,
                log_messages,
                progress,
            )
        });

        self.install_thread = Some(handle);
    }
}

fn pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

fn pick_mpi_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("MPI Package", &["mpi"])
        .add_filter("All Files", &["*"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

fn run_installation(
    mpi_path: PathBuf,
    fo3_path: Option<PathBuf>,
    fnv_path: Option<PathBuf>,
    oblivion_path: Option<PathBuf>,
    output_path: PathBuf,
    log_messages: Arc<Mutex<Vec<String>>>,
    progress: Arc<AtomicU32>,
) -> Result<(), String> {
    // Create log file
    let log_path = get_log_path("Installation");
    let log_file = Arc::new(Mutex::new(
        fs::File::create(&log_path).ok()
    ));

    // Helper to log messages to both UI and file
    let log_messages_clone = Arc::clone(&log_messages);
    let log_file_clone = Arc::clone(&log_file);
    let log = move |msg: &str| {
        // Add to UI
        if let Ok(mut logs) = log_messages_clone.lock() {
            logs.push(msg.to_string());
        }

        // Write to file
        if let Ok(mut file_guard) = log_file_clone.lock() {
            if let Some(ref mut file) = *file_guard {
                let timestamp = Local::now().format("%H:%M:%S");
                let _ = writeln!(file, "[{}] {}", timestamp, msg);
            }
        }
    };

    log(&format!("Log file: {}", log_path.display()));

    // Phase 1: Extract MPI (0-5%)
    progress.store(0, Ordering::Relaxed);
    let (mpi_dir, cleanup_needed) = if MpiExtractor::is_mpi_file(&mpi_path) {
        log("Extracting MPI package...");
        let extract_dir = output_path.join(".mpi_package");
        let extracted = MpiExtractor::extract_to(&mpi_path, &extract_dir)
            .map_err(|e| format!("Failed to extract MPI: {}", e))?;
        (extracted, true)
    } else if mpi_path.is_dir() {
        (mpi_path.clone(), false)
    } else {
        return Err(format!("Invalid MPI path: {}", mpi_path.display()));
    };
    progress.store(500, Ordering::Relaxed); // 5%

    // Phase 2: Load manifest (5-10%)
    let manifest_path = find_manifest(&mpi_dir)
        .map_err(|e| format!("Failed to find manifest: {}", e))?;
    log(&format!("Loading manifest: {}", manifest_path.display()));

    let manifest = ManifestLoader::load_from_file(&manifest_path)
        .map_err(|e| format!("Failed to load manifest: {}", e))?;

    if let Some(pkg) = &manifest.package {
        log(&format!("Package: {} v{}",
            pkg.title.as_deref().unwrap_or("Unknown"),
            pkg.version.as_deref().unwrap_or("?")));
    }

    let assets = ManifestLoader::parse_assets(&manifest)
        .map_err(|e| format!("Failed to parse assets: {}", e))?;
    let total_assets = assets.len();
    log(&format!("Parsed {} assets", total_assets));

    let locations = ManifestLoader::get_locations(&manifest, 0)
        .map_err(|e| format!("Failed to get locations: {}", e))?;

    let bsa_targets = ManifestLoader::get_bsa_target_locations(&manifest)
        .map_err(|e| format!("Failed to get BSA targets: {}", e))?;
    let total_bsas = bsa_targets.len();
    log(&format!("Found {} BSA targets", total_bsas));

    let variables = ManifestLoader::get_variables(&manifest, 0).unwrap_or_default();
    progress.store(1000, Ordering::Relaxed); // 10%

    // Create config and resolver
    let config = InstallConfig {
        fallout3_root: fo3_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        falloutnv_root: fnv_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        oblivion_root: oblivion_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        destination_path: output_path.to_string_lossy().to_string(),
        mpi_package_path: mpi_dir.to_string_lossy().to_string(),
    };

    let resolver = LocationResolver::new(locations.clone(), config)
        .with_variables(&variables);

    // Run pre-installation checks (hash verification, file existence, etc.)
    let checks = ManifestLoader::get_checks(&manifest);
    if !checks.is_empty() {
        log(&format!("Running {} pre-installation checks...", checks.len()));
        let verifier = FileVerifier::new(&resolver);
        let verification_result = verifier.run_checks(&checks)
            .map_err(|e| format!("Verification error: {}", e))?;

        if !verification_result.is_success() {
            for err in &verification_result.errors {
                log(&format!("CHECK FAILED: {}", err));
            }
            return Err(format!(
                "Verification failed: {} checks failed. Please ensure you have valid, unmodified game files.",
                verification_result.failed
            ));
        }
        log(&format!("All {} checks passed", verification_result.passed));
    }
    progress.store(1200, Ordering::Relaxed); // 12%

    log("Checking xdelta3...");
    let xdelta = XdeltaManager::ensure_available()
        .map_err(|e| format!("Failed to get xdelta3: {}", e))?;

    let processor = AssetProcessor::new(
        resolver,
        xdelta,
        mpi_dir.clone(),
        output_path.clone(),
        &locations,
        &bsa_targets,
    );

    std::fs::create_dir_all(&output_path)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    // Phase 3: Process assets (10-80%)
    log("Processing assets...");
    let progress_clone = Arc::clone(&progress);
    let log_messages_clone2 = Arc::clone(&log_messages);
    let log_file_clone2 = Arc::clone(&log_file);

    let stats = processor.process_assets_with_callback(&assets, move |current, total, msg| {
        // Update progress: 10% + (current/total * 70%)
        let pct = 1000 + ((current as u32 * 7000) / total as u32);
        progress_clone.store(pct, Ordering::Relaxed);

        // Log progress periodically
        if current % 1000 == 0 || current == total {
            if let Ok(mut logs) = log_messages_clone2.lock() {
                logs.push(format!("Assets: {}/{} - {}", current, total, msg));
            }
            if let Ok(mut file_guard) = log_file_clone2.lock() {
                if let Some(ref mut file) = *file_guard {
                    let timestamp = Local::now().format("%H:%M:%S");
                    let _ = writeln!(file, "[{}] Assets: {}/{} - {}", timestamp, current, total, msg);
                }
            }
        }
    }).map_err(|e| format!("Failed to process assets: {}", e))?;

    log(&format!("Processed: {} success, {} failed", stats.success, stats.failed));
    progress.store(8000, Ordering::Relaxed); // 80%

    // Phase 4: Write BSA archives (80-95%)
    log("Writing BSA archives...");
    let progress_clone = Arc::clone(&progress);
    let log_messages_clone3 = Arc::clone(&log_messages);
    let log_file_clone3 = Arc::clone(&log_file);

    let (bsa_success, bsa_fail) = processor.finalize_bsas_with_callback(move |current, total, bsa_name| {
        // Update progress: 80% + (current/total * 15%)
        let pct = 8000 + ((current as u32 * 1500) / total as u32);
        progress_clone.store(pct, Ordering::Relaxed);

        // Log each BSA
        if let Ok(mut logs) = log_messages_clone3.lock() {
            logs.push(format!("Writing BSA {}/{}: {}", current, total, bsa_name));
        }
        if let Ok(mut file_guard) = log_file_clone3.lock() {
            if let Some(ref mut file) = *file_guard {
                let timestamp = Local::now().format("%H:%M:%S");
                let _ = writeln!(file, "[{}] Writing BSA {}/{}: {}", timestamp, current, total, bsa_name);
            }
        }
    }).map_err(|e| format!("Failed to write BSAs: {}", e))?;

    log(&format!("BSA archives: {} created, {} failed", bsa_success, bsa_fail));
    progress.store(9000, Ordering::Relaxed); // 90%

    // Phase 5: Post-installation commands (90-95%)
    let post_commands = ManifestLoader::get_post_commands(&manifest);
    if !post_commands.is_empty() {
        log("Executing post-installation commands...");
        let (post_success, post_fail) = ManifestLoader::execute_post_commands(&post_commands, &output_path.to_string_lossy())
            .map_err(|e| format!("Failed to execute post-commands: {}", e))?;
        log(&format!("Post-commands: {} success, {} failed", post_success, post_fail));
    }
    progress.store(9500, Ordering::Relaxed); // 95%

    // Phase 6: Cleanup (95-100%)
    if cleanup_needed {
        log("Cleaning up...");
        let _ = MpiExtractor::cleanup_temp(&mpi_dir);
    }

    progress.store(10000, Ordering::Relaxed); // 100%
    log("Installation complete!");
    Ok(())
}

fn run_decompression(
    game: DecompressGame,
    data_path: PathBuf,
    output_path: Option<PathBuf>,
    backup: bool,
    log_messages: Arc<Mutex<Vec<String>>>,
    progress: Arc<AtomicU32>,
) -> Result<(), String> {
    let log = |msg: &str| {
        if let Ok(mut logs) = log_messages.lock() {
            logs.push(msg.to_string());
        }
    };

    log(&format!("Game: {}", game.name()));
    log(&format!("Data path: {}", data_path.display()));
    if let Some(ref out) = output_path {
        log(&format!("Output: {}", out.display()));
    }
    log(&format!("Backup: {}", if backup { "yes" } else { "no" }));

    // Validate path
    if !data_path.exists() {
        return Err(format!("Data path does not exist: {}", data_path.display()));
    }

    // Create decompressor
    let mut decompressor = BsaDecompressor::new(game, data_path.clone())
        .with_backup(backup);

    if let Some(out) = output_path {
        decompressor = decompressor.with_output(out);
    }

    // Find BSAs
    let bsas = decompressor.find_bsas()
        .map_err(|e| format!("Failed to find BSAs: {}", e))?;

    if bsas.is_empty() {
        return Err(format!("No BSA files found for {} in {}", game.name(), data_path.display()));
    }

    log(&format!("Found {} BSA files", bsas.len()));
    for bsa in &bsas {
        log(&format!("  - {}", bsa.file_name().unwrap_or_default().to_string_lossy()));
    }

    let progress_clone = Arc::clone(&progress);
    let log_messages_clone = Arc::clone(&log_messages);

    // Run decompression
    let result = decompressor.decompress_with_callback(move |current, total_bsas, msg| {
        // Update progress
        let pct = ((current as u32 * 10000) / total_bsas as u32).min(10000);
        progress_clone.store(pct, Ordering::Relaxed);

        // Log
        if let Ok(mut logs) = log_messages_clone.lock() {
            logs.push(format!("[{}/{}] {}", current, total_bsas, msg));
        }
    }).map_err(|e| format!("Decompression failed: {}", e))?;

    // Summary
    log(&format!("BSAs processed: {}", result.bsas_processed));
    log(&format!("Files extracted: {}", result.files_extracted));
    if result.files_converted > 0 {
        log(&format!("OGG->WAV conversions: {}", result.files_converted));
    }

    if !result.errors.is_empty() {
        for err in &result.errors {
            log(&format!("Error: {}", err));
        }
        return Err(format!("{} errors occurred", result.errors.len()));
    }

    progress.store(10000, Ordering::Relaxed);
    log("Decompression complete!");
    Ok(())
}

fn find_manifest(mpi_dir: &PathBuf) -> Result<PathBuf, String> {
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

    // Search recursively
    for entry in walkdir::WalkDir::new(mpi_dir).max_depth(3).into_iter().flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if (name.contains("manifest") || name == "index.json")
            && entry.path().extension().map(|e| e == "json").unwrap_or(false)
        {
            return Ok(entry.path().to_path_buf());
        }
    }

    Err(format!("Manifest not found in: {}", mpi_dir.display()))
}
