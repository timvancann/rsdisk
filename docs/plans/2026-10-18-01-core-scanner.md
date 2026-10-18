# Core Scanner Implementation Plan

**Goal:** `rsdisk-core scan <path>` walks a folder in parallel and prints totals at dua's speed.
**Architecture:** A Cargo workspace with `rsdisk-core` (lib + CLI). A rayon walk fills a node
arena; sizes roll up after the walk; atomic counters report progress. No gpui anywhere.
**Tech Stack:** Rust 2024 edition, rayon, clap (derive), tempfile (dev).
**Spec:** `spec.md` §2.3 to §2.9, §2.16, §3.1 to §3.6, §3.14, §3.15.

## Global Constraints

- Core has no gpui, gpui-component or Cocoa dependency (§3.1).
- Size is `st_blocks * 512`; hardlinks counted once per `(st_dev, st_ino)` (§3.4).
- Never follow symlinks; never enter another volume (§2.7, §3.4).
- Cloud folders are not descended into (§2.8); unreadable folders are flagged and counted (§2.9).
- Gate before each commit: `cargo test --workspace && cargo clippy --workspace -- -D warnings
  && cargo build --release`.

## Plan sequence (this is Plan 1 of several)

01 core scanner, 02 export and mockup. GUI plans (03 onward) are written after the mockup.

**Ruling during execution:** Task 3 (file kinds) ran before Task 2, because a tree node
stores its kind. The builder takes parents before children (the walk produces them in that
order anyway), not "any order".

**Ruling after Task 5:** the first full `~` scan peaked at 1.12 GB, over the §3.14 budget.
The tree moved from a `Vec` of children per node to consecutive child ids plus one shared
order array (`add_children` replaces `add`), node size went from 72 to 48 bytes, and
`mtime` became `u32`. Peak dropped to about 750 MB.

### Task 1: Workspace skeleton

**Files:** create `Cargo.toml` (workspace), `crates/core/Cargo.toml`, `crates/core/src/lib.rs`,
`crates/core/src/main.rs`, `.gitignore` (`target/`, `*.scan.json`).
**Interfaces:** `rsdisk-core --help` works.
**Tests:** none beyond the gate compiling.
**Steps:** scaffold, run the gate, commit.
**Verify:** gate green; `cargo run -p rsdisk-core -- --help` prints usage.

### Task 2: Size tree arena

**Files:** create `crates/core/src/tree.rs`.
**Interfaces:** `Tree` with `root()`, `node(id)`, `children(id)` (sorted largest first),
`path(id) -> PathBuf`, `remove(id)` (subtracts size and file count from ancestors). Node fields
per §3.3. A `TreeBuilder` that takes nodes in any order with parent links and finishes by
rolling up directory sizes and sorting children.
**Tests:** rollup sums nested sizes; children sorted by size; `path` rebuilds the full path;
`remove` updates every ancestor and detaches the node; removing a node twice is a no-op.
**Steps:** tests first, minimal implementation, gate, commit.
**Verify:** gate green; `size_of::<Node>()` asserted under 100 bytes in a test.

### Task 3: File kinds

**Files:** create `crates/core/src/kind.rs`.
**Interfaces:** `enum Kind { Video, Audio, Image, Archive, Document, Code, App, Other }`,
`fn kind_of(name: &str) -> Kind`, case-insensitive on extension; `.app` directory names map
to App (the walker passes directory names too).
**Tests:** a handful per kind including uppercase extensions, no extension, dotfiles
(`.zshrc` is Other), double extensions (`x.tar.gz` is Archive).
**Steps:** tests first, implementation, gate, commit.
**Verify:** gate green.

### Task 4: Parallel walk

**Files:** create `crates/core/src/scan.rs`.
**Interfaces:** `fn scan(root: &Path, opts: &ScanOptions, progress: &Progress) -> ScanResult`
where `ScanResult { tree: Tree, errors: Vec<ScanError>, elapsed: Duration }`. `Progress` holds
atomic files/dirs/bytes, the latest path, and a cancel flag. `ScanOptions` holds the cloud
folder paths to skip (defaults per §2.8, relative to `$HOME`).
**Tests (on temp trees):**
- total equals the sum of `st_blocks * 512` of every entry, compared against values read
  independently in the test;
- a file hardlinked twice in the tree counts once;
- a symlink to a big file counts only the link itself;
- a directory with mode `000` is flagged unreadable, appears in errors, and the scan finishes;
- a folder at a configured cloud path is present, flagged not-scanned, with size 0;
- cancelling mid-scan returns promptly with a partial result flagged as cancelled;
- the progress counters match the final tree's counts.
Crossing volumes can't be built in a temp dir; cover it by unit-testing the "same device"
check as a function and noting it in the test file.
**Steps:** tests first, implementation (rayon scope, one task per directory, nodes collected
per task and merged into the builder), gate, commit.
**Verify:** gate green.

### Task 5: CLI scan command

**Files:** modify `crates/core/src/main.rs`.
**Interfaces:** `rsdisk-core scan [path]` (default `$HOME`), prints total size (human units),
file and directory counts, error count, elapsed time. Prints a one-line Full Disk Access hint
when any error lies under `~/Library` (§3.13).
**Tests:** formatting of human sizes (B, KB, MB, GB, TB using 1000-based units like Finder).
**Steps:** tests, implementation, gate, commit.
**Verify:** `cargo run --release -p rsdisk-core -- scan ~` finishes and its total is within a
few percent of `dua aggregate ~` (they differ in hardlink and error handling only).

### Task 6: Benchmark against dua

**Files:** create `docs/benchmarks.md`.
**Steps:** `hyperfine` is not installed yet; ask Tim before `brew install hyperfine`. Run
`hyperfine --warmup 1 'dua aggregate ~' './target/release/rsdisk-core scan ~'`, measure peak
RSS with `/usr/bin/time -l`, record both with date, machine and file count. Cold-cache runs
need `sudo purge`, so leave that command for Tim and note it in the doc.
**Verify:** warm time within about 10% of dua and peak RSS under 1 GB (§3.14). If not, try
`getattrlistbulk` (§3.2) as a follow-up task appended to this plan, and record the ruling.

**Outcome:** memory met (730 MiB). Speed missed: cold cache 17.2 s against dua's 13.3 s
(+29%), with about twice dua's system time. See `docs/benchmarks.md`.

### Task 7: Close the speed gap (not started)

**Approach:** measure before changing anything. Profile a warm scan (`samply` or Instruments)
to see whether the extra system time is the stat calls themselves (full-path `lstat` per
entry), rayon threads sleeping and waking on tiny per-entry tasks, or lock contention. Then
try the matching fix, cheapest first: only spawn tasks for subdirectories; `fstatat` relative
to an open directory handle; `getattrlistbulk` for a whole directory per call.
**Verify:** cold and warm `hyperfine` within about 10% of dua on a quiet machine, recorded in
`docs/benchmarks.md`; all scan tests still green.
