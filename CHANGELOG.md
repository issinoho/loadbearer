# Changelog

All notable changes to loadbearer are documented in this file.

## 1.1.1 - Sun, 30 Aug 2026

- **"vs typical hardware" now notes a preset mismatch.** The model reference
  table is measured at `--duration thorough`; a `run` at a shorter preset reads
  systematically high (or low, once warm) against it. The block now prints
  `(this run used the <preset> preset; … treat the gap as approximate)` when the
  run's preset isn't `thorough`, and the result JSON's `model_ref` entries carry
  a `run_preset` field (omitted when it was `thorough`). Data and matching are
  unchanged.

## 1.1.0 - Sun, 30 Aug 2026

- **Model reference table — "vs typical hardware".** loadbearer now ships a
  small table of per-CPU/GPU-model expected raw values
  (`baseline/models/cpu.toml`, `gpu.toml`, embedded), seeded from the seven
  machines that calibrated `reference-v1`. When a `run`'s CPU or GPU model
  matches, the report adds a **vs typical hardware** block: the geometric mean
  of the per-subtest deltas against that model's reference, plus a verdict
  (matches / below par — check thermals / above par). CPU and GPU only, and
  **not graded**. `--no-model-ref` skips it. The table is calibration data, not
  part of the stability contract (`VERSIONING.md`).
- **`loadbearer models`.** Prints the embedded table; `loadbearer models MODEL`
  shows one entry; `--as-result` emits a synthetic result file for a model;
  `--add FILE ...` regenerates the TOML from result files (geomean per model)
  for review and commit.
- **`compare --against MODEL`.** Adds a synthetic machine straight from the
  model reference table (CPU and GPU only), so a result can be compared against
  a chip you don't have. `compare` now accepts a single result file when
  `--against` supplies the second machine.
- **`config.build_isa` in result files.** Records the widest x86 instruction set
  the CPU kernels may use (`sse2` for the released build, `avx2` / `avx512` for
  a `target-cpu=native` build); model references only apply directly to an
  `sse2` build. Empty in files written before 1.1.

## 1.0.0 - Sat, 29 Aug 2026

First stable release. From here, [`VERSIONING.md`](VERSIONING.md) is the semver
contract: the CLI, the `schema`-tagged JSON formats and the exit codes are
covered; absolute scores, the reference baseline, the `--plain` layout and the
diagnostic-log format are not.

- **`reference-v1` recalibrated from real hardware.** The synthetic anchors —
  which every real machine graded C/D against — are replaced by the geometric
  mean of each metric across seven Intel machines (2015–2023, an i7-1370P down
  to a Celeron J4005) run at `--duration thorough`. Absolute scores shift
  substantially; it is a small, older-leaning sample, so a current mainstream
  machine now grades A/S. The `baseline/reference-v1.toml` header lists the
  machines; recalibrating again is a normal minor release.
- **Every machine-readable output is versioned.** `loadbearer compare --json`
  and `loadbearer mem --json` now carry `schema` (`loadbearer.compare/1`,
  `loadbearer.mem/1`) and `tool_version`, joining `loadbearer.result/1` and
  `loadbearer.soak/1`. Adding an optional field keeps the number; removing or
  retyping one bumps it.
- **Shell completions and a man page** are generated at build time and bundled
  in every release archive — `completions/` (bash, zsh, fish, PowerShell) and
  `loadbearer.1`.
