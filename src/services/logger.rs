use anyhow::{Result, Context};
use std::path::PathBuf;
use std::fs;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use tracing_appender::non_blocking::WorkerGuard;
use chrono::Local;

/// Logger configuration and initialization
pub struct Logger {
    /// Guard that must be kept alive for async logging to work
    _guard: WorkerGuard,
    /// Path to the log file
    log_path: PathBuf,
}

impl Logger {
    /// Initialize logging for an installation
    /// Creates a timestamped log file in the logs directory
    pub fn init(package_name: &str) -> Result<Self> {
        let log_dir = Self::get_log_directory()?;
        fs::create_dir_all(&log_dir)
            .with_context(|| format!("Failed to create log directory: {}", log_dir.display()))?;

        // Create timestamped filename: 2026-01-20_13-45-22_PackageName.log
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        let safe_name = Self::sanitize_filename(package_name);
        let log_filename = format!("{}_{}.log", timestamp, safe_name);
        let log_path = log_dir.join(&log_filename);

        // Create file appender
        let file = fs::File::create(&log_path)
            .with_context(|| format!("Failed to create log file: {}", log_path.display()))?;

        let (non_blocking, guard) = tracing_appender::non_blocking(file);

        // Set up subscriber with both console and file output
        // Default to WARN for external crates, INFO for our crate
        // This suppresses noisy INFO logs from symphonia/lewton audio libraries
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("warn,ttw_installer=info"));

        // File layer - detailed output
        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true);

        // Console layer - cleaner output
        let console_layer = fmt::layer()
            .with_writer(std::io::stdout)
            .with_ansi(true)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(console_layer)
            .init();

        tracing::info!("=== MPI Installer Log ===");
        tracing::info!("Package: {}", package_name);
        tracing::info!("Log file: {}", log_path.display());
        tracing::info!("Started at: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));

        Ok(Self {
            _guard: guard,
            log_path,
        })
    }

    /// Initialize a simple console-only logger (for commands that don't need file logging)
    pub fn init_console_only() -> Result<()> {
        // Default to WARN for external crates, INFO for our crate
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("warn,ttw_installer=info"));

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_target(false))
            .init();

        Ok(())
    }

    /// Get the log directory path (next to executable)
    fn get_log_directory() -> Result<PathBuf> {
        // Store logs next to the executable for easy access
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                return Ok(exe_dir.join("logs"));
            }
        }
        // Fallback to current working directory
        Ok(PathBuf::from("./logs"))
    }

    /// Sanitize a string for use in filenames
    fn sanitize_filename(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
                ' ' | '.' => '_',
                _ => '_',
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    /// Get the path to the current log file
    pub fn log_path(&self) -> &PathBuf {
        &self.log_path
    }

    /// Log installation completion summary
    pub fn log_summary(&self, success: usize, failed: usize, bsa_created: usize, bsa_failed: usize) {
        tracing::info!("=== Installation Summary ===");
        tracing::info!("Assets processed: {} success, {} failed", success, failed);
        tracing::info!("BSA archives: {} created, {} failed", bsa_created, bsa_failed);
        tracing::info!("Completed at: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        tracing::info!("Log saved to: {}", self.log_path.display());
    }

    /// List recent log files
    pub fn list_recent_logs(limit: usize) -> Result<Vec<PathBuf>> {
        let log_dir = Self::get_log_directory()?;

        if !log_dir.exists() {
            return Ok(Vec::new());
        }

        let mut logs: Vec<_> = fs::read_dir(&log_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension()
                    .map(|ext| ext == "log")
                    .unwrap_or(false)
            })
            .collect();

        // Sort by modification time, newest first
        logs.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        Ok(logs.into_iter()
            .take(limit)
            .map(|e| e.path())
            .collect())
    }
}
