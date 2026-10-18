# rsdisk spec

The binding design for rsdisk. Plans in `docs/plans/` implement it and cite it by §-number.
When code and spec disagree, fix one of them in the same commit.

## 1. Goal

A disk space analyzer and cleaner for macOS, written in Rust. It scans a folder (by default
`~`) as fast as `dua`, shows where the space went in a native gpui-pre window, lets you move
around with vim keys or the mouse, and moves what you pick to the Trash.

The use case is "I need disk space back, quickly". Every feature is judged against that.

**Out of scope for v1:** permanent delete, saved or cached scans, a live-updating tree during
the scan, an `.app` bundle, APFS clone detection, zoxide integration, a volumes start screen,
scanning across volume boundaries.

## 2. Decisions

Each: the question, the answer, why, and what was rejected.

1. **GUI toolkit.** gpui-pre `=0.3.9` plus gpui-component `0.7.3` (which pins gpui-pre
   `=0.3.9`). Plain `gpui` on crates.io is unmaintained. gpui-component supplies the list,
   dialogs, scrollbars and theme plumbing; custom visuals are our own gpui elements.
   Rejected: plain gpui-pre with hand-rolled widgets (too much to build before the useful part).
2. **dua's role.** rsdisk depends on `dua-core` (dua's walking engine, a separate MIT crate
   since dua 2.45) for the walk only; the tree, size rules and everything above are rsdisk's.
   Revised 2026-10-18: the first version was "inspiration only", but our own rayon walk with
   one `lstat` per entry came out 29% slower than dua, and dua-core's macOS bulk-attribute
   listing is what closes that gap. Rejected: writing our own `getattrlistbulk` parser (a few
   hundred lines of unsafe buffer parsing plus its edge cases); depending on dua-cli itself.
3. **Code split.** A Cargo workspace: `rsdisk-core` (scan, tree, sizes, kinds, export,
   deletion logic; no gpui) and `rsdisk` (the app). Core has its own small CLI so it can be
   tested, benchmarked and used to feed the mockup without the GUI. Rejected: one crate with a
   debug flag.
4. **Scan root.** CLI argument if given, otherwise `~`. No remembered last root, no zoxide:
   zoxide ranks folders by visits, while cleanup is driven by size, and the space hogs are
   usually folders you never visit. Fuzzy search inside the scan (§3.10) covers "get there fast".
5. **Size counted.** Allocated bytes (`st_blocks * 512`), each hardlinked inode once, APFS
   clones ignored (documented in the README as a known overcount). Rejected: apparent size,
   because it isn't what deleting gives back.
6. **During the scan.** Live counters only (files, bytes, current path); the tree appears
   when the scan finishes. A live tree is a possible later upgrade. Rejected for v1: live tree,
   because it needs a tree that is shared and mutated under the UI.
7. **Volumes.** Never cross into another volume. A whole-disk scan means scanning
   `/System/Volumes/Data`; scanning `/` on its own stops at the sealed system volume.
8. **Cloud folders.** `~/Library/CloudStorage` and `~/Library/Mobile Documents` are not
   descended into. They appear as nodes marked "not scanned" with a key to scan them on demand.
   Walking them can trigger network fetches and is slow. Rejected: scanning them like any folder.
9. **Unreadable folders.** Marked in the tree as unknown size and counted ("N folders couldn't
   be read", with a list). Rejected: skipping silently.
10. **Visualization.** A sorted list is the backbone (it carries the keyboard navigation).
    Beside it, one visual pane that switches between a treemap and a sunburst with `v`,
    remembered in settings. Chosen by Tim on 2026-10-18 after the HTML mockup (§3.7) showed
    all three on his own home folder: both treemap and sunburst were useful. Colour shows file
    kind by default, age as a second mode. Rejected: the icicle; showing two visuals at once
    (too cramped next to the list on a laptop screen).
11. **Deletion.** Trash only, through Finder so "Put Back" works. Mark several items, review
    them with their total, confirm once. The scan root can't be trashed. After trashing, the
    tree drops the items and subtracts their sizes upward; no rescan. Revised 2026-10-18: the
    first version said `NSFileManager` keeps "Put Back"; it doesn't (a known macOS bug, noted
    in the `trash` crate). Finder is slower, plays its sound and asks once for permission to
    control Finder; Tim chose it for the undo. Rejected: permanent delete, single-item
    immediate delete, `NSFileManager` (no "Put Back").
12. **Search.** `/` fuzzy-matches paths across the whole scanned tree and jumps to the result.
    Rejected: filtering only the current folder.
13. **Running.** v1 is a binary launched from the terminal (`rsdisk [path]`), which borrows the
    terminal's Full Disk Access. Rejected for v1: an `.app` bundle, which needs its own grant
    and can lose it on every re-sign.