- **Windows code signing** wired to [SignPath.io](https://signpath.io)'s free
  programme for open source: the release workflow signs `loadbearer.exe` when
  the SignPath secrets are configured, and ships it unsigned otherwise. The
  Windows install docs now cover SmartScreen, WDAC/AppLocker, and Smart App
  Control (which takes no allow rules at all).
- `VERSIONING.md` and a wiki *Stability* page; `loadbearer.1` and `VERSIONING.md`
  added to the archives.

## 0.10.0 - Sat, 29 Aug 2026

**The last `0.x` release.** The next release is `1.0.0`.

- **Diagnostic logging.** Every invocation now writes a plain-text log — one
  timestamped line per event: the resolved settings, each benchmark and subtest
  boundary, the GPU / battery / OpenCL probe outcomes, disk `O_DIRECT`
  fallbacks, the scratch-file sweep, the final grade, and any error.
- Written by default to `$XDG_CACHE_HOME/loadbearer/loadbearer.log` (Linux),
  `%LOCALAPPDATA%\loadbearer\loadbearer.log` (Windows), or the system temp dir;
  appended to and rotated to `loadbearer.log.old` past ~2 MiB. A log file that
  can't be opened is a one-line stderr note, not a failure.
- New global flags (valid on any subcommand): `--log-file PATH`, `--no-log`,
  `--log-level off|error|warn|info|debug|trace` (default `info`). The
  `LOADBEARER_LOG` env var does the same as `--log-level`; the flag wins.
  `debug` adds a line per subtest, `trace` a line per timed iteration.
- Log lines sit at lifecycle boundaries and fallbacks, never inside a timed
  measurement, so logging does not perturb the benchmark numbers even at the
  default level.

## 0.9.0 - Sat, 29 Aug 2026

- **New `loadbearer mem` command** — per-program memory use, in the style of
  [`ps_mem`](https://github.com/pixelb/ps_mem): grouped by program, smallest
  first, with a grand total. A diagnostic, not a benchmark — nothing here is
  scored.
- On **Linux** the figures are true **PSS** (proportional set size) from
  `/proc/<pid>/smaps_rollup`: a page shared by N processes counts 1/N toward
  each, so the per-program totals sum to close to the RAM actually in use.
  `Private + Shared = PSS`. Reading another user's process needs root; skipped
  processes are counted and the footer says how many.
- On **Windows** the figures are the process **working set**, split into
  `PrivateUsage` (capped at the working set) and an estimated `Shared`. The
  report footer states which kind of number is shown, so a PSS snapshot and a
  working-set snapshot are never conflated.
- Options: `--limit N` (show only the N largest; the total still covers all),
  `--swap` (Linux — a proportional paged-out column), `--json`.

## 0.8.0 - Fri, 29 Aug 2026

- **Battery inventory and health.** On a machine with a battery, `loadbearer
  info` now shows a `Battery` section — charge and state, health (present
  full-charge capacity as a fraction of the pack's design capacity), cycle
  count, voltage, technology, vendor/model — and a `run` report gains a
  `BATTERY` block with a one-line wear verdict (`as-new` / `healthy` / `worn` /
  `degraded` / `failing`). Read once at inventory time (sysfs on Linux, the
  battery IOCTL on Windows) and stored in the result JSON's `machine.battery`.
- Battery health is **never folded into a grade** — it's the condition of a
  consumable part, not the silicon, like the `network` and `gpu` components. A
  machine running **on battery power** during a run also gets a note, since a
  power profile may be capping clocks.
- A machine with **no battery** (desktop, server, most VMs) shows none of the
  above — nothing is skipped or errored, the section is simply absent.

## 0.7.8 - Fri, 29 Aug 2026

- The disk benchmark now sweeps a **stale scratch file** left by a previous run
  that was hard-killed before its cleanup could run. On start-up it removes any
  `.loadbearer-scratch.<pid>` in `--target-dir` that isn't this run's and hasn't
  been modified in 20 minutes (a concurrent run is never touched), and records a
  note. On a recurring schedule the ~1 GiB orphan is now reclaimed on its own.

## 0.7.7 - Fri, 29 Aug 2026

- `compare`: a delta that rounds to zero now prints as a plain `0%` instead of
  a signed `-0%` / `+0%` — the signed zero next to a `=` verdict read like a
  real difference.

## 0.7.6 - Fri, 29 Aug 2026

- Fix: `run --only gpu` (or `--only network`, or any selection with no graded
  component) reported `OVERALL 0 [F]` with a bogus "balanced — every component
  is close to the reference baseline" line. It now says **"no graded components
  in this run"** and shows no score/grade for the overall — the selected
  components are still measured and reported as normal. Latent since 0.3.0
  (`--only network`); surfaced while validating the GPU path on real hardware.

## 0.7.5 - Fri, 29 Aug 2026

- `SHA256SUMS` now also lists the hash of the **bare executable inside each
  archive**, not just the archive — that's the value a WDAC / AppLocker
  file-hash allow rule wants, so a policy can be written straight from the
  release page without extracting first. No binary change.

## 0.7.4 - Fri, 29 Aug 2026

- Releases now publish a **`SHA256SUMS`** file alongside the archives — for
  download verification and, on a locked-down Windows estate, a WDAC / AppLocker
  **file-hash** allow rule (hash rules permit an unsigned binary; publisher
  rules don't).
- The release workflow will **Authenticode-sign** the Windows binary when the
  repo has a code-signing cert configured (`WINDOWS_PFX_BASE64` /
  `WINDOWS_PFX_PASSWORD` secrets); with no secret it ships unsigned exactly as
  before. No binary or behaviour change in this version.

## 0.7.3 - Fri, 29 Aug 2026

Hardening for fleet / managed-estate deployment:

- **`--no-gpu`** — a global flag that skips the `gpu` component *and* the OpenCL
  probe that `info` and `run` otherwise perform, so `OpenCL.dll` is never
  loaded. Use it where a stale ICD loader (left by an uninstalled driver) might
  stall device enumeration.
- **Windows: the OpenCL loader is now taken only from `System32`**
  (`LOAD_LIBRARY_SEARCH_SYSTEM32`) — never the executable's directory, the
  working directory, or `%PATH%`. Closes a DLL-planting path.

## 0.7.2 - Fri, 29 Aug 2026

- `compare`: the `±%` delta now sits in its own right-aligned column instead of
  running straight on from the value (`12907  +40%`, with the deltas lining up
  down the table). Rollup rows put their delta in the same column. Machine
  columns widened a little to fit the split. TUI and plain both.

## 0.7.1 - Fri, 29 Aug 2026

- `compare`: widen the gap between the last machine's value column and the
  winner tag (`A` / `B` / `=`) from 2 spaces to 5, in both the TUI and the
  plain table — the tag was crowding the number.

## 0.7.0 - Fri, 29 Aug 2026

- **New GPU compute component.** Two figures via OpenCL: FP32 fused-multiply-add
  throughput (`GFLOP/s`) and VRAM read bandwidth (`GiB/s`). The strongest GPU is
  picked automatically (a discrete device beats an integrated one).
  - The OpenCL ICD loader (`OpenCL.dll` / `libOpenCL.so.1`) is loaded at
    runtime, not linked — a machine with no OpenCL, or no GPU, simply has no
    `gpu` component and the binary is unaffected. `--only gpu` on such a machine
    is a clear error rather than a silent no-op.
  - **Not folded into the overall grade**, like `network`: GPU is optional
    hardware, and a discrete-vs-integrated gap would swamp the "which machine is
    faster for my work" question. It is scored, shown, recorded in the result
    JSON, and used by `compare`.
  - `loadbearer info` gains a GPU section (model, type, memory, OpenCL version,
    compute units); the result file's `machine` inventory records it too.
  - `reference-v1` gains `[components.gpu]` anchors (integrated-GPU class —
    a discrete card scores well above 1000, which is fine for an ungraded
    component). Adds the `libloading` crate.
- `compare` now sets the totals off from the subtests: a rule under each
  component's rows before its `<component> total`, a heavier rule before
  `OVERALL`, and (in the TUI) both rendered bold. When two machines are close
  overall it was easy to lose the headline delta among the per-subtest rows.

## 0.6.6 - Thu, 28 Aug 2026

- The `compare` TUI's machine columns now widen from 15 up to whatever
  their `A: <name>` header needs (capped at 28), so a machine name like
  `ThinkPad-X280` shows in full in the column header instead of
  truncating to `A: ThinkPad-X2…` on a terminal with room to spare. The
  machine list at the top keeps the full names regardless.

## 0.6.5 - Thu, 28 Aug 2026

- The `compare` TUI now word-wraps the `!` warning lines and the verdict
  onto indented continuation lines instead of letting a long OS-mismatch
  warning run off the edge of the pane.

## 0.6.4 - Thu, 28 Aug 2026

- The live run screen now sizes its subtest-label column from the
  terminal width too (bounded 22–46), so names like `Sequential read,
  all cores` and `TCP throughput, single stream` show in full on a
  normal terminal instead of truncating at a fixed 22. The value column
  is now a fixed field — number right-aligned, unit left-aligned — so
  the values and confidence flags line up straight down the list
  regardless of magnitude or unit length.

## 0.6.3 - Thu, 28 Aug 2026

Carries the `compare`-TUI fixes back to the `run` TUI:

- The graded results screen sizes its subtest-label column from the
  terminal width instead of a fixed 24, so names like `Sequential read,
  all cores` and `TCP throughput, single stream` stop truncating on a
  normal-width terminal.
- The results scroll position is now clamped and written back each
  frame, so pressing `↓` at the bottom no longer banks phantom scroll
  that `↑` then has to unwind before anything moves.
- The results screen gains `j`/`k`/`space` and `Home`/`End` (`g`/`G`)
  to match the `compare` view, and the scroll hint shows the position.

## 0.6.2 - Thu, 28 Aug 2026

- The `compare` TUI now sizes its label column from the terminal width
  (bounded 22–48) instead of a fixed 30, so subtest names like
  `Sequential read, all cores (GiB/s)` stop losing their unit to
  truncation on a wide terminal. The reference column also highlights
  green on rows it wins, matching the other columns.

## 0.6.1 - Thu, 28 Aug 2026

- **`loadbearer compare` now has a TUI.** In an interactive terminal it opens a
  scrollable, colour-coded view of the head-to-head — winning cells and deltas
  in green, regressions in red, the reference column dimmed, plus the warnings,
  the `SUSTAINED LOAD` block and the verdict. `--plain` forces the old text
  table (still the default when stdout is not a terminal); `--json` is
  unchanged. `↑`/`↓`/`PgUp`/`PgDn`/`Home`/`End` (or `j`/`k`/`space`/`g`/`G`)
  scroll; `q` / `Esc` / `Enter` exits.

## 0.6.0 - Thu, 28 Aug 2026

- **New `loadbearer score` command.** Re-score an existing result file against a
  different baseline, profile or curve without re-running the benchmarks — the
  file keeps every raw measurement, so the grade is just one view of it that can
  be recomputed in a second.
  - `loadbearer score result.json --baseline our-fleet.toml` scores against a
    baseline you built with `loadbearer baseline`; `--profile` and `--curve-k`
    override the values the file was scored with; `--output` writes a new
    result file; `--json` emits it to stdout.
  - A short banner shows what changed (baseline / profile / curve / tool
    version) and the old → new overall.
  - Subtests the baseline has no entry for are left out of the score with a
    note, rather than being a hard error — a fleet baseline that omits the
    OS-dependent network component is a normal thing to re-score against. The
    file's full `raw` is preserved in the output.
- `Baseline::load` reads a baseline TOML from a path (used by `score --baseline`).

## 0.5.1 - Thu, 28 Aug 2026

- **The `--soak` phase now runs inside the TUI.** On an interactive `loadbearer
  run --soak`, the sustained-load phase gets its own live screen — a progress
  gauge, current throughput, current clock, retained-vs-peak-so-far, and a
  throughput sparkline that grows a mark per sample — instead of running as a
  plain stderr line after the TUI closed. `q` skips the soak and keeps the
  graded result; a soak that produced at least a few samples is still embedded
  in the result file. The results screen gains a one-line `SOAK` summary.

## 0.5.0 - Thu, 28 Aug 2026

- **New sustained-load / thermal soak test.** The graded benchmarks are all
  short bursts and measure a machine near its boost clocks; the soak test holds
  every logical CPU under a blended integer + floating-point load for a fixed
  stretch (default 90 s), samples aggregate throughput and CPU frequency once a
  second, and reports the unthrottled **peak**, the **steady-state** rate, the
  percentage **retained**, the **throttle onset** time, and steady-window
  stability. It is the signal that tells two thin-and-lights with identical
  burst numbers apart. **Not scored** — measured and shown, like the
  `--net-target` link probe.
  - `loadbearer soak [--duration SECS] [--threads N] [--output FILE] [--json]`
    runs it on its own.
  - `loadbearer run --soak [--soak-duration SECS]` appends it to a full
    assessment; the result is embedded in the result JSON under `soak`.
  - `loadbearer compare` shows a `SUSTAINED LOAD` block — absolute steady
    throughput and retained-vs-own-peak — when every result file carries soak
    data.
- Refinement to `memory/bw_read_mt` (0.4.0): the per-thread buffers now start
  their timed read together at a barrier (removing staggered-start skew, most
  visible at `--duration short`) and the per-thread floor rose 16 MiB → 32 MiB
  (insurance against a large shared L3 on server parts). The metric's meaning is
  unchanged; the baseline anchor stays at 28 GiB/s.

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
