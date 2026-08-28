# Changelog

All notable changes to loadbearer are documented in this file.

## 0.2.0 - Thu, 28 Aug 2026

- **Network is now a first-class graded component.** Four loopback subtests over
  `127.0.0.1` measuring the machine's network *stack*, not any physical link:
  single-stream and all-core TCP throughput, TCP request/response round-trip
  latency, and UDP small-packet send rate. No packets leave the machine; it
  needs no network access and no target.
- `loadbearer run --net-target HOST:PORT` optionally probes a real link (TCP
  upload, round-trip, UDP send rate) to a `loadbearer net-server` running on
  another machine. Reported in its own block and in the result JSON's `link`
  field — it measures the path, not either host, so it is deliberately **not
  graded**.
- `loadbearer net-server [--bind ADDR]` runs the server side of that link test.
- Scoring profiles gained a `network` weight: `server` now favours it (1.4),
  `content-creation` de-emphasises it (0.5).
- `reference-v1` baseline gained `[components.network]` anchors.

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
