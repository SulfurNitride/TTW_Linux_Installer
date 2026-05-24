use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;
use ttw_installer::services::AudioProcessor;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input_dir = PathBuf::from(
        args.next()
            .context("usage: bench_rust_audio <input-dir> <output-dir> [limit]")?,
    );
    let output_dir = PathBuf::from(
        args.next()
            .context("usage: bench_rust_audio <input-dir> <output-dir> [limit]")?,
    );
    let limit = args
        .next()
        .and_then(|value| value.to_string_lossy().parse::<usize>().ok());

    std::fs::create_dir_all(&output_dir)?;
    let mut inputs: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"))
        })
        .collect();
    inputs.sort();
    if let Some(limit) = limit {
        inputs.truncate(limit);
    }

    let start = Instant::now();
    inputs.par_iter().try_for_each(|input| -> Result<()> {
        let source =
            std::fs::read(input).with_context(|| format!("failed to read {}", input.display()))?;
        let processor = AudioProcessor::new().with_params("-f:24000 -q:5");
        let output = processor
            .process_ogg_resample(&source)
            .with_context(|| format!("failed to convert {}", input.display()))?;
        std::fs::write(
            output_dir.join(input.file_name().context("input missing file name")?),
            output,
        )?;
        Ok(())
    })?;

    let elapsed = start.elapsed();
    let total_bytes: u64 = inputs
        .iter()
        .map(|input| {
            output_dir
                .join(input.file_name().unwrap())
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum();

    println!(
        "count={} elapsed_seconds={:.3} output_bytes={}",
        inputs.len(),
        elapsed.as_secs_f64(),
        total_bytes
    );

    Ok(())
}