14. **Persistence.** No scan cache; every launch scans fresh. Only settings persist (§3.12).
15. **Theming.** Installed Zed themes, loaded the way Jasmijn and Riff do it (copied, not
    shared), mapped onto our colour roles and pushed into gpui-component's theme. Follows the
    system light/dark setting with one chosen theme per mode and a built-in pair as fallback.
    File-kind colours come from the theme's 8 terminal colours. Rejected: gpui-component's
    bundled themes only; the cuby/musy tokens.
16. **Speed target.** Within about 10% of `dua aggregate ~` wall-clock on the same machine,
    cold and warm cache, measured with `hyperfine`; under 1 GB memory for a full home scan.

## 3. Design

### 3.1 Workspace

```
Cargo.toml            workspace
crates/core/          rsdisk-core: lib + `rsdisk-core` CLI binary
crates/app/           rsdisk: gpui-pre app, depends on rsdisk-core
mockup/               throwaway HTML mockup (§3.7)
docs/plans/           numbered implementation plans
```

Core never depends on gpui, gpui-component or anything Cocoa except the Trash call (§3.11),
which sits behind a small trait so tests can run without touching the real Trash.

### 3.2 Scan

- `dua_core::walk` does the walking (§2.2): its worker threads list directories with
  `getattrlistbulk`, which returns names and sizes for a batch of entries in one call, and
  stream entries to the scanning thread. Its `descend` hook keeps the walk off other volumes
  and out of skipped folders. Symlinks are never followed; they count as their own (tiny)
  allocated size.
- The scanning thread applies the size rules (§3.4), groups entries by the walk's directory
  ids, and lays them out as the tree (§3.3) when the walk ends.
- Progress: shared atomic counters (files, dirs, bytes) plus a slot holding a recent path,
  which the UI polls a few times a second. Cancelling sets an atomic flag; the scanning thread
  stops consuming and dropping the walk stops its workers.
- dua-core reports a directory it couldn't open without saying which one. The scan result
  carries their count (`unlocated`), and `locate_unreadable` finds them afterwards by
  re-opening the entered directories that came back empty. That takes a second or two when
  macOS privacy settings block many folders, so callers run it after showing the result: the
  CLI after printing the totals, the GUI in the background while you browse.
- Result: the size tree (§3.3) and the list of errors.

### 3.3 Size tree

An arena: `Vec<Node>` indexed by `u32`, built once after the scan and then only shrunk by
deletions. Each node holds its name (not the full path; paths are rebuilt by walking parents),
parent index, children, allocated size (for a directory: its own blocks plus all descendants),
file count, kind (§3.6), modified time, and flags: directory, unreadable, not-scanned, symlink.
Children are kept sorted by size, largest first. A directory's children get consecutive ids,
and their sorted order lives in one shared permutation array, so a node is 48 bytes plus its
name with no per-directory allocation. Measured: a 4.3-million-entry home folder peaks at
about 750 MB during the scan.

### 3.4 Size rules

- Size is allocated bytes: `st_blocks * 512`.
- A file with `st_nlink > 1` is counted the first time its `(st_dev, st_ino)` is seen and as
  zero after that. The node is still shown.
- An entry whose `st_dev` differs from the root's is shown as a node with zero size and not
  entered (§2.7).

### 3.5 Special folders

- Cloud folders (§2.8): matched by path under the scan root. Shown with a not-scanned flag.
  Scanning one on demand runs §3.2 on that subtree and splices the result in.
- Unreadable directories (§2.9): flagged, size unknown (counted as 0 in totals), path added
  to the error list. The total is shown as "at least" when any error exists.

### 3.6 File kinds and age

Kind is a pure function of the file name, eight values to match a theme's eight terminal
colours: video, audio, image, archive (zip, tar, gz, dmg, iso, pkg, ...), document (pdf,
office formats, text), code (source files and their usual build artifacts), app (`.app`
bundles, binaries, dylibs), other. A directory takes the kind holding most of the bytes below
it (an `.app` bundle stays App) and the size-weighted mean modified time of the files below
it, computed in one pass when the tree is built and again after removals (`Tree::summarize`).
Age mode colours by modified time on a single ramp from recent (teal) through sand to old
(red, 5 years or more), log scale, as the mockup showed.

### 3.7 Export and mockup

`rsdisk-core scan <path> --json <file>` writes a pruned tree:

- Keep the 100 largest files and the 100 largest directories (by total size) in the scan,
  plus every ancestor of those up to the root.
- Every kept directory gets one synthetic child `{"name": "(other)", "other": true}` holding
  its size minus the sizes of its kept children, when that is above zero.
- Node shape: `name`, `size`, `files` (count), `kind` (files only), `mtime` (unix seconds),
  `children` (directories only), `other` (synthetic only). Root also carries `path`,
  `scanned_at`, `elapsed_ms` and `errors` (count).

