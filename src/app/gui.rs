use chrono::Local;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use ttw_installer::{
    app::{run_install as run_shared_install, InstallEvent, InstallRequest},
    services::{DetectedGame, GameDetection, Logger},
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
            .map(|p| {
                p.parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("mpi_installer.json")
            })
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

#[derive(Clone)]
struct GuiLogger {
    messages: Arc<Mutex<Vec<String>>>,
    file: Arc<Mutex<Option<fs::File>>>,
}

impl GuiLogger {
    fn new(messages: Arc<Mutex<Vec<String>>>, log_path: &PathBuf) -> Self {
        Self {
            messages,
            file: Arc::new(Mutex::new(fs::File::create(log_path).ok())),
        }
    }

    fn log(&self, msg: impl AsRef<str>) {
        let msg = msg.as_ref();
        if let Ok(mut logs) = self.messages.lock() {
            logs.push(msg.to_string());
        }

        if let Ok(mut file_guard) = self.file.lock() {
            if let Some(ref mut file) = *file_guard {
                let timestamp = Local::now().format("%H:%M:%S");
                let _ = writeln!(file, "[{}] {}", timestamp, msg);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    // Set up panic handler to log crashes
    std::panic::set_hook(Box::new(|panic_info| {
        let crash_log = std::env::current_exe()
            .map(|p| {
                p.parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("crash.log")
            })
            .unwrap_or_else(|_| PathBuf::from("crash.log"));

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let message = format!(
            "[{}] MPI Installer crashed!\n{}\n\nBacktrace:\n{:?}\n\n",
            timestamp,
            panic_info,
            std::backtrace::Backtrace::capture()
        );

        // Try to write to crash log
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log)
        {
            let _ = file.write_all(message.as_bytes());
        }

        // Also print to stderr
        eprintln!("{}", message);
    }));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([480.0, 360.0]),
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

struct MpiInstallerApp {
    // Path inputs
    fo3_path: String,
    fnv_path: String,
    oblivion_path: String,
    mpi_path: String,
    output_path: String,

    // State
    status: InstallStatus,
    progress: Arc<AtomicU32>, // Progress as percentage * 100 (0-10000 for 0.00%-100.00%)
    log_messages: Arc<Mutex<Vec<String>>>,

    // Installation thread handle
    install_thread: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Default for MpiInstallerApp {
    fn default() -> Self {
        // Load saved config
        let mut config = UserConfig::load();
        let detected = GameDetection::detect();
        fill_missing_config_paths(&mut config, &detected);

        Self {
            fo3_path: config.fo3_path.clone(),
            fnv_path: config.fnv_path.clone(),
            oblivion_path: config.oblivion_path.clone(),
            mpi_path: String::new(), // Don't save MPI path - it changes per install
            output_path: config.output_path,
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
                        self.status =
                            InstallStatus::Failed(format!("Thread panicked: {}", panic_msg));
                        self.add_log(&format!("Thread panicked: {}", panic_msg));
                    }
                }
            } else {
                self.install_thread = Some(handle);
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            let small_screen = ui.available_height() < 500.0;
            let heading_size = if small_screen { 18.0 } else { 24.0 };
            let log_height = if small_screen { 100.0 } else { 180.0 };

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Title
                    ui.heading(egui::RichText::new("MPI Installer").size(heading_size));
                    ui.add_space(5.0);

                    self.show_installer_tab(ui);

                    ui.add_space(10.0);

                    // Progress bar (shared)
                    ui.horizontal(|ui| {
                        let progress_value = self.progress.load(Ordering::Relaxed) as f32 / 10000.0;
                        let progress_bar = egui::ProgressBar::new(progress_value).show_percentage();
                        let avail = (ui.available_width() - 80.0).max(120.0);
                        ui.add_sized([avail, 20.0], progress_bar);

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
                            .max_height(log_height)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if let Ok(logs) = self.log_messages.lock() {
                                    for msg in logs.iter() {
                                        ui.label(msg);
                                    }
                                }
                            });
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
        if let Ok(mut logs) = self.log_messages.lock() {
            logs.push(msg.to_string());
            // Keep last 100 messages
            if logs.len() > 100 {
                logs.remove(0);
            }
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
        // Reserve space for label column (~140) and Browse button (~80) plus spacing
        let total_w = ui.available_width();
        let edit_w = (total_w - 240.0).max(160.0);

        // Path inputs grid
        egui::Grid::new("paths_grid")
            .num_columns(3)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                // Fallout 3
                ui.label("Fallout 3:");
                ui.add_sized(
                    [edit_w, 20.0],
                    egui::TextEdit::singleline(&mut self.fo3_path)
                        .hint_text("Path to Fallout 3 installation (optional)"),
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
                    [edit_w, 20.0],
                    egui::TextEdit::singleline(&mut self.fnv_path)
                        .hint_text("Path to Fallout New Vegas installation (optional)"),
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
                    [edit_w, 20.0],
                    egui::TextEdit::singleline(&mut self.oblivion_path)
                        .hint_text("Path to Oblivion installation (optional)"),
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
                    [edit_w, 20.0],
                    egui::TextEdit::singleline(&mut self.mpi_path)
                        .hint_text("Path to .mpi file or extracted package folder"),
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
                    [edit_w, 20.0],
                    egui::TextEdit::singleline(&mut self.output_path)
                        .hint_text("Where to install output files"),
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

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.status != InstallStatus::Running,
                    egui::Button::new("Auto-detect games"),
                )
                .clicked()
            {
                self.apply_detected_games();
            }

            let mut enabled = false;
            ui.add_enabled(
                false,
                egui::Checkbox::new(&mut enabled, "dream_archive reader (disabled)"),
            )
            .on_hover_text("dream_archive support is temporarily disabled for release builds.");
        });

        ui.add_space(10.0);

        // Start Installation button
        let can_install = self.status != InstallStatus::Running
            && !self.mpi_path.is_empty()
            && !self.output_path.is_empty();

        let btn_w = ui.available_width().max(200.0);
        let button = egui::Button::new(egui::RichText::new("Start Installation").size(16.0))
            .min_size(egui::vec2(btn_w, 30.0));

        if ui.add_enabled(can_install, button).clicked() {
            self.start_installation();
        }
    }

    fn start_installation(&mut self) {
        self.status = InstallStatus::Running;
        self.progress.store(0, Ordering::Relaxed);

        // Clear logs
        {
            if let Ok(mut logs) = self.log_messages.lock() {
                logs.clear();
                logs.push("Starting installation...".to_string());
            }
        }

        // Clone values for the thread
        let fo3_path = if self.fo3_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.fo3_path))
        };
        let fnv_path = if self.fnv_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.fnv_path))
        };
        let oblivion_path = if self.oblivion_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.oblivion_path))
        };
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

    fn apply_detected_games(&mut self) {
        let detected = GameDetection::detect();
        let mut applied = 0usize;

        if self.fo3_path.trim().is_empty() {
            if let Some(path) = detected_path_string(detected.fallout3.as_ref()) {
                self.fo3_path = path;
                applied += 1;
            }
        }
        if self.fnv_path.trim().is_empty() {
            if let Some(path) = detected_path_string(detected.falloutnv.as_ref()) {
                self.fnv_path = path;
                applied += 1;
            }
        }
        if self.oblivion_path.trim().is_empty() {
            if let Some(path) = detected_path_string(detected.oblivion.as_ref()) {
                self.oblivion_path = path;
                applied += 1;
            }
        }

        self.save_config();
        self.add_log(&format!("Auto-detected {} missing game path(s)", applied));
    }
}

