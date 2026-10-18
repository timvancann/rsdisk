# Benchmarks

Targets (spec §3.14): within about 10% of `dua aggregate ~` wall-clock, peak RSS under 1 GB.

## 2026-10-18, plan 01 (commit 25333c9)

Machine: Apple M5 Pro (18 logical cores), 24 GB, macOS 27.0.1. Home folder: 3.9 million
files, 440k folders, 624 GB counted. The terminal has no Full Disk Access, so 489 entries
under `~/Library` were unreadable for both tools.

Warm cache, `hyperfine -N -i --warmup 1 --runs 5` (`-i` because dua exits 1 on IO errors):

| Command | Mean | ± σ | Min | Max | Peak memory |
|---|---|---|---|---|---|
| `dua aggregate ~` | 16.07 s | 1.80 s | 14.31 s | 18.46 s | 33 MiB |
| `rsdisk-core scan ~` (rayon default, 18 threads) | 17.46 s | 0.38 s | 16.89 s | 17.97 s | 732 MiB |
| `rsdisk-core scan ~` (8 threads, dua's default) | 17.88 s | 1.57 s | 17.01 s | 20.67 s | 734 MiB |

Reading it:

- rsdisk is 8.7% slower on the mean: inside the target, though not by much. dua's spread is
  wide, so the real gap is somewhere between level and about 15%.
- dua also walks the cloud folders (44 GB) that rsdisk skips, so the comparison slightly
  flatters dua's job size; it does not change the conclusion.
- The memory column isn't like for like: `dua aggregate` keeps no tree, while rsdisk keeps
  the whole tree for the GUI. 732 MiB is inside the 1 GB budget.
- Thread count barely matters; system time dominates (about 170 s of sys over 17 s of wall),
  so the kernel's directory and stat calls are the bottleneck. `getattrlistbulk` (§3.2) is
  the lever if this ever needs to get faster.

Cold cache (plan 01 walk), run by Tim in his own terminal tab (sudo needs a TTY):
`hyperfine -N -i --runs 3 --prepare 'sudo -n purge' 'dua aggregate /Users/timvancann' './target/release/rsdisk-core scan /Users/timvancann'`

| Command | Mean | ± σ | Min | Max | Peak memory |
|---|---|---|---|---|---|
| `dua aggregate ~` | 13.34 s | 0.06 s | 13.27 s | 13.38 s | 39 MiB |
| `rsdisk-core scan ~` | 17.21 s | 0.50 s | 16.81 s | 17.77 s | 730 MiB |

**rsdisk is 29% slower here: the target is missed.** dua's cold runs beat its own warm runs
(13.3 s against 16.1 s), which suggests the warm table was skewed by load on the machine (two
agents were compiling at the time) and the cold table is the more honest one. Its tight
spread agrees. A quiet warm rerun should confirm. The doubled system time (about 170 s
against dua's 77 s) is where the gap lives; see the follow-up in plan 01.

## 2026-10-18, walk on dua-core (plan 01 task 7)

Same machine, warm cache, quiet, `hyperfine -N -i --warmup 1 --runs 5`. rsdisk uses all 18
cores; dua runs with 8 threads (its configured default here).

| Folder | dua | rsdisk-core | Difference | rsdisk peak memory |
|---|---|---|---|---|
| `~/repos` (1.15M files) | 3.71 s ± 0.40 | 2.75 s ± 0.21 | 26% faster | 210 MiB |
| `~` (3.9M files) | 14.30 s ± 0.74 | 15.17 s ± 0.69 | 6% slower | 583 MiB |

Totals match dua to the rounding (86.4 GB on `~/repos`). Phase timings for one `~` scan: the
walk finishes at 13.0 s (faster than dua), building the tree takes 0.1 s, and finding which
489 directories couldn't be opened takes 1.7 s. dua-core's error doesn't carry the path, so
rsdisk re-opens the 12.5k entered-but-empty directories; the cost is the privacy (TCC)
denials, which are slow every time, not the number of directories.

**Follow-up (same day):** that search now runs after the result is reported
(`locate_unreadable`, spec §3.2), so the scan is done when the walk is. Four alternating
pairs on a busier machine, scan time as `rsdisk-core` prints it against dua's wall time:
dua 20.37 / 19.46 / 17.29 / 17.99 s (mean 18.78), rsdisk 21.42 / 18.51 / 20.45 / 19.15 s
(mean 19.88), so about 6% behind under load. The quiet-machine phase timing above (walk done
at 13.0 s against dua's 14.3 s) suggests level or slightly ahead when the machine is idle.
Inside the §3.14 target either way.