The mockup is one HTML page (`mockup/index.html`) that loads such a file through a file
picker, so it works from `file://`. It shows treemap (squarified), sunburst and icicle side
by side from the same data; clicking a cell zooms all three; a toggle switches colour between
kind and age; a breadcrumb shows the zoom path. d3 comes from cdnjs at a pinned version.
Exported JSON holds personal paths and is gitignored.

### 3.8 Window layout

- The list sits on the left and the visual pane (treemap or sunburst, §2.10) on the right.
  Above them: the breadcrumb of the current folder and the scan totals.
- The visual always shows the folder the list is in. Opening or leaving a folder in either
  moves both, and the selected item is highlighted in both.
- The visual draws down until cells would be a few pixels small and merges what's left in a
  folder into one neutral "(other)" cell, as the mockup does.
- Marked items get their own pane along the bottom, shown only while anything is marked:
  "N items, X GB", with `d` to review and trash and `u` to clear.

### 3.9 Keys and mouse

Starting set, refined during the GUI plans:

- `j`/`k` move, `l`/`enter` open a folder, `h`/`backspace` go up.
- `gg`/`G` top/bottom, `ctrl-d`/`ctrl-u` half page.
- `space` mark, `u` unmark all, `d` trash the marked items (after confirming).
- `/` search, `n`/`N` next/previous match.
- `s` cycle sort (size, name, count), `r` rescan the current folder, `c` scan a not-scanned
  cloud folder.
- `v` switch the visual (treemap, sunburst), `a` switch colour (kind, age).
- `o` reveal in Finder, `p` preview the selected item with Quick Look (again closes it),
  `?` help, `q` quit.
- Mouse: click selects, double-click opens, clicking a cell in the visual selects that item.

### 3.10 Search

An in-memory fuzzy match over every node's name, in parallel, with no path index (full paths
for 4 million nodes would cost hundreds of MB). A query with `/` in it matches its last part
against the name and the earlier parts, in order, against the ancestors' names, so
`lib/cach` finds `~/Library/Caches`. Results list ranked by match score, then size. Picking
one opens its parent and selects it; `n`/`N` step through the results afterwards.

### 3.11 Deletion

- Marks are a set of node indices. Marking a folder that contains marked items replaces them.
- `d` opens a dialog listing marked paths and their total; confirming trashes each one through
  Finder (the `trash` crate with `DeleteMethod::Finder`), off the main thread since Finder is
  slow.
- Each success removes the node and subtracts its size and file count from every ancestor.
  Each failure stays in the tree and is reported in the dialog.
- The scan root is never markable.

### 3.12 Theming and settings

- Theme loading copies the Riff/Jasmijn pattern: read every theme in
  `~/.config/zed/themes` and Zed's extension theme folders, map each onto rsdisk's colour
  roles, fall back to a built-in light and dark theme for missing keys.
- Roles are pushed into gpui-component's theme colours so its widgets match.
- The eight file-kind colours are the theme's terminal ANSI colours, with a fixed fallback.
- Settings live in `~/Library/Application Support/rsdisk/settings.json`: the light theme
  name, the dark theme name, and the last visual and colour mode. The window follows the
  system appearance.

### 3.13 Running and permissions

`rsdisk [path]` from a terminal. When the error count suggests missing Full Disk Access
(any unreadable directory under `~/Library`), the summary says so and names the setting.

### 3.14 Performance

- Benchmark: `hyperfine` comparing `dua aggregate ~` with `rsdisk-core scan ~` (no export),
  warm cache, plus one cold run each after `sudo purge` when Tim runs it.
- Targets: within about 10% of dua's time; peak RSS under 1 GB for `~`.
- Results are recorded in `docs/benchmarks.md` with date, machine and counts.

### 3.15 Testing

- Core: unit tests on temporary directory trees built by the test (files of known block
  sizes, nested folders, hardlinks, symlinks, an unreadable folder via permissions, a fake
  cloud folder path). Size, count, kind, pruning and deletion bookkeeping are all asserted.
- The Trash is behind a trait; tests use a fake. One ignored-by-default test hits the real
  Trash with a temp file.
- App: logic that isn't drawing (key handling to actions, selection sync, marks) lives in
  plain structs with unit tests. Drawing is checked by running the app.
- Gate before every commit: `cargo test --workspace && cargo clippy --workspace -- -D warnings
  && cargo build --release`.

## 4. GUI plan sequence

- Plan 03, app shell: window, scan with live progress, the list with vim keys, breadcrumb,
  Zed themes, settings, and finding unreadable folders in the background.
- Plan 04, visuals: treemap and sunburst as custom gpui elements, in sync with the list.
- Plan 05, cleanup: marks and their pane, the confirm dialog, Trash, search, rescanning a
  folder (`r`) and scanning a cloud folder on demand (`c`).
