//! BSA Processing Strategy Benchmark
//!
//! Tests different approaches to BSA extraction and processing:
//! 1. Sequential: One BSA at a time, all threads focused on decompression
//! 2. Parallel: All BSAs concurrently, threads shared across them
//! 3. Batched: N BSAs at a time (2, 4, 8)
//!
//! Also measures RAM usage throughout each strategy.
//!
//! Usage: cargo run --release --example bench_strategies

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::thread;

use ba2::tes4::{Archive, FileCompressionOptions};
use ba2::Reader;
use rayon::prelude::*;
use sysinfo::System;

// ─── RAM Monitor ────────────────────────────────────────────────────────────

struct RamSnapshot {
    _timestamp: Duration,
    rss_mb: f64,
    available_mb: f64,
}

struct RamMonitor {
    snapshots: Arc<std::sync::Mutex<Vec<RamSnapshot>>>,
    running: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RamMonitor {
    fn start(interval_ms: u64) -> Self {
        let monitor_start = Instant::now();
        let snapshots = Arc::new(std::sync::Mutex::new(Vec::new()));
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

        let s = snapshots.clone();
        let r = running.clone();
        let handle = thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                let rss_mb = get_process_rss_mb();
                let mut sys = System::new();
                sys.refresh_memory();
                let available_mb = sys.available_memory() as f64 / 1024.0 / 1024.0;

                s.lock().unwrap().push(RamSnapshot {
                    _timestamp: monitor_start.elapsed(),
                    rss_mb,
                    available_mb,
                });

                thread::sleep(Duration::from_millis(interval_ms));
            }
        });

        Self {
            snapshots,
            running,
            handle: Some(handle),
        }
    }

    fn stop(mut self) -> RamReport {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }

        let snapshots = self.snapshots.lock().unwrap();
        let peak_rss = snapshots.iter().map(|s| s.rss_mb).fold(0.0f64, f64::max);
        let min_available = snapshots.iter().map(|s| s.available_mb).fold(f64::MAX, f64::min);
        let initial_rss = snapshots.first().map(|s| s.rss_mb).unwrap_or(0.0);

        RamReport {
            initial_rss_mb: initial_rss,
            peak_rss_mb: peak_rss,
            delta_rss_mb: peak_rss - initial_rss,
            min_available_mb: min_available,
            sample_count: snapshots.len(),
        }
    }
}

impl Drop for RamMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
struct RamReport {
    initial_rss_mb: f64,
    peak_rss_mb: f64,
    delta_rss_mb: f64,
    min_available_mb: f64,
    sample_count: usize,
}

impl std::fmt::Display for RamReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RSS: {:.0}→{:.0} MB (Δ{:.0} MB) | Avail min: {:.0} MB | {} samples",
            self.initial_rss_mb, self.peak_rss_mb, self.delta_rss_mb,
            self.min_available_mb, self.sample_count
        )
    }
}

fn get_process_rss_mb() -> f64 {
    // Read /proc/self/statm for accurate RSS
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let parts: Vec<&str> = statm.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(rss_pages) = parts[1].parse::<u64>() {
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                return (rss_pages * page_size) as f64 / 1024.0 / 1024.0;
            }
        }
    }
    0.0
}

// ─── BSA Discovery ──────────────────────────────────────────────────────────

struct BsaInfo {
    path: PathBuf,
    name: String,
    disk_size_mb: f64,
    file_count: usize,
    compressed_count: usize,
}

