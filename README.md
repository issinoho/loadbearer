<img src="docs/assets/logo-mark.svg" alt="loadbearer logo" width="64" height="64">

# loadbearer

A command-line system-assessment tool. It benchmarks a host machine's **CPU**,
**memory** and **disk**, scores every measurement against an embedded reference
baseline, and grades each component — and the machine as a whole — on an S-to-F
scale. Run the same build on two laptops and it will tell you which one is
stronger, by how much, and *why*.

There is no GUI. Everything is driven by switches or a config file. In an
interactive terminal `loadbearer run` shows a TUI — live per-subtest progress, a
running overall gauge with an ETA, then a scrollable graded results screen. Piped
output, `--plain`, or `--json` fall back to plain text / machine-readable JSON, so
it works just as well from a script or CI job.

Primarily built for and released on **Windows** (a self-contained `.exe`, no
runtime to install — see [Install](#install)); Linux is supported as a
first-class runtime target and is what most development happens on.

This README covers install, usage and the scoring model at a working level. For
the deep dive — exactly what each benchmark kernel does, the scoring maths worked
through, `compare` internals, and how to recalibrate the baseline — see the
**[wiki](https://github.com/issinoho/loadbearer/wiki)**.

## What a run looks like

```
loadbearer 0.2.0 — assessment

  Machine   ThinkPad-X280 · Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz · 8 threads · 7.0 GiB RAM
  Profile   general · curve k=0.5 · baseline reference-v1 · short preset

  CPU         929  [B]   ███████████████░░░░░░░░░
    Integer, single-core          9204.6 Mops/s     0.92x     959  high
    Integer, all cores           38246.0 Mops/s     0.85x     922  high
    Float, single-core            7131.9 MFLOP/s    0.89x     944  high
    Float, all cores             55798.3 MFLOP/s    0.80x     893  high
    BLAKE3 hash                   3618.8 MiB/s      0.86x     928  high
    DEFLATE compress                58.4 MiB/s      0.86x     927  high

  MEMORY      814  [C]   █████████████░░░░░░░░░░░
    Sequential read                 14.1 GiB/s      0.64x     800  high
    Sequential write                 9.8 GiB/s      0.61x     781  medium
    Copy (memcpy)                    9.1 GiB/s      0.65x     808  medium
    Random access latency          125.4 ns         0.76x     870  high

  DISK        517  [D]   ████████░░░░░░░░░░░░░░░░
    Sequential write               319.7 MiB/s      0.21x     462  low
    Sequential read                349.1 MiB/s      0.12x     341  high
    Random 4K read               10461.7 IOPS       0.70x     835  high
    Random 4K write              11859.8 IOPS       0.30x     545  low

  NETWORK     755  [C]   ████████████░░░░░░░░░░░░
    TCP throughput, single stream         5.0 GiB/s      0.71x     841  high
    TCP throughput, all streams         7.0 GiB/s      0.35x     593  low
    TCP round-trip latency          18.8 us         0.74x     863  low
    UDP send rate                  256.9 Kpps       0.57x     756  high

  OVERALL     737  [C]   ████████████░░░░░░░░░░░░

  Why:
    - held back by Disk (score 517)
    - held back by Network (score 755)
    - low measurement confidence in: Disk, Network

  A score of 1000 = the reference-v1 baseline. Grades: S≥1400 A≥1150 B≥850 C≥600 D≥400.
```

Each subtest row is: raw measurement, its ratio to the baseline, its score, and a
confidence flag derived from run-to-run spread. In a terminal this is a coloured,
scrollable full-screen view with a live progress gauge while the run is in
progress; the block above is the `--plain` rendering.

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

The binary is unsigned, so Windows SmartScreen shows an "unrecognized app"
warning the first time — click **More info** then **Run anyway**. Put the folder
on your `PATH` if you want to call `loadbearer` from anywhere.

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
components. A letter grade is assigned from the score, and a short "why" names
the components that moved it and any low-confidence measurements.

The result is printed, and — with `--output` — written as a versioned JSON file
that `loadbearer compare` can diff against another machine's.

## Usage

```
loadbearer run        [OPTIONS]
loadbearer compare    FILE FILE [FILE ...] [--plain] [--json]
loadbearer info       [--json]
loadbearer list
loadbearer baseline   [FILE ...] [--name NAME] [--description TEXT]
loadbearer net-server [--bind ADDR]
```

- **`run`** — benchmark this machine, score it, print a graded assessment. TUI in
  an interactive terminal; plain text or JSON otherwise.
- **`compare`** — head-to-head of two or more result files: per-metric deltas,
  per-component and overall verdict. Built from the **raw** metrics, so it does
  not depend on the baseline or curve the files were scored with.
- **`info`** — machine inventory (host, CPU, memory, disks) and nothing else.
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
| `--only LIST` | Restrict the run to a comma-separated subset of benchmarks: `cpu`, `memory`, `disk`, `network`. Default: all four. |
| `--profile NAME` | Scoring profile that weights the overall grade: `general` (default), `dev-workstation`, `content-creation`, `server`. See [Profiles](#profiles). |
| `--duration PRESET` | Thoroughness: `short` (~10 s/benchmark, for quick checks and CI), `normal` (~30 s, default), `thorough` (~2 min, for a considered assessment). Trades wall-clock time for lower measurement variance. |
| `--curve-k FLOAT` | Display-curve exponent, 0.05–3.0 (default 0.5). Lower values compress the extremes toward 1000; higher values spread scores out. |
| `--target-dir PATH` | Directory for the disk benchmark's scratch file (default: the working directory). Point this at the disk you actually want to measure — **not** a `tmpfs`/RAM disk, where the numbers reflect memory, not storage (loadbearer detects this on Linux and says so). |
| `--runs N` | Override the number of timed iterations per subtest (default: 3 / 5 / 9 for short / normal / thorough). |
| `--seed N` | Seed for the pseudo-random workload data, for bit-for-bit reproducible inputs. |
| `--net-target HOST:PORT` | After the graded run, probe a real link (TCP upload, round-trip, UDP send rate) to a `loadbearer net-server` at this address. Reported in its own block and the result JSON's `link` field; **not graded** — it measures the network, not the machine. |
| `--output FILE` | Write the full result as a versioned JSON file. Works alongside the TUI or plain output. |
| `--plain` | Disable the TUI and emit the plain-text report. Implied automatically when stdout is not a terminal. |
| `--json` | Disable the TUI and emit only the result JSON to stdout. |
| `--config FILE` | Load defaults from a TOML config file. Explicit switches still win. See [Configuration](#configuration). |

While a TUI run is in progress: `q` cancels it cleanly (the current measurement
finishes, then the process exits). On the results screen: `↑`/`↓`/`PgUp`/`PgDn`
scroll, `q` or `Enter` exits.

### `compare` options

| Option | Description |
| --- | --- |
| `FILE ...` | Two or more result files written by `loadbearer run --output`. |
| `--plain` | Plain-text table (currently the default; a TUI view is planned). |
| `--json` | Emit the comparison as structured JSON. |

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

**Baseline.** Each raw measurement is compared against `reference-v1`, a set of
values for a hypothetical mid-range 2021 thin-and-light laptop (8-core mobile
CPU, dual-channel LPDDR4x, Gen3 NVMe). It is embedded in the binary and its
source is in [`baseline/reference-v1.toml`](baseline/reference-v1.toml). A
missing entry is a hard error, so the baseline cannot silently fall out of sync
with the benchmarks. **It is provisional** — synthetic anchors, not yet
calibrated against a fleet of real machines — so treat a single machine's
absolute score as indicative and lean on `compare` for head-to-head questions.

**Curve.** `score = 1000 · ratio^k`. With the default `k = 0.5`, a component
twice as fast as the baseline scores ~1414, half as fast ~707. Lower `k` is more
forgiving of a weak component; higher `k` rewards a strong one harder.

**Aggregation.** Component score = geometric mean of its subtest scores. Overall
score = geometric mean of the component scores, weighted by the profile. The
geometric mean means one very strong component can't paper over a weak one, and
ratios stay meaningful.

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
| `content-creation` | Favours CPU and memory bandwidth — encode, render; de-emphasises network. |
| `server` | Favours disk I/O, network and CPU — sustained throughput under load. |

## The benchmarks

| Component | Subtests |
| --- | --- |
| **CPU** | Integer throughput (single-core and all-core), floating-point throughput (single-core and all-core), BLAKE3 hashing, DEFLATE (level 6) compression. Integer and float kernels use eight independent accumulator lanes so they measure pipeline throughput, not dependency-chain latency. All-core subtests run the kernel on every logical CPU and sum the rates. |
| **Memory** | Sequential read, write and copy bandwidth over a working set sized past any last-level cache (256 MiB at `normal`); random-access latency via a single-cycle pointer chase (Sattolo) that defeats the prefetcher. Single-threaded. |
| **Disk** | Sequential write (each pass ends with `fsync`, so it's durable-write throughput) and read; random 4 KiB read and write IOPS at queue depth 1. Reads and random I/O use unbuffered I/O — `O_DIRECT` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows — to bypass the page cache, with a buffered fallback (and a recorded note) where the filesystem refuses it. The scratch file (1 GiB at `normal`) is filled with random data to defeat filesystem compression, reused by every subtest, and deleted when the run ends. |
| **Network** | Loopback (`127.0.0.1`) only — this measures the machine's network *stack* (syscall, TCP processing, scheduler wakeup latency), **not** a physical link, and makes no network calls. Single-stream and all-core TCP throughput, TCP request/response round-trip latency, and UDP small-packet send rate. For a real link test between two machines, run `loadbearer net-server` on one and `loadbearer run --net-target` on the other (reported separately, not graded). |

A single `run` writes on the order of a few GiB to `--target-dir` for the disk
benchmark; use `--only cpu,memory,network` to skip it.

## Result files

`loadbearer run --output result.json` writes a versioned
(`schema: "loadbearer.result/1"`) document containing:

- `machine` — the full inventory (as `loadbearer info --json`).
- `config` — profile, preset, curve-k, seed, thread count, baseline name.
- `raw` — every subtest's per-run values and summary statistics, unscored.
- `components` / `overall` — the scored, graded results.

Because the raw metrics are preserved, a result file can be re-scored later
against a different baseline or curve, and `compare` can work from it without
trusting the scores it was written with.

### Recalibrating the baseline

The shipped baseline is a placeholder. To build one from real hardware, collect
result files from machines you consider representative and average them:

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
  NTFS and ReFS; on other filesystems the read numbers may be cache-influenced,
  which `compare` and the report will note.

## Building and testing

```
cargo test            # unit tests across engine, scoring, compare, TUI state
cargo clippy --all-targets
cargo fmt --check
```

CI runs all three on Linux and Windows for every push and pull request. Tagged
`v*` pushes build release binaries for both and attach them to a GitHub Release
(see [`.github/workflows`](.github/workflows)).

## License

MIT — see [LICENSE](LICENSE).
