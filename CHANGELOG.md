# Changelog

All notable changes to loadbearer are documented in this file.

## 1.5.0 - Wed, 9 Sep 2026

Unattended-run controls for fleet use, and the results of actually measuring
whether the benchmark measures what it claims to. It mostly did; three graded
subtests didn't, and this release fixes two of them and documents the third.

- **⚠️ Graded values changed. 1.4.x and 1.5.0 scores are not comparable.**
  Four subtests now read differently, and the embedded `reference-v1` anchors
  have *not* been re-measured to match — they need all seven calibration
  machines re-run, so the baseline file records each as `KNOWN STALE` with
  what it would take to fix. Expect a modern machine to score high on
  `aes_gcm` (since 1.4.0), `int_multi`, `float_multi` and `memory.latency`.
  Use [`compare`](README.md#compare-options) for anything that has to be
  defensible: it works from raw metrics and doesn't touch the baseline.
- **Gates for when a run should *not* happen.** A graded run pins every core
  for minutes, which is unwelcome on someone's laptop mid-meeting and
  pointless on a machine running off a dying battery. `--not-on-battery`,
  `--if-idle`, `--skip-if-newer-than 7d` (don't re-benchmark when a
  deployment tool retries) and `--jitter 600` (don't have 500 machines hit one
  file share at once) let the caller say so. **A gate declining to run exits
  0** — deciding not to benchmark is the tool obeying instructions, and a
  fleet where every docked-at-lunchtime laptop shows as a failed deployment is
  a fleet where somebody turns the gates off. The reason goes to stderr and
  the log; what the gates observed lands in the result's new `gates` block, so
  a collector can filter for runs taken on mains and on an idle machine.
- **All-core CPU subtests now report their peak run, not their median — and
  this was a real defect.** An all-core series doesn't scatter around a
  centre, it decays: every core at full tilt holds boost for a few seconds
  and then drops to the package power limit. A median over that reports
  whichever regime straddles the middle sample, so `int_multi` was
  **bimodal** — two identical `--duration thorough --runs 9` runs measured
  103.5k and 72.7k Mops/s, 42 % apart, on nothing but thermal timing. It also
  explains the ~1.45× gap between `--duration short` and `thorough` (worth
  ~6 % of the CPU component score) and why per-run spread was erratic. Peak
  held to 6.8 % across presets and run counts where the median swung 53.6 %.
  How long a machine *sustains* all-core load is what [`soak`](README.md) is
  for, and it stays out of every grade.
- **Throttle detection worked on nothing.** `thermal_limited` compared the
  head and tail of the *mean* clock across logical CPUs; most subtests are
  single-threaded, so on a 20-thread machine nineteen idle cores held that
  mean flat. Across twenty real runs — several losing 40 % of their all-core
  throughput mid-subtest — it fired zero times. It now reads the busiest core
  in each sample. When it fires, peak-reported subtests have their confidence
  lowered, because a "peak" measured while throttling wasn't one; the reason
  goes into `notes`. Two limits, both measured and both documented: it can't
  see a machine that was *already* saturated when the run began (nothing
  declines — it started at the bottom), and it can't fire at all where the OS
  doesn't report real clocks, which on Windows and WSL2 it doesn't. Read a
  flat clock trace as "no data", not "no throttling".
- **Memory runs are 24 % faster at `--duration thorough`** (231 s → 176 s for
  the component). The pointer-chase cycle was rebuilt for every timed
  iteration, and shuffling a 134M-node array eleven times was most of the
  cost; it is now built once per subtest. That does move the figure —
  rebuilding left the array warm in cache from the shuffle's own writes, so
  the chase partly hit cache (189.3 → 200.6 ns on an unchanged footprint).
  The reused figure is the honest one, but it's why `memory.latency` is on the
  stale list above.
- **`memory/latency` is optimistic at `--duration short`, and isn't fixed.**
  The chase covers the preset-scaled working set — 128 MiB at `short` against
  512 MiB at `thorough` — and a shorter range reads ~9 % faster (0.902× and
  0.926× over two alternating pairs). Pinning the footprint was implemented,
  measured and reverted: it couldn't be shown to fix the bias and it doubled
  the run-to-run spread, since a large chase on a 350 ms budget completes too
  few traversals. Documented in the accuracy notes instead — don't read a
  `short` latency figure against a `thorough` one.
- **Two timing-sensitive tests no longer redden CI at random.**
  `link_probe_talks_to_a_local_server` gave the probe a 20 ms budget, which
  can mean a single iteration against a server thread that hasn't been
  scheduled yet; `abort_stops_the_run_early` asserted a flat 5 s on a 60 s
  soak while the rest of the suite saturated every core. Verified by
  reproducing both under one-CPU contention first.
- **Result files record which statistic produced each value.** Every subtest
  in `raw` gains `representative`, either `median` or `peak`. Additive and
  defaulted, so files written before 1.5.0 still parse (they were all
  medians), and `loadbearer.result/1` is unchanged.

## 1.4.0 - Tue, 8 Sep 2026

A dependency release, with one consequence big enough that it needs saying
first.

- **⚠️ `aes_gcm` roughly doubles on modern CPUs. Scores move.** The RustCrypto
  backend gained the **VAES** code paths in `aes` 0.9 — two or more AES blocks
  per instruction, where earlier versions used AES-NI one block at a time.
  Measured on an i7-1370P at `--duration thorough`, the preset the baseline
  itself was calibrated with: **1638.6 → 3626.6 MiB/s, +121%**, cv 0.56% and
  0.34%, both `high` confidence. An interleaved A/B at `--duration normal`
  independently gave +103%. Nothing about the benchmark changed; the old build
  simply wasn't using a capability the CPU had.

  This affects Intel Ice Lake / AMD Zen 3 and later. Older CPUs have no VAES,
  fall back to AES-NI, and are unchanged.

  What it means for you:
  - **CPU component and overall grades rise on modern hardware.** The
    component geomean dampens it, but it is not nothing.
  - **Don't compare an `aes_gcm` figure across this version boundary.** A 1.3.x
    number against a 1.4.0 number tells you about the crypto library, not the
    machine. `loadbearer compare` is fine as long as both sides ran the same
    build.
  - **The embedded `reference-v1` anchor for `aes_gcm` is now knowingly
    stale**, so a VAES-capable machine scores high on that one subtest. It has
    *not* been adjusted: the anchor is a geomean over seven machines and only
    two of them have VAES, so correcting it by modelling which ones changed
    would put an unverified number into calibration data. Fixing it properly
    means re-running all seven, and until then the file says so. Absolute
    scores and the baseline are explicitly outside the stability contract —
    see [VERSIONING.md](VERSIONING.md).

  Arguably the subtest is now more honest than it was: it claims to reflect
  "the crypto-instruction generation, not just the clock", and VAES is the
  current crypto-instruction generation.

- **Dependency majors.** `aes-gcm` 0.10 → 0.11, `sha2` 0.10 → 0.11,
  `libloading` 0.8 → 0.9, `windows-sys` 0.60 → 0.61, `clap_mangen` 0.2 → 0.3.
  Two needed code changes: `aes-gcm` deprecated `AeadInPlace` in favour of
  `AeadInOut::encrypt_inout_detached`, and `libloading` replaced the
  `AsRef<OsStr>` bound on `Library::new` with `AsFilename`, which the OpenCL
  loader's `&&str` no longer satisfied. The latter only broke non-Windows
  builds, since the Windows loader takes a different path.
- **Documented the crypto-library caveat** in the README accuracy notes, the
  baseline file itself, and the wiki's Accuracy Notes and The Baseline pages.
  `build_isa` doesn't capture this one, because the backend picks its AES path
  at runtime rather than at compile time. The README's capability list now
  mentions VAES alongside AES-NI, which it had missed.
- **Fixed a dead-code warning** on platforms with neither the Windows nor the
  Linux identity collector — a macOS build from source, which CI doesn't cover.

## 1.3.0 - Tue, 8 Sep 2026

A release about running loadbearer across a managed fleet rather than on one
machine at a time: knowing *which* machine a result came from, carrying the
organisational context a benchmark can't discover for itself, and surviving
the things a locked-down corporate desktop does to a benchmark. The
measurement kernels are untouched, so scores remain directly comparable with
1.2.x.

- **A machine identity in every result.** `machine.identity` now carries the
  SMBIOS system UUID, the chassis serial and asset tag, and the OS install's
  own id. `hostname` was the only correlation key a result file had, and in a
  managed fleet it's renameable, occasionally unset, and reissued between
  machines — so a collector had no reliable way to tell a repeat run of one
  machine from a machine it hadn't seen. The four identifiers fail in
  different ways on purpose: the firmware ones survive a reimage and match
  what asset, warranty and lease records key on, while the OS id is always
  readable but resets when the machine is reimaged. All are best-effort, and
  the block is left out entirely when nothing could be read (a container, a
  locked-down VM, a non-root Linux run). OEM placeholder serials like
  "To Be Filled By O.E.M." are rejected rather than recorded, since left in
  they collapse every unconfigured machine of a model into one fleet
  identity. On Windows this reads the registry and the raw SMBIOS table
  directly — no COM, no `wmic` subprocess — so it works in the Session 0
  context a deployment tool runs in.
- **`--tag key=value`.** Repeatable on `run` and `soak`, with a matching
  `[tags]` table in the config file, carried into the result file's `tags`
  field. loadbearer can read a machine's hardware but not its place in an
  organisation — which site it sits at, whose budget bought it, which
  deployment ring it's in — and that context already exists in whatever
  orchestrates the run. Switches override the config per key rather than
  replacing the set, so a config pushed fleet-wide can hold the constants
  while the command line adds what's specific to one run. Tags are metadata
  only and never reach a score, and they survive `score` re-grading.
- **A blocked optional subtest no longer ends the run.** A failure in a
  component that doesn't feed the grade — `network`, `gpu` — and a
  `--net-target` link probe that can't reach its target are now reported and
  skipped instead of aborting. Endpoint protection refusing the loopback
  socket the network component needs would previously throw away a completed
  CPU, memory and disk assessment, which is a poor trade for a component
  that is deliberately never graded. A *graded* component failing still
  stops the run, and every component being skipped is still a failure.
  **This changes an exit code**: a run whose `--net-target` probe failed used
  to exit 1 and now exits 0, because the run itself succeeded. Anything
  scripted against that needs to read the new `notes` field instead.
- **`notes` in the result file.** Records what was skipped and why, so a
  collector can tell a complete run from a complete-but-partial one instead
  of treating partial data as clean. Absent when the run was clean, and
  preserved through `score`.
- **`--fail-under GRADE`.** Exits **3** when the overall grade comes in below
  `GRADE`, so a management tool can flag slow hardware while keeping that
  distinct from exit 1, "this run broke". Opt-in for exactly that reason: on
  its own, a low grade is still a successful run and still exits 0. A run
  that grades nothing — `--only network` reports 0/F by construction — isn't
  judged at all and says so, rather than announcing a threshold breach that
  didn't happen. [VERSIONING.md](VERSIONING.md) now spells out which exit
  codes are covered by semver.
- **Fixed a flaky network test.** `link_probe_talks_to_a_local_server` bound
  an ephemeral TCP port and then assumed the same number was free on UDP. On
  a host running Hyper-V (WSL, Docker Desktop) it can land inside a dynamic
  UDP reservation, so the test failed at random — roughly one run in three on
  one affected machine — and could redden CI for reasons unrelated to the
  change under test.

## 1.2.4 - Tue, 8 Sep 2026

- **`--target-dir` and `--output` no longer need to exist first.** `run`
  created the disk scratch file with `create(true)`, which makes the file but
  not missing parent directories, so a fresh `--target-dir` (e.g. a scripted
  first run against `%ProgramData%\loadbearer` before that folder exists)
  failed with a bare "the system cannot find the path specified." `run`,
  `score` and `soak` now create the directory (or the output file's parent
  directory) first.

## 1.2.3 - Mon, 7 Sep 2026

A distribution release: new ways to install loadbearer and better ways to
check what you installed. The benchmark code is untouched since 1.2.2 —
`src/` is byte-for-byte identical — so scores and result files are directly
comparable across the two.

- **Debian/Ubuntu `.deb`.** Releases now attach
  `loadbearer_<version>-1_amd64.deb` alongside the archives:
  `sudo apt install ./loadbearer_1.2.3-1_amd64.deb` puts `loadbearer` on your
  `PATH` with the man page (`man loadbearer`) and bash/zsh/fish completions.
  It depends only on the C runtime. Built the way a distribution would build
  it — against the archive's own versioned Rust toolchain rather than rustup —
  and `lintian`-checked in CI.
- **Ubuntu PPA.** `sudo add-apt-repository ppa:issinoho/loadbearer` then
  `sudo apt install loadbearer`, after which upgrades arrive through `apt`
  with everything else. Built for 22.04 (jammy), 24.04 (noble) and 26.04
  (resolute), on amd64 and arm64. On arm64 the `aes_gcm` and `sha256` subtests
  read low — the build doesn't use Arm crypto instructions — the same caveat
  as Apple Silicon; the rest of the run is comparable.
- **GPG-signed `SHA256SUMS`.** CI now attaches a detached `SHA256SUMS.asc`
  signed with a key dedicated to loadbearer releases, for anyone who'd rather
  trust a key than GitHub's Sigstore instance. Fingerprint and public key are
  in [CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md). It covers the checksums
  as CI published them: the manual Windows re-sign regenerates `SHA256SUMS`
  and removes the now-stale signature rather than leaving it covering
  superseded hashes, so its absence on a release means that step has run.
- **Windows code signing, written down.** The `.exe` is Authenticode-signed
  with a Certum Open Source Code Signing certificate — by hand, shortly after
  each release publishes, not by CI (the cloud certificate has no unattended
  signing mode), so a release can be briefly unsigned right after tagging.
  [CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md) and the README now cover
  what that means for SmartScreen, Smart App Control and WDAC/AppLocker rules,
  and why the build-provenance attestation stops matching the Windows `.zip`
  once it's been re-signed.

## 1.2.2 - Fri, 4 Sep 2026

- **Intel Core Ultra 7 366H in the model reference table.** Adds a `[[cpu]]`
  entry to `baseline/models/cpu.toml` (16C/16T, from a 1.2.0 `--duration
  thorough` run), so a `run` on a Panther Lake H-series Core Ultra chip now
  gets a "vs typical hardware" block. CPU only: the iGPU enumerates as the
  generic "Intel Graphics", a name shared across every Core Ultra generation,
  so it can't key a GPU entry. Calibration data, not part of the stability
  contract (`VERSIONING.md`).

## 1.2.1 - Fri, 4 Sep 2026

- **Build provenance attestations.** Every release archive now carries a signed
  [build provenance
  attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
  binding it to the exact CI run, commit and workflow that produced it. Verify
  any download — Windows or Linux, signed or not — with
  `gh attestation verify <file> --repo issinoho/loadbearer`; no certificate
  needed, the trust root is GitHub's Sigstore. See
  [CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md).

## 1.2.0 - Fri, 4 Sep 2026

- **Informational subtests.** A new tier of measurement that is shown under its
  component as "informational (not graded)", carried in `raw` (with
  `"scored": false`) and usable by `compare`, but with no baseline entry and no
  effect on any score — for metrics the reference baseline hasn't been
  calibrated for. Grades and overall scores are unchanged; pre-1.2 result files
  still score.
- **Disk deep-queue random I/O** (`rand_read_qd` / `rand_write_qd`,
  informational). Several concurrent QD1 workers against one shared handle
  approximate a deep queue, so an NVMe drive that needs parallelism to shine is
  no longer indistinguishable from SATA.
- **Memory cache-latency ladder + latency under load** (informational).
  `lat_l1` / `lat_l2` / `lat_l3` are pointer chases over fixed 16 KiB / 256 KiB
  / 6 MiB working sets; `lat_loaded` is the DRAM-size chase run while the other
  cores stream reads. A single run now shows the L1→L2→L3→DRAM curve, not just
  one number.
- **CPU integer thread-scaling curve** (informational). `int_scale_2` … up to
  just under the logical CPU count, so you can see where scaling falls off
  (SMT, an E-core tier, a turbo/thermal wall) between the `int_single` and
  `int_multi` endpoints.
- **Clock / power telemetry.** Every run samples CPU frequency (all platforms)
  and Intel RAPL package power (Linux) and reports a one-line `Clocks …`
  summary plus a `telemetry` block in the JSON. A run whose clocks fall away is
  marked *thermally limited*, and the "vs typical hardware" verdict then warns
  that a low delta may be the cooling, not the chip. `--no-telemetry` opts out;
  macOS has no unprivileged source and reports `unavailable`.

## 1.1.4 - Tue, 1 Sep 2026

- **Apple M4 in the model reference table.** Adds `[[cpu]]` and `[[gpu]]`
  entries (`baseline/models/cpu.toml`, `gpu.toml`) from a 1.1.3 `--duration
  thorough` run on an M4 (10C/10T, 8-core GPU), so a `run` on a base M4 now
  gets a "vs typical hardware" block. Its `aes_gcm` / `sha256` figures are low
  because loadbearer's build doesn't use Arm crypto acceleration — a property
  of the tool, not the chip, same as the M3 Pro. Calibration data, not part of
  the stability contract (`VERSIONING.md`).

## 1.1.3 - Mon, 31 Aug 2026

- **Intel Core i7-1265U in the model reference table.** Adds a `[[cpu]]` entry
  to `baseline/models/cpu.toml` (10C/12T, from a 1.1.2 `--duration thorough`
  run), so a `run` on a 12th-gen Core U chip now gets a "vs typical hardware"
  block. No GPU entry: this part's Iris Xe shares a name with the P-series iGPU
  already in the table but runs well under it on a 15 W envelope, so folding the
  two together would misrepresent both. Calibration data, not part of the
  stability contract (`VERSIONING.md`).

## 1.1.2 - Sun, 30 Aug 2026

- **Apple M3 Pro in the model reference table.** First non-x86 entry
  (`baseline/models/cpu.toml`, `gpu.toml`), from a 1.1.1 `--duration thorough`
  run. Its `aes_gcm` / `sha256` figures are low because loadbearer's build
  doesn't use Arm crypto acceleration — a property of the tool, not the chip.
- **"vs typical hardware" no longer mis-flags Arm builds.** The "this is a
  wider build than the reference, expect higher" caveat now fires only for an
  x86 AVX/AVX2/AVX-512 build (`target-cpu=native`), not for any non-`sse2`
  `build_isa` — a portable Arm (NEON) run compares straight against the NEON
  reference.

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
