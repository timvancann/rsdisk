use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Scan a folder and report where the disk space went.
#[derive(Parser)]
#[command(name = "rsdisk-core", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a folder (default: your home folder) and print totals.
    Scan { path: Option<PathBuf> },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { path } => {
            let _ = path;
        }
    }
}
