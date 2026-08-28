# Contributing to loadbearer

Thanks for considering a contribution. This is a solo-maintained
project, so response times are best-effort — but bug reports, small
fixes, and well-scoped features are genuinely welcome.

This file covers contributing to the *code*. For how to *use*
loadbearer, see the [README](README.md); for the methodology in depth —
what each benchmark kernel does, the scoring maths, `compare` internals
— see the [wiki](https://github.com/issinoho/loadbearer/wiki).

By participating you're expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Getting started

```
git clone https://github.com/issinoho/loadbearer.git
cd loadbearer
cargo test
cargo run -- info
cargo run -- run --only cpu --duration short
```

A recent stable Rust toolchain (1.88+) is all you need.

## The checks

Every push and pull request runs, on **Linux and Windows**:

```
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

All three must pass. `-D warnings` means clippy lints are errors — run
`cargo clippy --all-targets` locally before opening a PR. Newer stable
clippy sometimes adds lints; if CI's clippy is stricter than yours,
`rustup update stable` and re-run.

Match the style of the surrounding code — comment density, naming,
idiom. Comments should explain *why*, especially for a platform quirk or
a deliberate trade-off, not restate what the code already says.

## Building, testing, releasing

### Build

```
cargo build                 # debug
cargo build --release        # what the release workflow ships
RUSTFLAGS="-C target-cpu=native" cargo build --release   # widest vectors for local runs
```

`build.rs` stamps the version string (`--version` output) from the Cargo
version plus the git commit, date, target and profile. It honours
`SOURCE_DATE_EPOCH` and falls back to `unknown` outside a git checkout —
nothing to configure.

### Test

The three commands under [The checks](#the-checks) are the whole suite;
run them before every PR. `--locked` is deliberate — if you change
dependencies, commit the updated `Cargo.lock`. A fast inner loop while
working on one area:

```
cargo test <module>          # e.g. cargo test scoring
cargo test --quiet
```

See [Testing conventions](#testing-conventions) for what to add. TUI
rendering is covered with `ratatui::backend::TestBackend` (render into a
fixed-size buffer, assert on the text) — see the `tests` modules in
`src/tui/view.rs` and `src/tui/compare.rs`.

### Release (maintainer only)

Contributors should **not** bump the version or edit `CHANGELOG.md`;
that happens here at release time:

1. Bump `version` in `Cargo.toml`, then `cargo build` once so
   `Cargo.lock` picks up the new version (or `--locked` checks fail).
2. Add a `## X.Y.Z - <date>` section at the top of `CHANGELOG.md`. The
   release workflow extracts this section verbatim as the GitHub Release
   body, so write it for a reader.
3. Update `README.md`, `docs/` (the website), and the
   [wiki](https://github.com/issinoho/loadbearer/wiki) for anything
   user-visible. The wiki is a separate git repo — `git pull --rebase`
   before pushing it.
4. Run the checks, commit, then tag: `git tag -a vX.Y.Z -m "…"`.
5. `git push origin main --follow-tags`.

The `v*` tag triggers `.github/workflows/release.yml`, which builds the
Windows `.zip` and Linux `.tar.gz`, extracts the changelog section, and
publishes the GitHub Release. A `workflow_dispatch` run builds the same
artifacts without publishing — for verifying an experimental build on
real hardware.

Version bumps: a new subcommand or capability is a **minor** bump; a
fix or UX refinement is a **patch**.

## Project layout

| Path | What |
| --- | --- |
| `src/engine/` | the `Benchmark` trait, the warmup + timed-iteration scheduler, `Progress`, core-affinity pinning, summary stats |
| `src/benches/` | one module per component: `cpu`, `memory`, `disk` (+ `disk/aligned`, `disk/platform`), `network`; `benches::all()` is the registry |
| `src/scoring/` | `baseline` (embedded `reference-v1.toml`), the curve/geomean/grade pipeline, `profiles`, and the versioned `ResultFile` |
| `src/compare.rs` | head-to-head of two or more result files |
| `src/tui/` | the ratatui run + results screens (`app` = state, `view` = drawing) |
| `src/output/` | plain-text and comparison rendering |
| `src/run.rs`, `src/cli.rs`, `src/config.rs` | `loadbearer run`, argument parsing, the `--config` file |
| `baseline/reference-v1.toml` | the reference values, embedded via `include_str!` |

## Adding a benchmark subtest or component

1. Implement (or extend) a `Benchmark` in `src/benches/`. Register a new
   component in `benches::all()`.
2. **Add a matching entry to `baseline/reference-v1.toml`** for every new
   subtest — a missing baseline value is a hard error at scoring time, by
   design, so the baseline can't drift out of sync with the code.
3. If the subtest is single-threaded, return `true` from
   `Benchmark::single_threaded()` for its id so the engine pins it to one
   core.
4. Decide whether it belongs in the overall grade. Network is scored but
   excluded (`scoring::UNGRADED_COMPONENTS`) because its result is more
   about the OS than the hardware — apply the same judgement.

## Testing conventions

- Unit tests live in each module's `#[cfg(test)] mod tests`.
- A benchmark kernel gets a "produces a finite positive value with a
  tiny (~20 ms) budget" smoke test — see `benches/cpu.rs`.
- Scoring is tested deterministically (curve exponent, geometric means,
  grade bands, baseline round-trip, JSON round-trip).
- The TUI's state machine (`tui/app.rs`) is unit-tested; screen rendering
  is checked with `ratatui`'s `TestBackend` (column widths, wrapping,
  no truncation of long labels on a wide terminal).
- **Anything touching real disk I/O, sockets, or the terminal is
  validated by running `loadbearer run` on the target OS**, not by a
  unit test. If your change touches the disk or network benchmark, the
  TUI, or platform code, say in the PR which OS you ran it on and what
  you saw (paste `loadbearer run --plain` output).

## Submitting a change

1. Fork and branch off `main`.
2. Make the change, with tests where the conventions above call for them.
3. `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo test` all pass locally.
4. Open a PR describing what changed and why. Reference any related
   issue (`Fixes #123`).

Leave `Cargo.toml`'s version and `CHANGELOG.md` alone — see
[Release](#release-maintainer-only).

## Reporting bugs / requesting features

Open a [GitHub issue](https://github.com/issinoho/loadbearer/issues/new/choose).
For a bug, include the exact command, the output of `loadbearer
--version` and `loadbearer info`, and — if a subtest looks wrong — the
`loadbearer run --plain` output with its confidence flags.
