use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use ttw_installer::bsa_decompressor::{BsaDecompressor, DecompressGame};

#[derive(Debug, Clone, ValueEnum)]
enum Game {
    Fo3,
    Fnv,
    Oblivion,
}

impl From<Game> for DecompressGame {
    fn from(game: Game) -> Self {
        match game {
            Game::Fo3 => DecompressGame::Fallout3,
            Game::Fnv => DecompressGame::FalloutNV,
            Game::Oblivion => DecompressGame::Oblivion,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "bsa_decompressor")]
#[command(author = "Sulfur Nitride")]
#[command(version = "0.1.2")]
#[command(about = "Decompress BSA archives for Fallout 3, Fallout New Vegas, and Oblivion")]
#[command(long_about = "A standalone tool to decompress BSA archives for Bethesda games.\n\n\
    This tool decompresses the game's BSA files, which can improve mod compatibility\n\
    and loading times. For Fallout New Vegas, it also converts ambient/emitter sounds\n\
    from OGG to WAV format and extracts architecture meshes as loose files to keep\n\
    the BSA under 2GB.\n\n\
    WARNING: Do not use this if you plan to install Tale of Two Wastelands (TTW).\n\
    TTW requires the original compressed game files.")]
struct Args {
    /// Game to decompress BSAs for
    #[arg(short, long, value_enum)]
    game: Game,

    /// Path to the game's Data folder
    #[arg(short, long)]
    data_path: PathBuf,

    /// Output directory (defaults to same as data_path, replacing original files)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Skip creating backups of original BSA files
    #[arg(long, default_value = "false")]
    no_backup: bool,
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();
    let game: DecompressGame = args.game.into();

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║              BSA Decompressor v0.1.2                         ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // TTW warning
    println!("┌────────────────────────────────────────────────────────────────┐");
    println!("│  WARNING: Do not use this if installing Tale of Two Wastelands │");
    println!("│           TTW requires the original compressed game files.     │");
    println!("│           Only use this for standalone modding.                │");
    println!("└────────────────────────────────────────────────────────────────┘");
    println!();

    println!("Game: {}", game.name());
    println!("Data path: {}", args.data_path.display());
    if let Some(ref out) = args.output {
        println!("Output: {}", out.display());
    }
    println!("Backup: {}", if args.no_backup { "disabled" } else { "enabled" });
    println!();

    // Create decompressor
    let mut decompressor = BsaDecompressor::new(game, args.data_path);

    if let Some(output) = args.output {
        decompressor = decompressor.with_output(output);
    }

    if args.no_backup {
        decompressor = decompressor.with_backup(false);
    }

    // Find BSAs
    let bsas = decompressor.find_bsas()?;
    if bsas.is_empty() {
        println!("No BSA files found for {}", game.name());
        return Ok(());
    }

    println!("Found {} BSA files to process:", bsas.len());
    for bsa in &bsas {
        println!("  - {}", bsa.file_name().unwrap_or_default().to_string_lossy());
    }
    println!();

    // Run decompression with progress
    let result = decompressor.decompress_with_callback(|current, total, msg| {
        if current == 0 {
            println!("{}", msg);
        } else if current <= total {
            println!("[{}/{}] {}", current, total, msg);
        }
    })?;

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("Decompression complete!");
    println!("  BSAs processed: {}", result.bsas_processed);
    println!("  Files extracted: {}", result.files_extracted);
    if result.files_converted > 0 {
        println!("  Audio files converted (OGG→WAV): {}", result.files_converted);
    }
    if !result.errors.is_empty() {
        println!("  Errors: {}", result.errors.len());
        for err in &result.errors {
            println!("    - {}", err);
        }
    }
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}
