use clap::{Parser, Subcommand};
use rsdisk_core::format;
use rsdisk_core::scan::{Progress, ScanOptions, scan};
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
        Command::Scan { path } => scan_command(path),
    }
}

fn scan_command(path: Option<PathBuf>) {
    let home = std::env::home_dir().expect("no home folder");
    let root = path.unwrap_or_else(|| home.clone());
    let root = std::path::absolute(&root).unwrap_or(root);
    let progress = Progress::default();
    let result = scan(&root, &ScanOptions::for_home(&home), &progress);
    let node = result.tree.node(result.tree.root());
    let at_least = if result.errors.is_empty() {
        ""
    } else {
        "at least "
    };

    println!("{}", root.display());
    println!("  size   {at_least}{}", format::bytes(node.size));
    println!("  files  {}", node.files);
    println!(
        "  dirs   {}",
        progress.dirs.load(std::sync::atomic::Ordering::Relaxed)
    );
    println!("  time   {:.2?}", result.elapsed);
    if !result.errors.is_empty() {
        println!(
            "  {} folders or files couldn't be read",
            result.errors.len()
        );
        let library = home.join("Library");
        if result.errors.iter().any(|e| e.path.starts_with(&library)) {
            println!(
                "  Some of ~/Library is unreadable. Give your terminal Full Disk Access in \
                 System Settings > Privacy & Security for a complete scan."
            );
        }
    }
}