fn discover_bsas(dirs: &[&str]) -> Vec<BsaInfo> {
    let mut bsas = Vec::new();

    for dir in dirs {
        let path = Path::new(dir);
        if !path.exists() {
            continue;
        }

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.extension().map(|x| x == "bsa").unwrap_or(false) {
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    let disk_size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);

                    // Skip tiny BSAs (invalidation files, etc.)
                    if disk_size < 1024 * 1024 {
                        continue;
                    }

                    // Count files
                    let read_result: Result<(Archive, _), _> = Archive::read(p.as_path());
                    let (file_count, compressed_count) = match read_result {
                        Ok((archive, _)) => {
                            let mut total = 0;
                            let mut compressed = 0;
                            for (_, folder) in archive.iter() {
                                for (_, file) in folder.iter() {
                                    total += 1;
                                    if file.is_compressed() {
                                        compressed += 1;
                                    }
                                }
                            }
                            (total, compressed)
                        }
                        Err(_) => (0, 0),
                    };

                    bsas.push(BsaInfo {
                        path: p,
                        name,
                        disk_size_mb: disk_size as f64 / 1024.0 / 1024.0,
                        file_count,
                        compressed_count,
                    });
                }
            }
        }
    }

    // Sort by size descending (largest first)
    bsas.sort_by(|a, b| b.disk_size_mb.partial_cmp(&a.disk_size_mb).unwrap());
    bsas
}

// ─── Extraction Strategies ──────────────────────────────────────────────────

/// Extract up to `max_files` from a single BSA, decompress in parallel
/// Returns (files_extracted, bytes_decompressed)
fn extract_bsa_parallel(bsa_path: &Path, max_files: usize) -> (usize, usize) {
    let (archive, options): (Archive, _) = match Archive::read(bsa_path) {
        Ok(a) => a,
        Err(_) => return (0, 0),
    };
    let compression_options: FileCompressionOptions = (&options).into();

    // Collect file references
    let mut files: Vec<&ba2::tes4::File> = Vec::new();
    for (_, folder) in archive.iter() {
        for (_, file) in folder.iter() {
            files.push(file);
            if files.len() >= max_files {
                break;
            }
        }
        if files.len() >= max_files {
            break;
        }
    }

    let total_bytes = AtomicUsize::new(0);
    let file_count = AtomicUsize::new(0);

    files.par_iter().for_each(|file| {
        let data = if file.is_compressed() {
            file.decompress(&compression_options)
                .map(|d| d.as_bytes().to_vec())
                .unwrap_or_default()
        } else {
            file.as_bytes().to_vec()
        };
        total_bytes.fetch_add(data.len(), Ordering::Relaxed);
        file_count.fetch_add(1, Ordering::Relaxed);
        drop(data);
    });

    (file_count.load(Ordering::Relaxed), total_bytes.load(Ordering::Relaxed))
}

struct StrategyResult {
    name: String,
    elapsed: Duration,
    total_files: usize,
    total_bytes: usize,
    ram: RamReport,
}

impl StrategyResult {
    fn throughput_mb_s(&self) -> f64 {
        self.total_bytes as f64 / 1024.0 / 1024.0 / self.elapsed.as_secs_f64()
    }
}

/// Strategy 1: Sequential - one BSA at a time, all threads on that BSA
fn bench_sequential(bsas: &[&BsaInfo], max_files_per_bsa: usize) -> StrategyResult {
    let monitor = RamMonitor::start(100);
    let start = Instant::now();

    let mut total_files = 0;
    let mut total_bytes = 0;

    for bsa in bsas {
        let (files, bytes) = extract_bsa_parallel(&bsa.path, max_files_per_bsa);
        total_files += files;
        total_bytes += bytes;
    }

    let elapsed = start.elapsed();
    let ram = monitor.stop();

    StrategyResult {
        name: "Sequential (1 BSA at a time)".to_string(),
        elapsed,
        total_files,
        total_bytes,
        ram,
    }
}

/// Strategy 2: Fully parallel - all BSAs concurrently using a dedicated thread pool per BSA
fn bench_all_parallel(bsas: &[&BsaInfo], max_files_per_bsa: usize) -> StrategyResult {
    let monitor = RamMonitor::start(100);
    let start = Instant::now();

    let total_files = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);

    let paths: Vec<_> = bsas.iter().map(|b| b.path.clone()).collect();

    std::thread::scope(|s| {
        for path in &paths {
            let tf = &total_files;
            let tb = &total_bytes;
            s.spawn(move || {
                let (files, bytes) = extract_bsa_parallel(path, max_files_per_bsa);
                tf.fetch_add(files, Ordering::Relaxed);
                tb.fetch_add(bytes, Ordering::Relaxed);
            });
        }
    });

    let elapsed = start.elapsed();
    let ram = monitor.stop();

    StrategyResult {
        name: "All Parallel (all BSAs at once)".to_string(),
        elapsed,
        total_files: total_files.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        ram,
    }
}

