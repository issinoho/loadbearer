# Changelog

All notable changes to loadbearer are documented in this file.

## 0.1.0 - Thu, 28 Aug 2026

First release.

- `loadbearer run` benchmarks CPU (integer and floating-point throughput single-
  and all-core, BLAKE3 hashing, DEFLATE compression), memory (sequential
  read/write/copy bandwidth, random-access latency) and disk (sequential
  read/write, random 4K read/write IOPS at queue depth 1), scores each metric
  against an embedded reference baseline, and grades every component and the
  machine as a whole on an S-to-F scale.
- Interactive terminals get a TUI with live per-subtest progress, an overall
  gauge with ETA, and a scrollable graded results screen; `q` cancels a run
  cleanly. Piped output, `--plain` and `--json` use text / JSON instead.
- `loadbearer compare` produces a head-to-head verdict between two or more result
  files, computed from the raw metrics so it is independent of the baseline and
  curve each file was scored with. It warns on mismatched presets/baselines and
  skips metrics not present in every file.
- `loadbearer info` prints the machine inventory; `loadbearer list` prints the
  benchmarks, baseline and profiles; `loadbearer baseline` prints the built-in
  baseline or regenerates one from result files.
- `run` settings resolve command-line switch, then `--config` TOML file, then
  built-in default. Scoring profiles: `general`, `dev-workstation`,
  `content-creation`, `server`.
- Disk reads and random I/O use unbuffered I/O (`O_DIRECT` on Linux,
  `FILE_FLAG_NO_BUFFERING` on Windows) to bypass the page cache, with a buffered
  fallback and a RAM-disk guard.
- Result files are versioned (`loadbearer.result/1`) and keep the full unscored
  raw metrics alongside the scored output.