fn fill_missing_config_paths(config: &mut UserConfig, detected: &GameDetection) {
    if config.fo3_path.trim().is_empty() {
        if let Some(path) = detected_path_string(detected.fallout3.as_ref()) {
            config.fo3_path = path;
        }
    }
    if config.fnv_path.trim().is_empty() {
        if let Some(path) = detected_path_string(detected.falloutnv.as_ref()) {
            config.fnv_path = path;
        }
    }
    if config.oblivion_path.trim().is_empty() {
        if let Some(path) = detected_path_string(detected.oblivion.as_ref()) {
            config.oblivion_path = path;
        }
    }
}

fn detected_path_string(game: Option<&DetectedGame>) -> Option<String> {
    game.map(|game| game.path.to_string_lossy().to_string())
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
    let log_path = Logger::log_path_for("Installation")
        .map_err(|e| format!("Failed to create log path: {}", e))?;
    let ui_log = GuiLogger::new(log_messages, &log_path);
    ui_log.log(format!("Log file: {}", log_path.display()));

    let report = run_shared_install(
        InstallRequest {
            mpi_path,
            fallout3_path: fo3_path,
            falloutnv_path: fnv_path,
            oblivion_path,
            destination_path: output_path,
            dry_run: false,
        },
        |event| match event {
            InstallEvent::Log(message) => ui_log.log(message),
            InstallEvent::Progress { current, .. } => {
                progress.store(current.min(10_000), Ordering::Relaxed);
            }
        },
    )
    .map_err(|e| e.to_string())?;

    let elapsed = report.elapsed;
    let minutes = elapsed.as_secs() / 60;
    let seconds = elapsed.as_secs() % 60;
    ui_log.log(format!(
        "Installation complete! Total time: {}m {}s",
        minutes, seconds
    ));

    Ok(())
}