/// Strategy 3: Batched - N BSAs at a time
fn bench_batched(bsas: &[&BsaInfo], max_files_per_bsa: usize, batch_size: usize) -> StrategyResult {
    let monitor = RamMonitor::start(100);
    let start = Instant::now();

    let total_files = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);

    for batch in bsas.chunks(batch_size) {
        std::thread::scope(|s| {
            for bsa in batch {
                let path = bsa.path.clone();
                let tf = &total_files;
                let tb = &total_bytes;
                s.spawn(move || {
                    let (files, bytes) = extract_bsa_parallel(&path, max_files_per_bsa);
                    tf.fetch_add(files, Ordering::Relaxed);
                    tb.fetch_add(bytes, Ordering::Relaxed);
                });
            }
        });
    }

    let elapsed = start.elapsed();
    let ram = monitor.stop();

    StrategyResult {
        name: format!("Batched ({} BSAs at a time)", batch_size),
        elapsed,
        total_files: total_files.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        ram,
    }
}

/// Strategy 4: Sequential with dedicated thread pool (isolated from global rayon)
fn bench_sequential_dedicated_pool(bsas: &[&BsaInfo], max_files_per_bsa: usize, num_threads: usize) -> StrategyResult {
    let monitor = RamMonitor::start(100);
    let start = Instant::now();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to build thread pool");

    let total_files = AtomicUsize::new(0);
    let total_bytes = AtomicUsize::new(0);

    for bsa in bsas {
        let (archive, options): (Archive, _) = match Archive::read(bsa.path.as_path()) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let compression_options: FileCompressionOptions = (&options).into();

        let mut files: Vec<&ba2::tes4::File> = Vec::new();
        for (_, folder) in archive.iter() {
            for (_, file) in folder.iter() {
                files.push(file);
                if files.len() >= max_files_per_bsa {
                    break;
                }
            }
            if files.len() >= max_files_per_bsa {
                break;
            }
        }

        pool.install(|| {
            files.par_iter().for_each(|file| {
                let data = if file.is_compressed() {
                    file.decompress(&compression_options)
                        .map(|d| d.as_bytes().to_vec())
                        .unwrap_or_default()
                } else {
                    file.as_bytes().to_vec()
                };
                total_bytes.fetch_add(data.len(), Ordering::Relaxed);
                total_files.fetch_add(1, Ordering::Relaxed);
                drop(data);
            });
        });
    }

    let elapsed = start.elapsed();
    let ram = monitor.stop();

    StrategyResult {
        name: format!("Sequential (dedicated pool, {} threads)", num_threads),
        elapsed,
        total_files: total_files.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        ram,
    }
}

