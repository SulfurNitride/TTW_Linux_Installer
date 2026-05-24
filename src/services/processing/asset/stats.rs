use crate::services::MemoryReport;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Operation types from the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    Copy = 0,
    New = 1,
    Patch = 2,
    OggEnc2 = 4,
    AudioEnc = 5,
}

impl OpType {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Copy),
            1 => Some(Self::New),
            2 => Some(Self::Patch),
            4 => Some(Self::OggEnc2),
            5 => Some(Self::AudioEnc),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::New => "New",
            Self::Patch => "Patch",
            Self::OggEnc2 => "OggEnc2",
            Self::AudioEnc => "AudioEnc",
        }
    }
}

/// Per-operation timing breakdown.
#[derive(Debug, Default, Clone)]
pub struct OpTimings {
    pub copy_ns: u64,
    pub new_ns: u64,
    pub patch_ns: u64,
    pub ogg_ns: u64,
    pub audio_ns: u64,
    pub decompress_ns: u64,
    pub copy_count: usize,
    pub new_count: usize,
    pub patch_count: usize,
    pub ogg_count: usize,
    pub audio_count: usize,
    pub decompress_count: usize,
}

impl std::fmt::Display for OpTimings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_line = |name: &str, ns: u64, count: usize| -> String {
            if count == 0 {
                return String::new();
            }
            let secs = ns as f64 / 1_000_000_000.0;
            let avg_ms = if count > 0 {
                ns as f64 / count as f64 / 1_000_000.0
            } else {
                0.0
            };
            format!(
                "    {:<14} {:>8} ops  {:>8.1}s thread-time  {:>6.1}ms avg\n",
                name, count, secs, avg_ms
            )
        };

        write!(
            f,
            "{}{}{}{}{}{}",
            format_line("Decompress", self.decompress_ns, self.decompress_count),
            format_line("Copy", self.copy_ns, self.copy_count),
            format_line("New", self.new_ns, self.new_count),
            format_line("Patch", self.patch_ns, self.patch_count),
            format_line("OggEnc2", self.ogg_ns, self.ogg_count),
            format_line("AudioEnc", self.audio_ns, self.audio_count),
        )
    }
}

/// Thread-safe atomic counters for operation timing.
pub(super) struct AtomicOpTimings {
    copy_ns: AtomicU64,
    new_ns: AtomicU64,
    patch_ns: AtomicU64,
    ogg_ns: AtomicU64,
    audio_ns: AtomicU64,
    decompress_ns: AtomicU64,
    copy_count: AtomicUsize,
    new_count: AtomicUsize,
    patch_count: AtomicUsize,
    ogg_count: AtomicUsize,
    audio_count: AtomicUsize,
    decompress_count: AtomicUsize,
}

impl AtomicOpTimings {
    pub(super) fn new() -> Self {
        Self {
            copy_ns: AtomicU64::new(0),
            new_ns: AtomicU64::new(0),
            patch_ns: AtomicU64::new(0),
            ogg_ns: AtomicU64::new(0),
            audio_ns: AtomicU64::new(0),
            decompress_ns: AtomicU64::new(0),
            copy_count: AtomicUsize::new(0),
            new_count: AtomicUsize::new(0),
            patch_count: AtomicUsize::new(0),
            ogg_count: AtomicUsize::new(0),
            audio_count: AtomicUsize::new(0),
            decompress_count: AtomicUsize::new(0),
        }
    }

    pub(super) fn record(&self, op: OpType, elapsed_ns: u64) {
        match op {
            OpType::Copy => {
                self.copy_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                self.copy_count.fetch_add(1, Ordering::Relaxed);
            }
            OpType::New => {
                self.new_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                self.new_count.fetch_add(1, Ordering::Relaxed);
            }
            OpType::Patch => {
                self.patch_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                self.patch_count.fetch_add(1, Ordering::Relaxed);
            }
            OpType::OggEnc2 => {
                self.ogg_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                self.ogg_count.fetch_add(1, Ordering::Relaxed);
            }
            OpType::AudioEnc => {
                self.audio_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
                self.audio_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn record_decompress(&self, elapsed_ns: u64) {
        self.decompress_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        self.decompress_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> OpTimings {
        OpTimings {
            copy_ns: self.copy_ns.load(Ordering::Relaxed),
            new_ns: self.new_ns.load(Ordering::Relaxed),
            patch_ns: self.patch_ns.load(Ordering::Relaxed),
            ogg_ns: self.ogg_ns.load(Ordering::Relaxed),
            audio_ns: self.audio_ns.load(Ordering::Relaxed),
            decompress_ns: self.decompress_ns.load(Ordering::Relaxed),
            copy_count: self.copy_count.load(Ordering::Relaxed),
            new_count: self.new_count.load(Ordering::Relaxed),
            patch_count: self.patch_count.load(Ordering::Relaxed),
            ogg_count: self.ogg_count.load(Ordering::Relaxed),
            audio_count: self.audio_count.load(Ordering::Relaxed),
            decompress_count: self.decompress_count.load(Ordering::Relaxed),
        }
    }
}

/// Statistics from processing.
#[derive(Debug, Default)]
pub struct ProcessingStats {
    pub success: usize,
    pub failed: usize,
    pub bsa_success: usize,
    pub bsa_failed: usize,
    pub errors: Vec<String>,
    pub memory: Option<MemoryReport>,
    pub timings: Option<OpTimings>,
}

impl ProcessingStats {
    pub fn print_summary(&self) {
        println!("\nProcessing Summary:");
        println!("  Successful: {}", self.success);
        println!("  Failed: {}", self.failed);
        if self.bsa_success > 0 || self.bsa_failed > 0 {
            println!(
                "  BSA archives: {} built, {} failed",
                self.bsa_success, self.bsa_failed
            );
        }

        if let Some(ref timings) = self.timings {
            println!("\n  Operation Breakdown (thread-time = sum of per-thread durations):");
            print!("{}", timings);
        }

        if let Some(ref mem) = self.memory {
            println!("\n  Memory Usage:");
            println!("    Initial RSS:       {:.0} MB", mem.initial_rss_mb);
            println!("    Peak RSS:          {:.0} MB", mem.peak_rss_mb);
            println!("    Delta (peak-init): {:.0} MB", mem.delta_mb);
            println!("    Final RSS:         {:.0} MB", mem.final_rss_mb);
            println!(
                "    System Available:  {:.1} GB / {:.1} GB total",
                mem.system_available_mb / 1024.0,
                mem.system_total_mb / 1024.0
            );
        }

        if !self.errors.is_empty() {
            println!("\nErrors:");
            for error in &self.errors {
                println!("  - {}", error);
            }
        }
    }
}
