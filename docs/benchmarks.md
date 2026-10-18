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

Cold cache not measured yet: it needs `sudo purge` before each run, e.g.
`hyperfine -N -i --runs 3 --prepare 'sudo purge' 'dua aggregate ~' './target/release/rsdisk-core scan ~'`.
