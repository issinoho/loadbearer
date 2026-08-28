# Changelog

All notable changes to loadbearer are documented in this file.

## 0.4.0 - Thu, 28 Aug 2026

- **Two new CPU subtests: AES-256-GCM and SHA-256 throughput.** These pick up
  the CPU's AES-NI / CLMUL / SHA hardware where present (detected at runtime),
  so a chip that lacks crypto acceleration — common before ~2019 — now shows up
  in the grade. Adds the `aes-gcm` and `sha2` crates.
- **New memory subtest: `Sequential read, all cores`.** The other bandwidth
  subtests are single-threaded; this one runs the read kernel on every logical
  CPU and sums the rates, capturing whether the memory subsystem scales past
  one or two cores (many dual-channel laptops don't).
- `reference-v1` gains `cpu/aes_gcm`, `cpu/sha256` and `memory/bw_read_mt`
  anchors (provisional, like the rest).
- Because these join the CPU and memory geometric means, scores shift for
  machines that lack crypto acceleration or don't scale memory bandwidth.

## 0.3.1 - Thu, 28 Aug 2026

- **Single-threaded subtests are pinned to one core** while they run, so the OS
  scheduler can't bounce the measurement between core types (P/E cores on Intel
  hybrid CPUs, big.LITTLE on ARM) part-way through. On a 13th-gen Intel laptop
  this cut single-threaded memory-bandwidth run-to-run variance from `±40–60%`
  to a few percent. Each such iteration runs on a throwaway thread pinned to the
  fastest core (highest rated frequency on Linux; the first reported core
  elsewhere). All-core subtests are unaffected. Adds the `core_affinity` crate.

## 0.3.0 - Thu, 28 Aug 2026

- **The network component is no longer folded into the overall grade.** It is
  still a first-class component — measured, scored, shown, and used by
  `compare` — but `OVERALL` is now the geometric mean of CPU, memory and disk
  only. The loopback network figures depend heavily on the host OS and any
  security tooling (an EDR's packet inspection can add tens of microseconds per
  syscall), so folding them into a *hardware* grade produced misleading results
  — e.g. a fast Windows laptop grading F on network and dragging its overall
  down two letters. The result JSON gains a `graded` flag per component.
- `compare` now warns when result files are from different operating systems,
  since the network component in particular is not comparable across OSes.
- Scoring profiles no longer carry a `network` weight (it had no effect on the
  overall); `server`'s description no longer claims to favour it.

## 0.2.1 - Thu, 28 Aug 2026

- `loadbearer --version` / `-V` now report a full build version: the crate
  version plus the git commit, build date, target triple and profile, e.g.
  `loadbearer 0.2.1 (a1b2c3d4e 2026-08-28, x86_64-pc-windows-msvc, release)`.
  A `build.rs` captures this at compile time; it honours `SOURCE_DATE_EPOCH`
  for reproducible builds and falls back to `unknown` for the commit when built
  outside a git checkout.

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