/// Strategy 5: Producer-consumer with crossbeam channel (mirrors actual pipeline)
fn bench_producer_consumer(bsas: &[&BsaInfo], max_files_per_bsa: usize, channel_cap: usize) -> StrategyResult {
    use crossbeam_channel::bounded;

    let monitor = RamMonitor::start(100);
    let start = Instant::now();

    let total_files = Arc::new(AtomicUsize::new(0));
    let total_bytes = Arc::new(AtomicUsize::new(0));

    let (tx, rx) = bounded::<Vec<u8>>(channel_cap);

    // Spawn producer threads (one per BSA)
    let mut producer_handles = Vec::new();
    for bsa in bsas {
        let path = bsa.path.clone();
        let tx = tx.clone();
        let max = max_files_per_bsa;

        producer_handles.push(thread::spawn(move || {
            let (archive, options): (Archive, _) = match Archive::read(path.as_path()) {
                Ok(a) => a,
                Err(_) => return,
            };
            let compression_options: FileCompressionOptions = (&options).into();

            let mut count = 0;
            for (_, folder) in archive.iter() {
                for (_, file) in folder.iter() {
                    if count >= max {
                        return;
                    }

                    let data: Vec<u8> = if file.is_compressed() {
                        file.decompress(&compression_options)
                            .map(|d| d.as_bytes().to_vec())
                            .unwrap_or_default()
                    } else {
                        file.as_bytes().to_vec()
                    };

                    if tx.send(data).is_err() {
                        return;
                    }
                    count += 1;
                }
            }
        }));
    }
    drop(tx);

    // Consumer: process on rayon pool (simulates asset processing)
    let tf = total_files.clone();
    let tb = total_bytes.clone();
    rx.into_iter().par_bridge().for_each(|data| {
        tb.fetch_add(data.len(), Ordering::Relaxed);
        tf.fetch_add(1, Ordering::Relaxed);
        // Simulate light processing (checksum)
        let _sum: u64 = data.iter().map(|&b| b as u64).sum();
        drop(data);
    });

    for h in producer_handles {
        let _ = h.join();
    }

    let elapsed = start.elapsed();
    let ram = monitor.stop();

    StrategyResult {
        name: format!("Producer-Consumer (cap={})", channel_cap),
        elapsed,
        total_files: total_files.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        ram,
    }
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn print_result(result: &StrategyResult) {
    println!(
        "  {:42} {:>6.1}s  {:>6} files  {:>8.1} MB  {:>7.0} MB/s  | {}",
        result.name,
        result.elapsed.as_secs_f64(),
        result.total_files,
        result.total_bytes as f64 / 1024.0 / 1024.0,
        result.throughput_mb_s(),
        result.ram,
    );
}

fn main() {
    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║         BSA Processing Strategy Benchmark                      ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    // System info
    let mut sys = System::new_all();
    sys.refresh_all();
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let total_ram_gb = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
    let available_ram_gb = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    println!("System: {} CPUs, {:.1} GB total RAM, {:.1} GB available", cpus, total_ram_gb, available_ram_gb);
    println!("Process RSS at start: {:.0} MB", get_process_rss_mb());

    // Discover BSAs
    let bsa_dirs = [
        "/home/luke/.local/share/Steam/steamapps/common/Fallout 3 goty/Data",
        "/home/luke/.local/share/Steam/steamapps/common/Fallout New Vegas/Data",
        "/home/luke/.local/share/Steam/steamapps/common/Oblivion/Data",
    ];

    let all_bsas = discover_bsas(&bsa_dirs);

    println!("\nDiscovered {} BSAs:\n", all_bsas.len());
    println!("  {:<50} {:>8} {:>8} {:>10}", "Name", "Files", "Compr.", "Disk Size");
    println!("  {}", "-".repeat(80));
    for bsa in &all_bsas {
        println!(
            "  {:<50} {:>8} {:>8} {:>8.1} MB",
            bsa.name, bsa.file_count, bsa.compressed_count, bsa.disk_size_mb
        );
    }

    // Select BSAs for benchmarking - use the largest ones that represent the real workload
    let bench_bsas: Vec<&BsaInfo> = all_bsas.iter()
        .filter(|b| b.file_count > 100 && b.compressed_count > 0)
        .take(8)
        .collect();

    if bench_bsas.is_empty() {
        println!("\nNo suitable BSAs found for benchmarking!");
        return;
    }

    println!("\nBenchmarking with {} BSAs (largest with compressed files):", bench_bsas.len());
    for bsa in &bench_bsas {
        println!("  {} ({} files, {:.1} MB)", bsa.name, bsa.file_count, bsa.disk_size_mb);
    }

    let max_files = 2000; // Files per BSA to extract

    // ─── Warmup ─────────────────────────────────────────────────────────
    println!("\nWarming up (extracting 10 files from each BSA)...");
    for bsa in &bench_bsas {
        extract_bsa_parallel(&bsa.path, 10);
    }

    // ─── Benchmark 1: Extraction Strategies ─────────────────────────────
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  BENCHMARK 1: Extraction Strategy Comparison ({} files/BSA, {} BSAs)", max_files, bench_bsas.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let results = vec![
        bench_sequential(&bench_bsas, max_files),
        bench_all_parallel(&bench_bsas, max_files),
        bench_batched(&bench_bsas, max_files, 2),
        bench_batched(&bench_bsas, max_files, 4),
    ];

    for r in &results {
        print_result(r);
    }

    // Find winner
    let fastest = results.iter().min_by(|a, b| a.elapsed.cmp(&b.elapsed)).unwrap();
    println!("\n  Winner: {} ({:.1}s)", fastest.name, fastest.elapsed.as_secs_f64());

    // ─── Benchmark 2: Thread Pool Sizes ─────────────────────────────────
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  BENCHMARK 2: Thread Pool Size (sequential, {} files/BSA)", max_files);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let thread_counts: Vec<usize> = vec![
        cpus / 4,
        cpus / 2,
        cpus,
        cpus + cpus / 2,
    ].into_iter().filter(|&n| n >= 2).collect();

    let mut pool_results = Vec::new();
    for &threads in &thread_counts {
        let result = bench_sequential_dedicated_pool(&bench_bsas, max_files, threads);
        pool_results.push(result);
    }

    for r in &pool_results {
        print_result(r);
    }

    let fastest = pool_results.iter().min_by(|a, b| a.elapsed.cmp(&b.elapsed)).unwrap();
    println!("\n  Winner: {} ({:.1}s)", fastest.name, fastest.elapsed.as_secs_f64());

    // ─── Benchmark 3: Producer-Consumer Channel Capacity ────────────────
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  BENCHMARK 3: Producer-Consumer Channel Capacity ({} files/BSA)", max_files);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let capacities = [8, 16, 32, 64, 128];
    let mut chan_results = Vec::new();
    for &cap in &capacities {
        let result = bench_producer_consumer(&bench_bsas, max_files, cap);
        chan_results.push(result);
    }

    for r in &chan_results {
        print_result(r);
    }

    let fastest = chan_results.iter().min_by(|a, b| a.elapsed.cmp(&b.elapsed)).unwrap();
    println!("\n  Winner: {} ({:.1}s)", fastest.name, fastest.elapsed.as_secs_f64());

    // ─── Benchmark 4: Full Files (no limit) on top BSAs ─────────────────
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  BENCHMARK 4: Full BSA Extraction (top 4 BSAs, ALL files)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let top_bsas: Vec<&BsaInfo> = bench_bsas.iter().take(4).copied().collect();
    let full_max = usize::MAX;

    let full_results = vec![
        bench_sequential(&top_bsas, full_max),
        bench_all_parallel(&top_bsas, full_max),
        bench_batched(&top_bsas, full_max, 2),
    ];

    for r in &full_results {
        print_result(r);
    }

    let fastest = full_results.iter().min_by(|a, b| a.elapsed.cmp(&b.elapsed)).unwrap();
    let lowest_ram = full_results.iter().min_by(|a, b|
        a.ram.peak_rss_mb.partial_cmp(&b.ram.peak_rss_mb).unwrap()
    ).unwrap();

    println!("\n  Fastest:    {} ({:.1}s)", fastest.name, fastest.elapsed.as_secs_f64());
    println!("  Lowest RAM: {} (peak {:.0} MB)", lowest_ram.name, lowest_ram.ram.peak_rss_mb);

    // ─── Summary ────────────────────────────────────────────────────────
    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║                        SUMMARY                                ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    println!("  Final process RSS: {:.0} MB", get_process_rss_mb());

    let mut sys = System::new();
    sys.refresh_memory();
    println!("  System available RAM: {:.1} GB", sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0);
}
