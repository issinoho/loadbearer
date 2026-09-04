# Versioning and stability

From **1.0.0**, loadbearer follows [semantic versioning](https://semver.org/)
for the surface described here. A release that breaks anything under **Covered**
gets a **major** bump; new subcommands or capabilities are **minor**; fixes and
polish are **patch**.

## Covered by semver

A breaking change to any of these is a major release:

- **The CLI.** The set of subcommands (`run`, `compare`, `score`, `soak`,
  `info`, `mem`, `list`, `baseline`, `models`, `net-server`) and their flags.
  Removing a subcommand or a flag, renaming one, or changing what a flag takes
  is breaking. Adding a subcommand or an optional flag is not.
- **The result-file schemas.** Each machine-readable document carries a `schema`
  string:
  - `loadbearer.result/1` — `loadbearer run --output` / `--json`
  - `loadbearer.soak/1` — `loadbearer soak --output` / `--json`
  - `loadbearer.compare/1` — `loadbearer compare --json`
  - `loadbearer.mem/1` — `loadbearer mem --json`

  Removing, renaming, or changing the type of a field is breaking and bumps the
  number after the slash (`/1` → `/2`). **Adding a new optional field does
  not** — consumers must ignore unknown fields. `loadbearer info --json` is the
  `machine` block of `loadbearer.result/1` and is covered by that schema.
- **Exit codes.** `0` on success, non-zero on any failure. Scripts can rely on
  that distinction.

## Not covered — may change in any release

- **Absolute scores and letter grades** for a given machine. They move whenever
  the reference baseline is recalibrated (below) or a kernel is refined. For a
  stable, baseline-independent answer, use `loadbearer compare`, which works
  from the raw metrics.
- **The reference baseline** (`baseline/reference-v1.toml`, embedded). It is
  calibration *data*, not API. Recalibrating it against more or newer hardware
  is a normal minor/patch change even though it shifts everyone's absolute
  numbers. A result file keeps its full raw metrics, so any run can be
  re-scored against any baseline with `loadbearer score`.
- **The model reference table** (`baseline/models/`, embedded) and the
  `model_ref` "vs typical hardware" block it drives. Also calibration data:
  entries and values change as more machines are measured, and the block is
  never folded into a grade.
- **Informational subtests** — rows shown under a component as "informational
  (not graded)" and carried in `raw` with `"scored": false` (CPU thread-scaling
  points, the memory cache-latency ladder, deep-queue disk IOPS, …). Which ones
  exist, their ids and their values may change in any release, and any may later
  be promoted to a scored subtest when the baseline is recalibrated. The
  `telemetry` block (sampled CPU clocks / package power) is likewise indicative,
  not API.
- **Human-readable output** — the `--plain` text layout, the TUI, the wording of
  the "why" lines.
- **The diagnostic log** — file location, line format, which events are logged,
  and the log levels.
- **Internal Rust API.** loadbearer is a binary, not a published library crate;
  module layout and function signatures are not stable.

## Minimum supported Rust version

The MSRV is the `rust-version` in [`Cargo.toml`](Cargo.toml). Raising it is a
**minor** release, not a major one, and will be called out in the changelog.

## In short

If you script against `loadbearer` — parse a `schema`-tagged JSON file, check an
exit code, or drive a subcommand — a major bump is the only thing that can break
you. If you compare two machines, use `loadbearer compare`; its verdict does not
depend on the baseline and is stable across versions.
