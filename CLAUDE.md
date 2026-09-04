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
Windows `.zip` and Linux `.tar.gz`, attaches a build-provenance attestation to
each, GPG-signs `SHA256SUMS` (`SHA256SUMS.asc`, once `GPG_PRIVATE_KEY` /
`GPG_PASSPHRASE` are set — key details in `CODE_SIGNING_POLICY.md`), and
(once `WINGET_TOKEN` is set and `Issinoho.Loadbearer` exists in
`microsoft/winget-pkgs`) opens the winget version-bump PR. Nothing else to do
from here. The first winget submission is manual — see `packaging/winget/`.
The GPG signature is CI-automated, unlike Windows code signing below — no
manual step needed for it on future releases.

**Windows code signing is a separate manual step, not part of this flow.**
CI always ships the Windows `.zip` unsigned (the Certum cloud cert has no
unattended-CI signing mode — see `CODE_SIGNING_POLICY.md`). Verified working
on v1.2.2 (2026-09-04). Iain runs `scripts/publish-loadbearer.ps1 X.Y.Z` on
Windows (SimplySign Desktop + `signtool`) sometime after the tag push, which
re-signs `loadbearer.exe`, re-uploads the archive, and updates `SHA256SUMS`
on the release. That's a preflight wrapper — it knows the thumbprint, finds
`signtool` under the Windows Kits if it isn't on PATH, and refuses early on
a missing release, a still-draft release, a closed SimplySign session, or
`-UpdateWinget` without a `WINGET_TOKEN` (which the inner script otherwise
only notices *after* signing and uploading). It calls
`scripts/sign-windows-release.ps1`, which takes `-Version`/`-Thumbprint`
directly if you need it. Not something to do from here: it needs a live,
logged-in SimplySign session, which this environment doesn't have. The
thumbprint defaulted in both scripts is for the cert valid to 2027-09-04 —
update it there and in `CODE_SIGNING_POLICY.md` after renewal.

Re-signing an already-signed archive is refused (`-Force` overrides): it
would change the archive's hash a second time and invalidate whatever
pinned the first, the winget manifest above all.

Versioning (semver from 1.0.0 — see `VERSIONING.md` for the covered surface):
breaking change to the CLI or a `schema`-tagged JSON format → **major**; new
subcommand / capability → **minor** (`0.6.0` was `score`); UX polish /
refinement → **patch** (`0.6.1`–`0.6.5` were TUI work). Recalibrating the
baseline or raising the MSRV is **minor**, not major — absolute scores and the
baseline are explicitly *not* part of the stability contract.

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
