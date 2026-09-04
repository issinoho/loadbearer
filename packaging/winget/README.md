# winget (Windows Package Manager)

Once loadbearer is in [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs):

```
winget install Issinoho.Loadbearer
winget upgrade  Issinoho.Loadbearer
```

winget extracts the release `.zip` and puts `loadbearer` on `PATH` as a
portable command.

## First submission (one-off, manual)

The three YAML files here are the seed manifest for **PackageVersion 1.2.1**.
Submit them as a pull request to `microsoft/winget-pkgs` under
`manifests/i/Issinoho/Loadbearer/1.2.1/`. Easiest path:

```
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/issinoho/loadbearer/releases/download/v1.2.1/loadbearer-1.2.1-x86_64-pc-windows-msvc.zip
```

and cross-check what it generates against these files (in particular
`NestedInstallerFiles.RelativeFilePath`, which must match the folder inside the
zip — `loadbearer-<version>-x86_64-pc-windows-msvc\loadbearer.exe`). Or run
`wingetcreate submit` on this directory directly. A new package is reviewed by a
human moderator; expect a few days to ~two weeks.

`PackageIdentifier` (`Issinoho.Loadbearer`) is permanent once merged.

## Subsequent releases (automated)

`.github/workflows/release.yml` has a `winget` job that runs `wingetcreate
update … --submit` after each `v*` release, opening the version-bump PR
automatically. It is a no-op until the `WINGET_TOKEN` secret is set, and it can
only *update* a package that already exists, so the first PR above must merge
first.

Keep the `PackageVersion` in these files roughly current for reference, but the
automated PRs are generated from the published release, not from this directory.

## The `WINGET_TOKEN` secret

`wingetcreate --submit` forks `microsoft/winget-pkgs` to the token owner's
account, pushes a branch, and opens a PR. Add the token under **repo Settings →
Secrets and variables → Actions → `WINGET_TOKEN`**.

**Use a classic PAT.** One scope:

| Scope | Why |
| --- | --- |
| `public_repo` | Create/refresh the `winget-pkgs` fork, push the manifest branch, open the PR. Nothing else is touched. |

Do **not** grant `repo` (full), `workflow`, `admin:*`, `delete_repo`, or any
`write:packages` — the manifest PR is data-only and never edits workflows.

A **fine-grained PAT** also works but is fiddlier (fork creation): owner = your
account, "All repositories", with *Contents: read/write* and *Pull requests:
read/write*. The classic `public_repo` token is what Microsoft's docs and
`wingetcreate` expect — prefer it.

Housekeeping:

- Set an **expiry** (≤ 1 year) and diary a renewal. An expired token makes the
  `winget` job go red but does **not** affect the release itself.
- The token owner's GitHub account is the one that appears as PR author on
  `winget-pkgs`.
- If `wingetcreate` complains the fork is stale, hit **Sync fork** on
  `github.com/<you>/winget-pkgs` and re-run.

## First-PR checklist

Do this once, from a Windows machine, to get `Issinoho.Loadbearer` into
`winget-pkgs`. After it merges, the CI job handles every later version.

**Prep**

- [ ] `PackageIdentifier` is `Issinoho.Loadbearer` — PascalCase `Publisher.Package`,
      **permanent** once merged, renames need a moderator.
- [ ] The target release is public and **not a draft**; the asset
      `loadbearer-<v>-x86_64-pc-windows-msvc.zip` is attached.
- [ ] Have the zip's SHA-256 (from the release `SHA256SUMS`) — `wingetcreate`
      will recompute and should match.
- [ ] `winget install Microsoft.WingetCreate`.

**Build / validate the manifest**

- [ ] `wingetcreate new https://github.com/issinoho/loadbearer/releases/download/v<v>/loadbearer-<v>-x86_64-pc-windows-msvc.zip`
- [ ] Answer the prompts: Architecture `x64`; InstallerType `zip`;
      NestedInstallerType `portable`; nested file
      `loadbearer-<v>-x86_64-pc-windows-msvc\loadbearer.exe` (backslash, exact
      folder name — it embeds the version); PortableCommandAlias `loadbearer`.
- [ ] Diff the generated YAML against the files in this directory — especially
      `NestedInstallerFiles.RelativeFilePath` and `InstallerSha256` (UPPERCASE).
- [ ] Fill the locale fields from `Issinoho.Loadbearer.locale.en-US.yaml`
      (Publisher, Description, `License: MIT`, LicenseUrl, Tags, Moniker,
      PublisherSupportUrl, ReleaseNotesUrl).
- [ ] `ManifestVersion: 1.6.0` in all three files; installer URL is the
      **versioned** asset (a `…/releases/latest/…` URL is rejected).
- [ ] `winget validate --manifest <dir>` passes.
- [ ] Optional dry run: `winget install --manifest <dir>` → `loadbearer --version`
      → `winget uninstall Issinoho.Loadbearer`.

**Submit**

- [ ] `wingetcreate submit --token <PAT> <dir>` (or add `--submit` to the
      `new` command). It forks, pushes
      `manifests/i/Issinoho/Loadbearer/<v>/`, and opens the PR
      (`New package: Issinoho.Loadbearer version <v>`).
- [ ] Watch the PR: the `azure-pipelines` / wingetbot validation runs manifest
      checks, a sandbox install + uninstall, and a malware scan. Clear any
      `Needs-Author-Feedback`.
- [ ] A human moderator approves and merges — a few days to ~two weeks for a
      brand-new package.

**After merge**

- [ ] `winget install Issinoho.Loadbearer` works from a clean machine (index can
      take ~30 min to propagate).
- [ ] Add the `WINGET_TOKEN` secret (above) so future releases auto-submit.
- [ ] On the next `v*` tag, confirm the `winget` CI job opened the update PR and
      it merged.

**Known snags**

- Portable-zip install just extracts — no execution — so an unsigned
  `loadbearer.exe` passes validation. That changes if the manifest ever moves to
  an installer type that runs code on install.
- Every release changes the nested folder name (it carries the version);
  `wingetcreate update` handles it, a hand-edit must not forget it.
