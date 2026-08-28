# loadbearer — notes for Claude

CLI system-assessment / benchmark tool in Rust. Benchmarks CPU, memory, disk and
network, scores each metric against an embedded reference baseline
(`baseline/reference-v1.toml`), and grades every component and the whole machine
on an S–F scale. Primary target is Windows; Linux is first-class and is where
development happens.

`edition = "2024"`, `rust-version = "1.88"`, `build = "build.rs"` (stamps
`LOADBEARER_VERSION` from git + Cargo.toml for `--version`).

## Checks (must pass — this is exactly what CI runs)

```
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs on ubuntu-latest **and** windows-latest. `--locked` matters: after a
version bump, run `cargo build` once to refresh `Cargo.lock` or `--locked` fails.
Clippy on CI's toolchain is sometimes stricter than a local one — if in doubt
`rustup update stable`.

## Cutting a release

1. Bump `version` in `Cargo.toml`.
2. Add a `## X.Y.Z - <date>` section at the top of `CHANGELOG.md` (the release
   workflow extracts this section verbatim for the GitHub Release body).
3. Update `README.md` / `docs/` / the wiki if the change is user-visible.
4. Commit, then `git tag -a vX.Y.Z -m "..."`.
5. Push with tags: `git push <remote> main --follow-tags`.

`.github/workflows/release.yml` fires on the `v*` tag and builds+attaches the
Windows `.zip` and Linux `.tar.gz`. Nothing else to do.

Versioning this repo uses: new subcommand / capability → minor (`0.6.0` was
`score`); UX polish / refinement → patch (`0.6.1`–`0.6.5` were TUI work).

## Pushing

SSH auth is unreliable here; push over tokenised HTTPS:

```
git push "https://x-access-token:$(gh auth token)@github.com/issinoho/loadbearer.git" main --follow-tags
```

Transient `Could not resolve host: github.com` happens — just retry.

## Layout

- `src/engine/` — the `Benchmark` trait, the scheduler (warmup + timed
  iterations, `throughput`, `parallel_sum`), core-affinity pinning for
  single-threaded subtests.
- `src/benches/` — one module per component: `cpu`, `memory`, `disk` (+ `disk/`
  submodules for aligned/unbuffered I/O), `network`.
- `src/soak.rs` — the sustained-load / thermal test (`loadbearer soak`,
  `run --soak`). Not scored.
- `src/scoring/` — `mod.rs` (ratio → curve → geomean → grade, `ResultFile`),
  `baseline.rs`, `profiles.rs`.
- `src/compare.rs` — `loadbearer compare` (verdict from raw metrics, baseline-
  independent).
- `src/score.rs` — `loadbearer score` (re-grade a saved result against another
  baseline/profile/curve, no re-run).
- `src/tui/` — `mod.rs` (event loop + worker thread for `run`), `app.rs` (state),
  `view.rs` (run + results + soak screens), `compare.rs` (the compare view).
- `src/output/` — plain-text and JSON rendering for non-TUI paths.
- `src/cli.rs` — clap definitions. `src/run.rs` — resolves settings (CLI >
  config > default) and dispatches.

Network and the `--soak` / `--net-target` results are **measured and shown but
never folded into a grade** — they depend on the OS / cooling, not the silicon.

## Conventions

- TUI screens size their columns from the terminal width (bounded), don't
  truncate to fixed widths. Prose (warnings, verdicts) word-wraps; tables clip.
  Key parity across `run` results and `compare`: `↑↓/jk`, `PgUp/PgDn/space`,
  `Home/End/g/G`, `q/Esc/Enter`.
- TUI render tests use `ratatui::backend::TestBackend` — see the `tests` modules
  in `src/tui/view.rs` and `src/tui/compare.rs`.
- Scoring changes: `score_run` errors hard if the baseline lacks a subtest ("the
  baseline can't quietly fall out of sync"). `score` is the lenient exception
  (skips-with-note).

## Docs live in three places — keep them in sync

- `README.md` — install, usage, scoring model at a working level.
- `docs/` — the website (GitHub Pages, custom domain `loadbearer.issinoho.com`).
  `docs/script.js` fills the release tag/links from the GitHub API, so no version
  is baked into the page chrome; only the terminal **mockups** carry a version
  string.
- The **wiki is a separate git repo** (`loadbearer.wiki.git`, branch `master`).
  Clone it separately; it's also editable via the GitHub web UI, so `git pull
  --rebase` before pushing.
