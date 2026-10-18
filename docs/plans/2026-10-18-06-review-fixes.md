# Review Fixes Plan

**Goal:** act on the UX and UI review of 2026-10-18 (two reviewer agents, from screenshots
and code), with Tim's answers: RSdisk.app is how he starts it, `d` keeps requiring marks, the
Full Disk Access line stays. Cloud folders must never be trashable.
**Spec:** `spec.md` §2.13, §3.5, §3.8, §3.9, §3.11, §3.13 (revised in the same commit).

## What changed

1. **Guard.** Cloud folders (`~/Library/CloudStorage`, `~/Library/Mobile Documents`, and each
   folder directly inside them), not-scanned folders and other volumes can't be marked, nor
   any folder holding one (so not `~/Library` either). A refused mark says why in a new note
   line in the header and doesn't move down. Unreadable items show "size unknown" in the
   confirm dialog. `Tree::find` (core) looks nodes up by path.
2. **The app bundle.** `NSAppleEventsUsageDescription` in Info.plist; permission messages
   name RSdisk when running from the bundle; the Full Disk Access line also checks Desktop,
   Documents and Downloads; `tools/bundle.sh` signs with a fixed local certificate.
3. **Rescans.** Counters in the header, Esc stops one (the tree stays as it was), a second
   `r` or `c` says one is running, marks inside carry over by path, and `d` waits for it.
4. **Keyboard details.** A line under the list describes the selection (the tooltip's text);
   `e` lists the unreadable folders in the search box; search opens only while browsing and
   the header shows "result 3 of 20"; Esc stops a rescan, else closes Quick Look, else forgets
   the search results; `l` on a folder that can't be opened says why; the header shows the
   current folder's size "of" the total; "?: keys" in the legend; a cancelled scan's total
   says "at least".
5. **Dialogs.** All-failed trash runs say "Nothing was moved" and that the items are still
   marked; dialogs have buttons; search results are clickable; help, search and dialogs sit
   on one scrim that blocks clicks to the rows behind.
6. **Colour roles.** List bars use the item's own fill (kind or age), so folders no longer
   share the accent yellow with images. Marks are a dashed outline in the text colour and an
   inverted "marked" tag. Warnings get their own `warning` token (Zed's `warning`). The light
   accent is darker (about 5:1). Merged remainders are "small items" in their own neutral and
   are labelled when big enough; "age unknown" has a swatch. Cell text follows the theme.
7. **Labels and layout.** Treemap headers cut the name, never the size. Sunburst labels show
   whole names or at least 8 characters, else nothing. One-line breadcrumbs collapse the
   middle. The empty folder message sits in the list and the visual says "Nothing here".
   Numbers use one mono face (`theme::MONO`); scan stats have fixed widths; the scan path is
   cut from the start; half-page jumps use the list's real height.

## Rulings during execution

- Signing uses Tim's existing "murmur dev" code-signing certificate (override with
  `RSDISK_SIGN`), since it was already trusted; without it, `bundle.sh` signs ad hoc and warns.
- Files inside a cloud account folder stay markable once `c` scanned it: trashing one file
  there is a deliberate act. Only the cloud roots, the folders directly inside them, and what
  holds them are guarded.
- Not done: asking before `q` quits with marks pending, and a narrower list column (both
  dropped by the reviewers themselves); a hatch pattern for small items (a distinct neutral
  plus a label was enough).
- Verified by screenshots: the treemap, sunburst, age mode, empty folder and scan screen.
  Not verified (needs keys or the Dock app): refused marks, `e`, the dialogs and their
  buttons, rescan counters and Esc, the bundle's permission prompts.
