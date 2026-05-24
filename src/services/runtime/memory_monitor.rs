use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::System;
use tracing::{info, warn};

/// Lightweight background RAM monitor that tracks process memory usage.
/// Samples RSS and available system RAM at configurable intervals.
pub struct MemoryMonitor {
    running: Arc<AtomicBool>,
    peak_rss_bytes: Arc<AtomicU64>,
    initial_rss_bytes: u64,
    handle: Option<thread::JoinHandle<()>>,
    start: Instant,
}

/// Summary of memory usage over a monitoring period.
#[derive(Debug, Clone)]
pub struct MemoryReport {
    pub initial_rss_mb: f64,
    pub peak_rss_mb: f64,
    pub final_rss_mb: f64,
    pub delta_mb: f64,
    pub system_total_mb: f64,
    pub system_available_mb: f64,
    pub elapsed: Duration,
}

impl std::fmt::Display for MemoryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RAM: {:.0} MB initial → {:.0} MB peak (Δ{:.0} MB) | Final: {:.0} MB | System: {:.0}/{:.0} GB avail",
            self.initial_rss_mb,
            self.peak_rss_mb,
            self.delta_mb,
            self.final_rss_mb,
            self.system_available_mb / 1024.0,
            self.system_total_mb / 1024.0,
        )
    }
}

impl MemoryMonitor {
    /// Start monitoring with the given sample interval.
    /// A good default is 250ms - frequent enough to catch spikes, low overhead.
    pub fn start(interval: Duration) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let peak_rss_bytes = Arc::new(AtomicU64::new(0));
        let initial_rss = get_rss_bytes();

        peak_rss_bytes.store(initial_rss, Ordering::Relaxed);

        let r = running.clone();
        let peak = peak_rss_bytes.clone();

        let handle = thread::Builder::new()
            .name("memory-monitor".into())
            .spawn(move || {
                while r.load(Ordering::Relaxed) {
                    let rss = get_rss_bytes();
                    peak.fetch_max(rss, Ordering::Relaxed);
                    thread::sleep(interval);
                }
            })
            .map_err(|e| warn!("Failed to spawn memory monitor thread: {}", e))
            .ok();

        Self {
            running,
            peak_rss_bytes,
            initial_rss_bytes: initial_rss,
            handle,
            start: Instant::now(),
        }
    }

    /// Get current RSS in MB (can be called while monitoring is active).
    pub fn current_rss_mb(&self) -> f64 {
        get_rss_bytes() as f64 / 1024.0 / 1024.0
    }

    /// Get peak RSS seen so far in MB.
    pub fn peak_rss_mb(&self) -> f64 {
        self.peak_rss_bytes.load(Ordering::Relaxed) as f64 / 1024.0 / 1024.0
    }

    /// Stop monitoring and return a summary report.
    pub fn stop(mut self) -> MemoryReport {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }

        let final_rss = get_rss_bytes();
        let peak = self.peak_rss_bytes.load(Ordering::Relaxed);
        // Final sample might be highest
        let peak = peak.max(final_rss);

        let mut sys = System::new();
        sys.refresh_memory();

        let report = MemoryReport {
            initial_rss_mb: self.initial_rss_bytes as f64 / 1024.0 / 1024.0,
            peak_rss_mb: peak as f64 / 1024.0 / 1024.0,
            final_rss_mb: final_rss as f64 / 1024.0 / 1024.0,
            delta_mb: (peak - self.initial_rss_bytes) as f64 / 1024.0 / 1024.0,
            system_total_mb: sys.total_memory() as f64 / 1024.0 / 1024.0,
            system_available_mb: sys.available_memory() as f64 / 1024.0 / 1024.0,
            elapsed: self.start.elapsed(),
        };

        info!("Memory: {}", report);
        report
    }
}

impl Drop for MemoryMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Read process RSS in bytes (cross-platform via sysinfo).
fn get_rss_bytes() -> u64 {
    // Linux fast path: /proc/self/statm
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = statm.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(rss_pages) = parts[1].parse::<u64>() {
                    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                    return rss_pages * page_size;
                }
            }
        }
    }

    // Cross-platform fallback: use sysinfo
    #[cfg(not(target_os = "linux"))]
    {
        use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System, UpdateKind};
        let pid = Pid::from_u32(std::process::id());
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
        );
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        if let Some(process) = sys.process(pid) {
            return process.memory();
        }
    }
    0
}

/// Get current process RSS in MB (standalone function for one-off checks).
pub fn current_rss_mb() -> f64 {
    get_rss_bytes() as f64 / 1024.0 / 1024.0
}

/// Print a one-line RAM status to stdout.
pub fn print_ram_status(label: &str) {
    let rss = current_rss_mb();
    let mut sys = System::new();
    sys.refresh_memory();
    let available_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let total_gb = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    println!(
        "  [RAM] {}: Process {:.0} MB | System {:.1}/{:.1} GB available",
        label, rss, available_gb, total_gb
    );
}
