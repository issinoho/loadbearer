## What changed and why

<!-- A short description. Link a related issue if there is one (e.g. "Fixes #123"). -->

## How was this tested?

<!--
- `cargo test` output (or just "passes locally" if nothing unusual).
- If this touches the disk or network benchmark, the TUI, or platform
  code: which OS you ran a real `loadbearer run` on, and what you saw
  (paste the `--plain` output). Those paths aren't unit-tested.
-->

## Checklist

- [ ] `cargo test` passes locally
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `cargo fmt --all --check` is clean
- [ ] A new subtest has a matching entry in `baseline/reference-v1.toml`
- [ ] Docs updated if this changes user-facing behavior (README, and/or
      the relevant [wiki](https://github.com/issinoho/loadbearer/wiki) page)

<!--
No need to touch the version in Cargo.toml, CHANGELOG.md, or cut a
release — that's a maintainer step done at release time.
-->
