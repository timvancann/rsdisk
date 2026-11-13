# rsdisk spec

The binding design for rsdisk. Plans in `docs/plans/` implement it and cite it by §-number.
When code and spec disagree, fix one of them in the same commit.

## 1. Goal

A disk space analyzer and cleaner for macOS, written in Rust. It scans a folder (by default
`~`) as fast as `dua`, shows where the space went in a native gpui-pre window, lets you move
around with vim keys or the mouse, and moves what you pick to the Trash.

The use case is "I need disk space back, quickly". Every feature is judged against that.

**Out of scope for v1:** permanent delete, saved or cached scans, a live-updating tree during
the scan, APFS clone detection, zoxide integration, a volumes start screen,
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
13. **Running.** RSdisk.app, built and installed with `just install` and started from the
    Dock (scans `~`); `rsdisk [path]` from a terminal still works. Revised 2026-10-18: v1 was
    terminal only, borrowing the terminal's Full Disk Access; Tim now starts it from the Dock.
    The app needs its own Full Disk Access and Finder automation grants, so it is signed with
    one fixed local certificate, which keeps those grants across rebuilds (an ad hoc
    signature changes every build and loses them).
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
  which the UI polls a few times a second. Like dua, there is no separate scan screen: the
  browser's list and visual show the items under the root as they fill in, under a header
  with the counters, and keys wait for the real tree. The scanning thread keeps a small tree
  of the folders down to the sunburst's depth (5) plus the files directly under the root;
  everything deeper adds its size, kind bytes and age to the folder holding it. It publishes
  that every 250 ms and the UI rebuilds a `Tree` from it, with the same sizes, kinds and ages
  the finished tree will have for those folders. The walk can deliver a folder's contents
  before the folder itself, so such entries wait until that folder is placed.
  Cancelling sets an atomic flag; the scanning thread
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
  to the error list. The total is shown as "at least" when any error exists, or the scan was
  stopped early. `e` lists them, to step through like search results.
- Cloud folders (and each account or app folder directly inside them), not-scanned folders
  and other volumes can't be marked, nor any folder holding one: their size reads 0 B or
  unknown, so a mark would look harmless, and trashing iCloud Drive or a Google Drive account
  deletes files on every device. A refused mark says why.

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
  Above them: the breadcrumb of the current folder (one line; deep paths collapse the middle)
  and the folder's size and file count, then the scan total. Under the list, one line about
  the selected item: its path, size, files, kind and age, the same as the visual's tooltip.
- A note line in the header answers keys that can't do what was asked (a refused mark, a
  folder that can't be opened); the next key clears it.
- The visual always shows the folder the list is in. Opening or leaving a folder in either
  moves both, and the selected item is highlighted in both.
- The visual draws down until cells would be a few pixels small and merges what's left in a
  folder into one neutral "small items" cell, as the mockup does.
- Colour roles: data colours (kinds, the age ramp) are never reused for state. Marks are a
  dashed outline in the text colour and an inverted "marked" tag in the list; the selection is
  a solid outline; warnings have their own colour.
- Marked items get their own pane along the bottom, shown only while anything is marked:
  "N items, X GB", with `d` to review and trash and `u` to clear.
- The list has a header row (Name, Size, Files) laid out like its rows; the current sort
  is lit, and a click on a column sorts by it. Its scrollbar is always shown (it never
  fades), so a long folder shows how far down you are; it can be dragged.
- A key bar along the window's bottom edge shows the keys used most (while the first scan
  runs, the few that work then), with `?` for the full sheet, grouped by what the keys do.
  Its `v` and `a` entries name the current visual and colour mode, which the legend under
  the visual used to say.
- Type: Space Grotesk for the interface and JetBrains Mono for paths, sizes and counts,
  both embedded (as in Odeion). The folder's name is the largest line, its size leads the
  totals in the text colour, and the rest of the header is small and dim.

### 3.9 Keys and mouse

Starting set, refined during the GUI plans:

- `j`/`k` move, `l`/`enter` open a folder, `h`/`backspace` go up.
- `gg`/`G` top/bottom, `ctrl-d`/`ctrl-u` half page.
- `space` mark, `u` unmark all, `d` trash the marked items (after confirming).
- `/` search, `n`/`N` next/previous match.
- `s` cycle sort (size, name, count), `r` rescan the current folder, `c` scan a not-scanned
  cloud folder.
- `v` switch the visual (treemap, sunburst), `a` switch colour (kind, age).
- `o` reveal in Finder, `p` preview the selected item in the Quick Look panel, which follows the
  selection while open (again closes it), `e` list the unreadable folders,
  `?` help, `q` quit.
- `Esc` stops a rescan, else closes the preview, else forgets the last search's results.
- Mouse: click selects, double-click opens, clicking a cell in the visual selects that item
  (or the item of the current folder it lies in). A right-click on a row or a cell marks or
  unmarks exactly that item, however deep, without moving the selection; the tooltip says
  what it would do. A "small items" cell, an item inside a marked folder and what §3.5
  guards answer with a note instead. While the first scan runs, clicks do nothing.
  Dialogs have buttons, search results can be clicked, and overlays block clicks to what is
  behind them.

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
- The scan root is never markable, nor what §3.5 guards.
- A rescan (`r`, `c`) shows its counters in the header and Esc stops it. Marks inside the
  rescanned folder carry over by path. `d` waits for a running rescan: its result was read
  before Finder moved anything, so grafting it in would bring trashed items back.

### 3.12 Theming and settings

- Theme loading copies the Riff/Jasmijn pattern: read every theme in
  `~/.config/zed/themes` and Zed's extension theme folders, map each onto rsdisk's colour
  roles, fall back to a built-in light and dark theme for missing keys.
- Roles are pushed into gpui-component's theme colours so its widgets match.
- The eight file-kind colours are the theme's terminal ANSI colours, with a fixed fallback.
- Settings live in `~/Library/Application Support/rsdisk/settings.json`: the light theme
  name, the dark theme name, and the last visual and colour mode. The window follows the
  system appearance.
- A settings window (Cmd-, or RSdisk > Settings) sets all four, laid out like macOS System
  Settings: titled groups of rows, label left, control right. Changes apply and save at
  once. It also shows the permissions of §3.13.

### 3.13 Running and permissions

RSdisk.app from the Dock, or `rsdisk [path]` from a terminal (§2.13). When the errors
suggest missing Full Disk Access (any unreadable folder under `~/Library`, `~/Desktop`,
`~/Documents` or `~/Downloads`), the summary says so and names the setting. Permission
messages name RSdisk when it runs from its bundle and the terminal otherwise.

The settings window shows both permissions and rechecks them every two seconds, so a change
made in System Settings shows up without reopening it:

- Full Disk Access: on when the system's permission database (`TCC.db`) can be opened.
  macOS never asks for it, so the window opens System Settings at Full Disk Access and says
  what to turn on and that rsdisk must be restarted.
- Control Finder: read with `AEDeterminePermissionToAutomateTarget`. When macOS hasn't asked
  yet, "Allow..." makes it show its own prompt; when it was refused, the window opens System
  Settings at Automation.

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
- Plan 06, review fixes: the UX and UI review of 2026-10-18 (guarding cloud folders, the app
  bundle's permissions, rescan safety, keyboard details, colour roles, labels).
