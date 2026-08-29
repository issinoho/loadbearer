<img src="docs/assets/logo-mark.svg" alt="loadbearer logo" width="64" height="64">

# loadbearer

[![Tiny Tool Town](https://img.shields.io/badge/Tiny_Tool_Town-featured-6f42c1?style=flat)](https://www.tinytooltown.com/tools/loadbearer)

A command-line system-assessment tool. It benchmarks a host machine's **CPU**,
**memory**, **disk** and **network stack** — and its **GPU** where there is one —
scores every measurement against an embedded reference baseline, and grades each
component and the machine as a whole on an S-to-F scale. Run the same build on
two laptops and it will tell you which one is stronger, by how much, and *why*.

It also runs a **sustained-load test** that shows how much speed a machine keeps
once it heats up, and can **re-grade** a saved result against a different
baseline without re-running anything.

There is no GUI. Everything is driven by switches or a config file. In an
interactive terminal `loadbearer run` shows a TUI — live per-subtest progress, a
running overall gauge with an ETA, then a scrollable graded results screen — and
`loadbearer compare` shows the head-to-head in a scrollable coloured view. Piped
output, `--plain`, or `--json` fall back to plain text / machine-readable JSON, so
it works just as well from a script or CI job. Every invocation also writes a
[diagnostic log](#diagnostic-logging) (`--no-log` to skip it).

Primarily built for and released on **Windows** (a self-contained `.exe`, no
runtime to install — see [Install](#install)); Linux is supported as a
first-class runtime target and is what most development happens on.

**Website:** <https://loadbearer.issinoho.com/> — the short version, with
screenshots.

This README covers install, usage and the scoring model at a working level. For
the deep dive — exactly what each benchmark kernel does, the scoring maths worked
through, `compare` internals, and how to recalibrate the baseline — see the
**[wiki](https://github.com/issinoho/loadbearer/wiki)**.

## What a run looks like

```
loadbearer 1.0.0 — assessment

  Machine   ThinkPad-X280 · Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz · 8 threads · 7.0 GiB RAM
  Profile   general · curve k=0.5 · baseline reference-v1 · thorough preset

  CPU         984  [B]   ████████████████░░░░░░░░
    Integer, single-core          6092.9 Mops/s     0.95x     973  high
    Integer, all cores           25180.3 Mops/s     1.43x    1194  high
    Float, single-core            4746.0 MFLOP/s    0.83x     914  high
    Float, all cores             36997.5 MFLOP/s    1.61x    1267  high
    BLAKE3 hash                   2384.6 MiB/s      1.27x    1128  high
    DEFLATE compress                38.8 MiB/s      0.87x     935  high
    AES-256-GCM encrypt            736.1 MiB/s      0.97x     984  high
    SHA-256 hash                   140.5 MiB/s      0.39x     628  high

  MEMORY      947  [B]   ███████████████░░░░░░░░░
    Sequential read                  8.9 GiB/s      0.83x     913  medium
    Sequential write                 8.3 GiB/s      1.21x    1101  medium
    Copy (memcpy)                    7.2 GiB/s      1.11x    1054  medium
    Sequential read, all cores        10.4 GiB/s      0.64x     798  medium
    Random access latency          181.8 ns         0.81x     901  high

  DISK        835  [C]   █████████████░░░░░░░░░░░
    Sequential write               175.5 MiB/s      0.51x     716  medium
    Sequential read                380.4 MiB/s      0.67x     816  high
    Random 4K read                8093.8 IOPS       0.99x     994  medium
    Random 4K write               7601.6 IOPS       0.70x     838  low

  NETWORK    1219  [A]   ████████████████████░░░░   · measured, not in the overall
    TCP throughput, single stream         3.2 GiB/s      1.63x    1275  medium
    TCP throughput, all streams           6.7 GiB/s      2.26x    1505  low
    TCP round-trip latency          54.6 us         0.73x     853  low
    UDP send rate                  169.1 Kpps       1.82x    1348  high

  GPU        1318  [A]   █████████████████████░░░   · measured, not in the overall
    FP32 compute (FMA)            1124.9 GFLOP/s    3.99x    1997  high
    VRAM read bandwidth             15.5 GiB/s      0.76x     869  medium

  OVERALL     920  [B]   ███████████████░░░░░░░░░

  Why:
    - held back by Disk (score 835)
    - low measurement confidence in: Disk

  A score of 1000 = the reference-v1 baseline. Grades: S≥1400 A≥1150 B≥850 C≥600 D≥400.
  Network and GPU are measured and shown, but kept out of the overall grade (OS-dependent / optional hardware).

  BATTERY  health 88% of design · 257 cycles · not graded
    Charge       86% (discharging)
    Health       42.0 / 48.0 Wh design (88%)
    Cycles       257
    Voltage      12.09 V
    → healthy — minor capacity loss
    note: on battery power — clocks may be capped; prefer mains for a clean grade
```

Each subtest row is: raw measurement, its ratio to the baseline, its score, and a
confidence flag derived from run-to-run spread. In a terminal this is a coloured,
scrollable full-screen view with a live progress gauge while the run is in
progress; the block above is the `--plain` rendering. The `GPU` block appears
only where there's a GPU and the `BATTERY` block only on a machine with a
battery — neither counts toward the grade.

This 2018 X280 lands **B** against `reference-v1` (a
[small, older-leaning sample](#scoring-model) — its all-core CPU and memory
figures actually clear the baseline). The rows that stand out are exactly the
kind of thing the subtests exist to surface: `SHA-256` at `0.39x` is the chip
lacking the SHA instruction extension, `Sequential read, all cores` (`0.64x`)
barely beating single-thread read (`0.83x`) is its dual-channel memory not
scaling, and the ungraded `FP32 compute` at `3.99x` just reflects how weak the
baseline's four-iGPU GPU anchor is.

## Requirements

- **Windows 10/11 (x86-64)** — the released `.exe` needs nothing else.
- **Linux (x86-64)** — a released binary, or build from source with a recent
  stable Rust toolchain (1.88+). `O_DIRECT` on the target filesystem gets you
  device-accurate disk numbers; loadbearer falls back to buffered I/O and says so
  when it can't.
- No admin/root privileges and no config required to run. Nothing leaves the
  machine unless you explicitly pass `--net-target`; the network benchmark itself
  is loopback only.

## Install

### Windows

Download `loadbearer-<version>-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/issinoho/loadbearer/releases/latest), unzip
it anywhere, and run `loadbearer.exe` from a terminal (PowerShell or Command
Prompt):

```
loadbearer.exe run
```

Put the folder on your `PATH` if you want to call `loadbearer` from anywhere.

**Smart App Control.** On Windows 11 with Smart App Control on (the default on
clean installs), a binary that isn't signed *and* known-good is blocked outright
— "An Application Control policy has blocked this file" — with **no allow-list
or file-hash exception**. Run it on a machine without SAC (Windows Sandbox works;
SAC doesn't apply inside it), or build from source. Releases are
Authenticode-signed through [SignPath.io](https://signpath.io)'s free programme
for open source (certificate by the [SignPath Foundation](https://signpath.org)),
but SAC also wants download reputation — a freshly-signed build may still be
blocked until that accrues.

**SmartScreen.** Without SAC, SmartScreen shows an "unrecognized app" prompt the
first time — **More info → Run anyway**, or `Unblock-File .\loadbearer.exe`. This
eases as the signing certificate accumulates reputation.

**Locked-down estates (WDAC / AppLocker).** These honour explicit rules: add a
**publisher** rule for the SignPath Foundation certificate, or a **file-hash**
rule for `loadbearer.exe` — the exact SHA-256 is in the release's `SHA256SUMS`,
or `Get-FileHash loadbearer.exe`. Hash rules work on an unsigned binary;
Smart App Control ignores both kinds of rule. For running it unattended across a
fleet (PDQ / Intune / GPO) see
[Fleet Deployment](https://github.com/issinoho/loadbearer/wiki/Fleet-Deployment)
in the wiki.

### Linux

Download `loadbearer-<version>-x86_64-unknown-linux-gnu.tar.gz` from the
[latest release](https://github.com/issinoho/loadbearer/releases/latest),
extract it, and run `./loadbearer`. Or install straight from source with Cargo:

```
cargo install --git https://github.com/issinoho/loadbearer --locked
```

### From source

```
git clone https://github.com/issinoho/loadbearer
cd loadbearer
cargo build --release
./target/release/loadbearer info
```

For CPU numbers that use your machine's widest vector instructions (AVX2/AVX-512
where available) rather than the portable SSE2 baseline, build with:

```
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

The absolute numbers go up; relative grades stay meaningful **as long as both
machines you're comparing are built the same way**.

### Shell completions and man page

Every release archive contains a `completions/` directory (`bash`, `zsh`,
`fish`, PowerShell) and `loadbearer.1`. Point your shell at the file for it —
e.g. `source completions/loadbearer.bash`, or copy `_loadbearer` onto your
`$fpath` for zsh — and `man ./loadbearer.1` for the manual. A source build
generates the same files next to the binary
(`target/release/loadbearer.1`, `target/release/loadbearer.bash`, …).

## How it works

`loadbearer run` executes each selected benchmark, one subtest at a time. A
subtest runs for a fixed wall-clock budget and counts how much work it completed
(operations, bytes, I/Os) — a "fixed time, measure throughput" shape that stays
well-scaled from a netbook to a workstation. Each subtest is run several times;
the median is taken as its value and the spread becomes a `high` / `medium` /
`low` confidence flag.

Every raw value is then divided by the matching number in the reference baseline
(`baseline/reference-v1.toml`, embedded in the binary; latency-style metrics are
inverted first) to give a **ratio**. The ratio goes through a display curve,
`score = 1000 · ratio^k` (`k` defaults to 0.5), so a machine that matches the
baseline everywhere scores **1000**. Component scores are the geometric mean of
their subtests; the overall score is a profile-weighted geometric mean of the
**CPU, memory and disk** components. Network is scored and shown but kept out of
the overall — its loopback figures depend too much on the host OS and any
security tooling to belong in a hardware grade. A letter grade is assigned from
the score, and a short "why" names the components that moved it and any
low-confidence measurements.

The result is printed, and — with `--output` — written as a versioned JSON file
that `loadbearer compare` can diff against another machine's.

## Usage

```
loadbearer run        [OPTIONS]
loadbearer compare    FILE FILE [FILE ...] [--plain] [--json]
loadbearer score      FILE [--baseline FILE] [--profile NAME] [--curve-k FLOAT] [--output FILE] [--json]
loadbearer soak       [--duration SECS] [--threads N] [--seed N] [--output FILE] [--json]
loadbearer info       [--json]
loadbearer mem        [--limit N] [--swap] [--json]
loadbearer list
loadbearer baseline   [FILE ...] [--name NAME] [--description TEXT]
loadbearer net-server [--bind ADDR]
```

- **`run`** — benchmark this machine, score it, print a graded assessment. TUI in
  an interactive terminal; plain text or JSON otherwise.
- **`compare`** — head-to-head of two or more result files: per-metric deltas,
  per-component and overall verdict. Built from the **raw** metrics, so it does
  not depend on the baseline or curve the files were scored with.
- **`score`** — recompute a result file's grade against a different baseline,
  profile or curve, without re-running anything. Point it at a baseline you
  built from your own hardware (`--baseline our-fleet.toml`) and the absolute
  score starts meaning something for your context; try `--profile server` or
  `--curve-k 0.7` to see how the knobs move it. Subtests the baseline doesn't
  cover are left out with a note. `--output` writes the re-scored file.
- **`soak`** — hold every core under sustained load (default 90 s) and report
  how much throughput the machine *retains* once the thermal mass saturates and
  the power limit bites: peak vs steady-state rate, percentage retained, when
  throttling set in, and how steady the clock held. **Not scored** — it's the
  signal that separates two laptops with identical burst numbers. Also available
  as `loadbearer run --soak`, which embeds the result in the result JSON.
- **`info`** — machine inventory (host, CPU, memory, disks, and the GPU and
  battery where present) and nothing else.
- **`mem`** — per-program memory use right now, in the style of
  [`ps_mem`](https://github.com/pixelb/ps_mem): grouped by program, smallest
  first, with a grand total. On Linux the numbers are true **PSS**
  (proportional set size — shared pages counted once, split across their
  sharers), read from `/proc/<pid>/smaps_rollup`; on Windows they're the
  **working set**, split into private and an estimated shared. A diagnostic,
  not a benchmark — nothing here is scored.
- **`list`** — the available benchmarks, the active baseline, and the scoring
  profiles.
- **`baseline`** — with no arguments, prints the built-in baseline. Given result
  files, emits a new baseline TOML whose values are the geometric mean of each
  metric across those files (see [Recalibrating](#recalibrating-the-baseline)).
- **`net-server`** — runs the server side of the optional `--net-target` link
  test; leave it running on one machine and point another machine's
  `loadbearer run --net-target` at it. Listens on `0.0.0.0:47913` by default.

### `run` options

| Option | Description |
| --- | --- |
| `--only LIST` | Restrict the run to a comma-separated subset of benchmarks: `cpu`, `memory`, `disk`, `network`, `gpu`. Default: CPU / memory / disk / network, plus GPU when one is present. |
| `--profile NAME` | Scoring profile that weights the overall grade: `general` (default), `dev-workstation`, `content-creation`, `server`. See [Profiles](#profiles). |
| `--duration PRESET` | Thoroughness: `short` (~10 s/benchmark, for quick checks and CI), `normal` (~30 s, default), `thorough` (~2 min, for a considered assessment). Trades wall-clock time for lower measurement variance. |
| `--curve-k FLOAT` | Display-curve exponent, 0.05–3.0 (default 0.5). Lower values compress the extremes toward 1000; higher values spread scores out. |
| `--target-dir PATH` | Directory for the disk benchmark's scratch file (default: the working directory). Point this at the disk you actually want to measure — **not** a `tmpfs`/RAM disk, where the numbers reflect memory, not storage (loadbearer detects this on Linux and says so). |
| `--runs N` | Override the number of timed iterations per subtest (default: 3 / 5 / 9 for short / normal / thorough). |
| `--seed N` | Seed for the pseudo-random workload data, for bit-for-bit reproducible inputs. |
| `--net-target HOST:PORT` | After the graded run, probe a real link (TCP upload, round-trip, UDP send rate) to a `loadbearer net-server` at this address. Reported in its own block and the result JSON's `link` field; **not graded** — it measures the network, not the machine. |
| `--soak` | After the graded run, hold every core under sustained load and report throughput retention (thermal / power-limit throttling). Adds ~90 s. Reported in its own block and the result JSON's `soak` field; **not graded**. |
| `--soak-duration SECS` | Duration for `--soak` (default 90; range 15–1800). |
| `--no-gpu` | Never touch the GPU: skip the `gpu` component **and** the OpenCL probe that `info` / `run` otherwise perform, so `OpenCL.dll` is never loaded. A global flag — works with any subcommand. Useful for fleet deployment where a stale ICD loader could stall enumeration. |
| `--output FILE` | Write the full result as a versioned JSON file. Works alongside the TUI or plain output. |
| `--plain` | Disable the TUI and emit the plain-text report. Implied automatically when stdout is not a terminal. |
| `--json` | Disable the TUI and emit only the result JSON to stdout. |
| `--config FILE` | Load defaults from a TOML config file. Explicit switches still win. See [Configuration](#configuration). |

While a TUI run is in progress: `q` cancels it cleanly (the current measurement
finishes, then the process exits). On the results screen: `↑`/`↓`/`PgUp`/`PgDn`
(or `j`/`k`/`space`) scroll, `Home`/`End` (or `g`/`G`) jump to top/bottom, `q` or
`Enter` exits — the same keys as the `compare` view.

### `compare` options

| Option | Description |
| --- | --- |
| `FILE ...` | Two or more result files written by `loadbearer run --output`. |
| `--plain` | Force the plain-text table. The default is a scrollable TUI when stdout is a terminal, the plain table otherwise. |
| `--json` | Emit the comparison as structured JSON (`schema` `"loadbearer.compare/1"`, `tool_version`, `machines`, `components`, `overall`, `warnings`, `soak`). |

The first file is the reference; every other machine's metrics are shown as a
direction-adjusted percentage relative to it (`+28%` always means "better").
`compare` warns when the files used different presets, baselines or curves, and
skips any component or subtest that isn't present in every file.

```
  metric                           A: thinkpad-x280 B: precision-5560
  CPU
  Integer, single-core (Mops/s)               9238       13396 +45%  B
  Integer, all cores (Mops/s)                37346       54152 +45%  B
    → component                                ref            +45%  B
  MEMORY
  Sequential read (GiB/s)                      12.8        15.1 +18%  B
  Random access latency (ns)                  145.3       123.1 +18%  B
    → component                                ref            +18%  B

  OVERALL                                      ref            +31%  B

  Verdict: precision-5560 leads by 31% overall (ahead on cpu +45%, memory +18%).
```

When every result file carries `--soak` data, `compare` adds a `SUSTAINED LOAD`
block: absolute steady-state throughput (with a delta to the reference machine)
and each machine's steady-state as a percentage of *its own* peak. It is not
folded into the verdict.

### `score` options

| Option | Description |
| --- | --- |
| `FILE` | A result file written by `loadbearer run --output`. |
| `--baseline FILE` | Baseline TOML to score against (as written by `loadbearer baseline`). Default: the built-in `reference-v1`. |
| `--profile NAME` | Scoring profile. Default: the profile recorded in the file. |
| `--curve-k FLOAT` | Display-curve exponent, 0.05–3.0. Default: the value in the file. |
| `--output FILE` | Write the re-scored result as a new JSON file. |
| `--json` | Emit the re-scored result as JSON to stdout instead of a report. |

```
re-scoring thinkpad-x280 (2026-08-28T15:45:20Z)
  tool      0.4.0  →  1.0.0
  baseline  reference-v1  →  our-fleet
  profile   general  →  server
  curve k   0.5  →  0.7
  overall   809 [C]  →  976 [B]
```

The scored `components` reflect only the subtests the baseline covers; the
file's full `raw` is preserved in the `--output` file, so it can be re-scored
again later.

### `soak` options

| Option | Description |
| --- | --- |
| `--duration SECS` | Sustained-load duration (default 90; range 15–1800). |
| `--threads N` | Worker threads (default: one per logical CPU). |
| `--seed N` | Seed for the sustained-load kernel. |
| `--output FILE` | Write the soak result as a `loadbearer.soak/1` JSON document. |
| `--json` | Emit only the JSON to stdout. |

```
  SOAK      90s · 20 threads · not graded
    Peak               68420 Mops/s   (2–5s)
    Steady             55110 Mops/s   (68–90s)   80.5% retained
    Throttle     onset ~22s (first sustained drop below 95% of peak)
    Stability    steady-window CV 1.4%
    Clock        3.90 GHz peak → 2.70 GHz steady
    Trace        ▇█▇▆▅▅▄▄▄▄▃▃▃▃▃▃▃▃▃▃  (≈1s/mark)
    → throttles from ~22s; settles at 80% of peak
```

The load is a blended integer + floating-point kernel that stays in registers
(no memory traffic), run on every logical CPU at once — enough to hit a
thin-and-light's sustained power limit. A build with `-C target-cpu=native`
pushes harder still. `Retained` is the number to compare: a machine that holds
90% of its peak for 90 s will out-work one that holds 65%, even if the second
has the higher burst.

### `mem` options

| Option | Description |
| --- | --- |
| `--limit N` | Show only the N largest programs. The grand total still covers every program. |
| `--swap` | Add a `Swap` column (Linux only — the proportional paged-out size, `SwapPss`). |
| `--json` | Emit the snapshot as JSON (`schema` `"loadbearer.mem/1"`, `tool_version`, `source`, `programs[]`, `unreadable`) instead of the table. |

```
loadbearer 1.0.0 — memory by program

    Private +     Shared =   RAM used   Program

    6.0 MiB +   43.0 KiB =    6.0 MiB   loadbearer
   33.0 MiB +    4.2 MiB =   37.2 MiB   ptyxis
  208.9 MiB +   10.4 MiB =  219.4 MiB   gnome-shell
  643.2 MiB +   74.0 KiB =  643.2 MiB   claude (2)
    1.9 GiB +  103.2 MiB =    2.0 GiB   firefox (17)
 ------------------------------------
                              3.2 GiB
 ====================================
 PSS from /proc/<pid>/smaps_rollup — shared pages counted proportionally.
 50 process(es) not readable — run as root for the full total.
```

On Linux, reading another user's process needs root, so an unprivileged run
sees only its own processes and the total is short by the rest — the footer
says how many were skipped. `Private + Shared = RAM used` is PSS: a shared
library mapped by 40 processes counts about 1/40 toward each, so the per-program
totals sum to something close to real RAM in use. Windows has no PSS; there
`RAM used` is the working set and `Shared` is an estimate.

## Diagnostic logging

Every invocation writes a diagnostic log — the resolved settings, each
benchmark and subtest boundary, the GPU / battery / OpenCL probe results, disk
`O_DIRECT` fallbacks, the scratch-file sweep, the final grade, and any error.
It's a plain text file, one timestamped line per event:

```
2026-08-29T10:12:18.480Z INFO  loadbearer::run       resolved settings: profile=general, preset=Short, …
2026-08-29T10:12:18.515Z INFO  loadbearer::gpu       probe: selected Intel(R) UHD Graphics 620 · integrated · …
2026-08-29T10:12:20.276Z DEBUG loadbearer::engine    subtest cpu/int_single done: median 548.8 Mops/s (cv 0.5%, high)
2026-08-29T10:12:30.427Z INFO  loadbearer::scoring   overall 188 [F] · profile general · … · 1 graded component(s)
```

| Where | |
| --- | --- |
| Default path | `$XDG_CACHE_HOME/loadbearer/loadbearer.log` (Linux), `%LOCALAPPDATA%\loadbearer\loadbearer.log` (Windows), else the system temp dir. Rotated to `…/loadbearer.log.old` once it passes ~2 MiB. |
| `--log-file PATH` | Write here instead. |
| `--no-log` | Don't write a log at all. |
| `--log-level LEVEL` | `off` / `error` / `warn` / `info` (default) / `debug` / `trace`. `debug` adds a line per subtest; `trace` adds a line per timed iteration. |
| `LOADBEARER_LOG` | Same as `--log-level`, for when you can't pass a flag. The flag wins. |

All four are global — they work on any subcommand. Logging is placed at
lifecycle boundaries and fallbacks, never inside a timed measurement, so it
doesn't perturb benchmark numbers even at `info`. If the log file can't be
opened the run still proceeds (with a one-line note on stderr).

## Configuration

`run` settings resolve in this order: **command-line switch → `--config` file →
built-in default**. The config file is TOML; every key is optional and unknown
keys are rejected. A full example ships as
[`loadbearer.example.toml`](loadbearer.example.toml):

```toml
profile   = "dev-workstation"
duration  = "thorough"
curve_k   = 0.5
target_dir = "/var/tmp"
# only    = ["cpu", "memory"]
# runs    = 7
# seed    = 42
```

## Scoring model

**Baseline.** Each raw measurement is compared against `reference-v1`, the
geometric mean of that metric across seven real machines (Intel, 2015–2023) run
at `--duration thorough` — the machine list is in the header of
[`baseline/reference-v1.toml`](baseline/reference-v1.toml). It is embedded in
the binary; a missing entry is a hard error, so the baseline cannot silently
fall out of sync with the benchmarks. It is a **small, Intel-only sample skewed
toward older low-power laptops**, so the anchors sit low — a current mainstream
machine grades A/S. Treat a single machine's absolute score and letter as a
rough position, lean on `compare` for head-to-head questions, and see
[`VERSIONING.md`](VERSIONING.md): the baseline is calibration data, not part of
the stability contract, and will be recalibrated as more machines are measured.

**Curve.** `score = 1000 · ratio^k`. With the default `k = 0.5`, a component
twice as fast as the baseline scores ~1414, half as fast ~707. Lower `k` is more
forgiving of a weak component; higher `k` rewards a strong one harder.

**Aggregation.** Component score = geometric mean of its subtest scores. Overall
score = geometric mean of the **CPU, memory and disk** component scores,
weighted by the profile. The geometric mean means one very strong component
can't paper over a weak one, and ratios stay meaningful.

**Network and GPU are not in the overall.** Both are scored and displayed like
the others, and `compare` uses their raw metrics, but neither counts toward the
grade. Loopback network depends heavily on the host OS (Windows has no in-kernel
loopback fast path) and on any security tooling — an EDR inspecting loopback
packets can add tens of microseconds per syscall — so it's not a hardware
signal. GPU is optional hardware: folding a discrete card vs an iGPU into the
grade would drown out the CPU/memory/disk answer most comparisons are actually
after. `compare` warns when two result files are from different operating
systems, for the network reason.

**The `--net-target` link probe and the `--soak` sustained-load test are not
scored at all** — the link probe measures the path between two hosts, and the
soak test reports throughput *retention* (a property, not a speed). Both are
shown in their own block, stored in the result JSON, and used by `compare` in a
separate block, but neither touches a grade.

**Battery health** is read into the inventory on a machine that has a battery
(design vs current full-charge capacity, cycle count, charge, technology) and a
`run` prints a `BATTERY` block with a wear verdict. It is pack condition, not
machine speed, so it never touches a grade; a machine on battery power also gets
a note, since a power profile may be capping clocks. A machine with no battery
shows none of this.

**Grades.** `S ≥ 1400`, `A ≥ 1150`, `B ≥ 850`, `C ≥ 600`, `D ≥ 400`, else `F` —
centred so the baseline (1000) lands in the middle of B.

**Confidence.** Each subtest's coefficient of variation across its timed runs
becomes `high` (< 3%), `medium` (< 8%) or `low`. Components inherit the weakest
flag among their subtests, and low-confidence components are called out in the
"why". Use `--duration thorough` to tighten a noisy result.

### Profiles

| Profile | Weighting |
| --- | --- |
| `general` | CPU, memory and disk count equally (default). |
| `dev-workstation` | Favours CPU and disk — builds, containers, version control. |
| `content-creation` | Favours CPU and memory bandwidth — encode, render; de-emphasises disk. |
| `server` | Favours disk I/O and CPU — sustained throughput under load. |

## The benchmarks

| Component | Subtests |
| --- | --- |
| **CPU** | Integer and floating-point throughput (single-core and all-core), BLAKE3 hashing, DEFLATE (level 6) compression, **AES-256-GCM** and **SHA-256** throughput. The integer/float kernels use eight independent accumulator lanes so they measure pipeline throughput, not dependency-chain latency. AES-GCM and SHA-256 pick up the CPU's AES-NI / CLMUL / SHA-extension hardware at runtime where it exists. All-core subtests run the kernel on every logical CPU and sum the rates. |
| **Memory** | Sequential read, write and copy bandwidth over a working set sized past any last-level cache (256 MiB at `normal`), plus an **all-core** read that sums the read kernel across every logical CPU; random-access latency via a single-cycle pointer chase (Sattolo) that defeats the prefetcher. Single-threaded except the all-core read. |
| **Disk** | Sequential write (each pass ends with `fsync`, so it's durable-write throughput) and read; random 4 KiB read and write IOPS at queue depth 1. Reads and random I/O use unbuffered I/O — `O_DIRECT` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows — to bypass the page cache, with a buffered fallback (and a recorded note) where the filesystem refuses it. The scratch file (1 GiB at `normal`) is filled with random data to defeat filesystem compression, reused by every subtest, and deleted when the run ends. A hard-killed run leaves it behind; the next run against the same directory sweeps any orphan that isn't its own and hasn't been touched in 20 minutes. |
| **Network** | Loopback (`127.0.0.1`) only — this measures the machine's network *stack* (syscall, TCP processing, scheduler wakeup latency), **not** a physical link, and makes no network calls. Single-stream and all-core TCP throughput, TCP request/response round-trip latency, and UDP small-packet send rate. **Scored and shown, but not in the overall grade** (see [Scoring model](#scoring-model)). For a real link test between two machines, run `loadbearer net-server` on one and `loadbearer run --net-target` on the other (reported separately, also not graded). |
| **GPU** | FP32 fused-multiply-add throughput (GFLOP/s) and VRAM read bandwidth (GiB/s), via OpenCL. The strongest GPU is picked automatically (discrete beats integrated). The OpenCL loader is opened at runtime, not linked — on Windows **only from `System32`** (`LOAD_LIBRARY_SEARCH_SYSTEM32`), so a planted `OpenCL.dll` can't be picked up. No GPU or no OpenCL means no `gpu` component, and the binary is unaffected. **Scored and shown, but not in the overall grade**: GPU is optional hardware and a discrete-vs-integrated gap would swamp the "faster for my work" question. Runs only when a GPU is present, or on explicit `--only gpu`; `--no-gpu` disables it (and the probe) entirely. |
| **Sustained load** *(opt-in: `soak` / `run --soak`)* | Holds every logical CPU under a blended integer + floating-point kernel for a fixed stretch (default 90 s), sampling aggregate throughput and CPU clock once a second. Reports the unthrottled peak, the steady-state rate, the percentage retained, when throttling set in, and steady-window stability. **Not scored** — it's a measure of how well a machine holds up under a long workload once it heats up, not of raw speed. |

A single `run` writes on the order of a few GiB to `--target-dir` for the disk
benchmark; use `--only cpu,memory,network` to skip it.

## Result files

`loadbearer run --output result.json` writes a versioned
(`schema: "loadbearer.result/1"`) document containing:

- `machine` — the full inventory (as `loadbearer info --json`), including the
  GPU and battery when the machine has them.
- `config` — profile, preset, curve-k, seed, thread count, baseline name.
- `raw` — every subtest's per-run values and summary statistics, unscored.
- `components` / `overall` — the scored, graded results.
- `link` — the `--net-target` link probe, if one ran (ungraded).
- `soak` — the `--soak` sustained-load result, if one ran (ungraded): every
  per-second sample plus the derived peak / steady / retained / onset figures.

Because the raw metrics are preserved, a result file can be re-scored later
against a different baseline, profile or curve with [`loadbearer
score`](#score-options), and `compare` can work from it without trusting the
scores it was written with.

### Recalibrating the baseline

`reference-v1` is a small sample (see [Scoring model](#scoring-model)). To
re-anchor it to hardware you care about — your own fleet, or a wider spread —
collect result files from representative machines and average them:

```
loadbearer run --output ref-laptop-1.json
loadbearer run --output ref-laptop-2.json
loadbearer baseline ref-laptop-1.json ref-laptop-2.json \
  --name reference-v2 --description "our 2026 standard-issue laptops" \
  > baseline/reference-v1.toml
cargo build --release
```

Each value in the generated file is the geometric mean of that metric across the
inputs; subtests missing from some inputs are flagged on stderr.

## Accuracy notes

- **Build both sides the same way.** A `target-cpu=native` build and a portable
  build produce different CPU numbers; only compare like with like.
- **Point `--target-dir` at real storage.** On a `tmpfs`/RAM disk the disk
  scores measure memory bandwidth. loadbearer detects this on Linux and adds a
  note; elsewhere it's on you.
- **Thermals and background load matter.** A thin laptop throttling under
  sustained load is a real property of that machine — but if you want a clean
  number, run on mains power, let it cool between runs, and close other work. The
  confidence flags exist to tell you when a result was jittery.
- **Windows `O_DIRECT`-equivalent** (`FILE_FLAG_NO_BUFFERING`) is honoured by
  NTFS and ReFS (verified — the disk numbers are real device speed, not cache);
  on other filesystems the read numbers may be cache-influenced, which the report
  will note.
- **Security tooling / EDR** (CrowdStrike, SentinelOne, Defender ATP, …) inspects
  every loopback packet, which adds tens of microseconds per network syscall and
  noticeable jitter across the board. This is the main reason the **network
  component is not in the overall grade**. Expect a low network score and
  `medium`/`low` confidence on managed machines; `--duration thorough` and
  repeated runs help, and the confidence flags will flag it.

## Building and testing

```
cargo test            # unit tests across engine, scoring, compare, TUI state
cargo clippy --all-targets
cargo fmt --check
```

CI runs all three on Linux and Windows for every push and pull request. Tagged
`v*` pushes build release binaries for both and attach them to a GitHub Release
(see [`.github/workflows`](.github/workflows)).

## Stability

From 1.0.0, loadbearer follows semantic versioning for its **CLI** (subcommands
and flags), the four `schema`-tagged JSON formats (`loadbearer.result/1`,
`loadbearer.soak/1`, `loadbearer.compare/1`, `loadbearer.mem/1`), and its
**exit codes** — a breaking change to any of those is a major release.

**Not** covered, and free to change in any release: absolute score values and
letter grades, the reference baseline, the `--plain` text layout and the TUI,
and the diagnostic-log format. The full contract is in
[`VERSIONING.md`](VERSIONING.md).

## Contributing

Bug reports, small fixes and well-scoped features are welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md) for the project layout, the checks CI
enforces, and how to add a benchmark. All participants follow the
[Code of Conduct](CODE_OF_CONDUCT.md). Security reports go through
[SECURITY.md](SECURITY.md), not the public tracker.

loadbearer collects no data and sends nothing anywhere — see
[PRIVACY.md](PRIVACY.md).

## License

MIT — see [LICENSE](LICENSE).
