use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::thread::JoinHandle;

use ttw_installer::bsa_decompressor::{BsaDecompressor, DecompressGame};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Idle,
    Running,
    Success,
    Failed,
}

struct DecompressorApp {
    // Settings
    game: i32, // 0=FO3, 1=FNV, 2=Oblivion
    data_path: String,
    output_path: String,
    create_backup: bool,

    // State
    status: Status,
    progress: Arc<AtomicU32>,
    log_messages: Arc<Mutex<Vec<String>>>,
    worker_thread: Option<JoinHandle<Result<(), String>>>,
}

impl Default for DecompressorApp {
    fn default() -> Self {
        Self {
            game: 1, // Default to FNV
            data_path: String::new(),
            output_path: String::new(),
            create_backup: true,
            status: Status::Idle,
            progress: Arc::new(AtomicU32::new(0)),
            log_messages: Arc::new(Mutex::new(Vec::new())),
            worker_thread: None,
        }
    }
}

impl eframe::App for DecompressorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check if worker thread finished
        if self.status == Status::Running {
            if let Some(ref handle) = self.worker_thread {
                if handle.is_finished() {
                    if let Some(handle) = self.worker_thread.take() {
                        match handle.join() {
                            Ok(Ok(())) => self.status = Status::Success,
                            Ok(Err(e)) => {
                                if let Ok(mut logs) = self.log_messages.lock() {
                                    logs.push(format!("ERROR: {}", e));
                                }
                                self.status = Status::Failed;
                            }
                            Err(_) => self.status = Status::Failed,
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Title
            ui.horizontal(|ui| {
                ui.heading("BSA Decompressor");
                ui.label("v0.1.2");
            });
            ui.separator();
            ui.add_space(5.0);

            ui.label("Decompress game BSA archives to improve mod compatibility and loading times.");
            ui.add_space(5.0);

            // TTW Warning
            if self.game == 0 || self.game == 1 {
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

            // Settings grid
            egui::Grid::new("settings_grid")
                .num_columns(3)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    // Game selector
                    ui.label("Game:");
                    egui::ComboBox::from_id_salt("game_select")
                        .width(400.0)
                        .selected_text(match self.game {
                            0 => "Fallout 3",
                            1 => "Fallout New Vegas",
                            2 => "Oblivion",
                            _ => "Select game...",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.game, 0, "Fallout 3");
                            ui.selectable_value(&mut self.game, 1, "Fallout New Vegas");
                            ui.selectable_value(&mut self.game, 2, "Oblivion");
                        });
                    ui.label("");
                    ui.end_row();

                    // Data folder path
                    ui.label("Data Folder:");
                    ui.add_sized(
                        [400.0, 20.0],
                        egui::TextEdit::singleline(&mut self.data_path)
                            .hint_text("Path to game's Data folder")
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.data_path = path.to_string_lossy().to_string();
                        }
                    }
                    ui.end_row();

                    // Output folder (optional)
                    ui.label("Output Folder:");
                    ui.add_sized(
                        [400.0, 20.0],
                        egui::TextEdit::singleline(&mut self.output_path)
                            .hint_text("Leave empty to replace in-place (with backup)")
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.output_path = path.to_string_lossy().to_string();
                        }
                    }
                    ui.end_row();
                });

            ui.add_space(5.0);
            ui.checkbox(&mut self.create_backup, "Create backup of original BSA files (.bsa.backup)");

            // FNV note
            if self.game == 1 {
                ui.add_space(3.0);
                ui.colored_label(
                    egui::Color32::LIGHT_BLUE,
                    "Note: FNV decompression converts ambient/emitter OGG sounds to WAV and extracts architecture meshes as loose files."
                );
            }

            ui.add_space(10.0);

            // Start button
            let can_start = self.status != Status::Running && !self.data_path.is_empty();
            let button_text = match self.status {
                Status::Running => "Decompressing...",
                _ => "Start Decompression",
            };

            let button = egui::Button::new(
                egui::RichText::new(button_text).size(16.0)
            ).min_size(egui::vec2(550.0, 30.0));

            if ui.add_enabled(can_start, button).clicked() {
                self.start_decompression();
            }

            ui.add_space(10.0);

            // Progress bar
            if self.status == Status::Running {
                let progress = self.progress.load(Ordering::Relaxed) as f32 / 10000.0;
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }

            // Status indicator
            match self.status {
                Status::Success => {
                    ui.colored_label(egui::Color32::GREEN, "Decompression completed successfully!");
                }
                Status::Failed => {
                    ui.colored_label(egui::Color32::RED, "Decompression failed. Check log for details.");
                }
                _ => {}
            }

            ui.add_space(5.0);

            // Log output
            ui.label("Log:");
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if let Ok(logs) = self.log_messages.lock() {
                        for msg in logs.iter() {
                            ui.label(msg);
                        }
                    }
                });
        });
    }
}

impl DecompressorApp {
    fn start_decompression(&mut self) {
        self.status = Status::Running;
        self.progress.store(0, Ordering::Relaxed);

        // Clear logs
        if let Ok(mut logs) = self.log_messages.lock() {
            logs.clear();
            logs.push("Starting BSA decompression...".to_string());
        }

        let game = match self.game {
            0 => DecompressGame::Fallout3,
            1 => DecompressGame::FalloutNV,
            2 => DecompressGame::Oblivion,
            _ => DecompressGame::FalloutNV,
        };

        let data_path = PathBuf::from(&self.data_path);
        let output_path = if self.output_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.output_path))
        };
        let backup = self.create_backup;
        let log_messages = Arc::clone(&self.log_messages);
        let progress = Arc::clone(&self.progress);

        let handle = thread::spawn(move || {
            run_decompression(game, data_path, output_path, backup, log_messages, progress)
        });

        self.worker_thread = Some(handle);
    }
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
        let pct = ((current as u32 * 10000) / total_bsas as u32).min(10000);
        progress_clone.store(pct, Ordering::Relaxed);

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

    log("Decompression complete!");
    progress.store(10000, Ordering::Relaxed);
    Ok(())
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "BSA Decompressor",
        options,
        Box::new(|_cc| Ok(Box::new(DecompressorApp::default()))),
    )
}
