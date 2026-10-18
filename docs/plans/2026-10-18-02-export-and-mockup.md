# Export and Mockup Implementation Plan

**Goal:** Export a pruned scan as JSON and compare treemap, sunburst and icicle on it in HTML.
**Architecture:** A pruning pass in core picks the nodes worth drawing and writes them with
serde. A single static HTML page loads the file through a file picker and draws all three
visuals with d3.
**Tech Stack:** serde, serde_json (core); d3 from cdnjs, pinned version (mockup).
**Spec:** `spec.md` §2.10, §3.6, §3.7.

## Global Constraints

- Pruning rule and node shape exactly as §3.7.
- Exported JSON holds personal paths: never commit it (`*.scan.json` is gitignored).
- The mockup is throwaway. No build step, no framework, one HTML file.
- Gate before each commit as in plan 01.

## Plan sequence (this is Plan 2 of several)

Needs plan 01. Its outcome settles the open points in `spec.md` §4 for plan 03.

### Task 1: Pruning

**Files:** create `crates/core/src/export.rs`.
**Interfaces:** `fn prune(tree: &Tree, files: usize, dirs: usize) -> ExportNode` producing the
nested shape of §3.7.
**Tests:**
- the N largest files and M largest directories are all present;
- every kept node's ancestors are present;
- each directory's `(other)` child equals its size minus its kept children, and is absent
  when that is zero;
- sizes in the export add up to the root's size at every level;
- a tree smaller than N + M is exported whole, with no `(other)` nodes.
**Steps:** tests first, implementation, gate, commit.
**Verify:** gate green.

### Task 2: `--json` flag

**Files:** modify `crates/core/src/main.rs`.
**Interfaces:** `rsdisk-core scan [path] --json <file>`, using 100 and 100 (§3.7).
**Tests:** a round-trip test that serialises a pruned temp tree and checks the root fields.
**Steps:** tests, implementation, gate, commit.
**Verify:** `rsdisk-core scan ~ --json home.scan.json` writes a file of a few hundred KB.

### Task 3: Mockup page

**Files:** create `mockup/index.html`.
**Behaviour:**
- File picker (and drag-and-drop) loads a `.scan.json`.
- Three panels from the same data: squarified treemap, sunburst, icicle. Labels where they fit,
  size on hover.
- Clicking any cell zooms all three to that folder; a breadcrumb zooms back out.
- Toggle colour between kind (a fixed 8-colour palette standing in for theme colours) and age
  (one ramp, recent to old). `(other)` nodes are neutral grey.
- Follows system light/dark.
**Tests:** none; check it by loading a real export in a browser (use `127.0.0.1`, not
`localhost`, if served) and confirming zoom, toggle and breadcrumb work in all three panels.
**Steps:** build, check in the browser, commit.
**Verify:** Tim opens it on his own `~` export and picks the visual. Record the choice and the
reasons in `spec.md` §2.10 and §4, which unblocks plan 03.
