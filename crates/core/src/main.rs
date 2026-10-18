use clap::{Parser, Subcommand};
use rsdisk_core::export::{self, Export, prune};
use rsdisk_core::format;
use rsdisk_core::scan::{Progress, ScanOptions, locate_unreadable, scan};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    Scan {
        path: Option<PathBuf>,
        /// Also write the largest files and folders as JSON, for the mockup.
        #[arg(long, value_name = "FILE")]
        json: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan { path, json } => scan_command(path, json),
    }
}

fn scan_command(path: Option<PathBuf>, json: Option<PathBuf>) {
    let home = std::env::home_dir().expect("no home folder");
    let root = path.unwrap_or_else(|| home.clone());
    let root = std::path::absolute(&root).unwrap_or(root);
    let progress = Progress::default();
    let mut result = scan(&root, &ScanOptions::for_home(&home), &progress);
    let node = result.tree.node(result.tree.root());
    let at_least = if result.error_count() == 0 {
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
    locate_unreadable(&mut result);
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

    if let Some(file) = json {
        let scanned_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let export = Export {
            path: root.to_string_lossy().into(),
            scanned_at,
            elapsed_ms: result.elapsed.as_millis() as u64,
            errors: result.errors.len(),
            root: prune(&result.tree, export::FILES, export::DIRS),
        };
        let written = serde_json::to_vec(&export)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&file, bytes));
        match written {
            Ok(()) => println!("  wrote  {}", file.display()),
            Err(e) => {
                eprintln!("couldn't write {}: {e}", file.display());
                std::process::exit(1);
            }
        }
    }
}
