# winget (Windows Package Manager)

loadbearer is in [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs)
as `Issinoho.Loadbearer`:

```
winget install Issinoho.Loadbearer
winget upgrade  Issinoho.Loadbearer
```

winget extracts the release `.zip` and puts `loadbearer` on `PATH` as a
portable command.

## First submission (one-off, manual)

**Done.** Submitted as
[microsoft/winget-pkgs#429367](https://github.com/microsoft/winget-pkgs/pull/429367)
for 1.5.2, moderator-approved and merged 2026-10-01, published to the index
2026-10-02. Nothing below needs doing again; it is kept as a record and for the
checklist's sake.

The three YAML files here are the seed manifest for **PackageVersion 1.5.2**.
Submit them as a pull request to `microsoft/winget-pkgs` under
`manifests/i/Issinoho/Loadbearer/1.5.2/`. Easiest path:

```
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/issinoho/loadbearer/releases/download/v1.5.2/loadbearer-1.5.2-x86_64-pc-windows-msvc.zip
```

and cross-check what it generates against these files (in particular
`NestedInstallerFiles.RelativeFilePath`, which must match the folder inside the
zip — `loadbearer-<version>-x86_64-pc-windows-msvc\loadbearer.exe`). Or run
`wingetcreate submit` on this directory directly. A new package is reviewed by a
human moderator; expect a few days to ~two weeks.

`PackageIdentifier` (`Issinoho.Loadbearer`) is permanent once merged.

## Subsequent releases

`.github/workflows/winget.yml` runs `wingetcreate update … --submit`, opening
the version-bump PR. It is **not** triggered by the tag push: the release
workflow ships the Windows `.zip` unsigned and `scripts/publish-loadbearer.ps1`
replaces it with a signed one afterwards, which changes its hash. A PR opened
on the tag would pin the unsigned archive and be wrong as soon as it was signed.
So the order is:

```
.\scripts\publish-loadbearer.ps1 X.Y.Z -UpdateWinget
```

which signs, uploads, then dispatches `winget.yml` for `X.Y.Z` with
`gh workflow run`. The workflow downloads the release archive and refuses to
submit unless `loadbearer.exe` in it is Authenticode-signed. To submit without
signing (or to retry), run it from the Actions tab or with
`gh workflow run winget.yml -f version=X.Y.Z [-f allow_unsigned=true]`.

Keep the `PackageVersion` in these files roughly current for reference, but the
automated PRs are generated from the published release, not from this directory.

## The `WINGET_TOKEN` secret

`wingetcreate --submit` forks `microsoft/winget-pkgs` to the token owner's
account, pushes a branch, and opens a PR. The token lives only in **repo
Settings → Secrets and variables → Actions → `WINGET_TOKEN`**. It is used by
the workflow, so it doesn't need to be on the signing machine.

**Status:** set 2026-10-02, a classic PAT with `public_repo` scope. Renew it before
it expires (at most a year, so by 2027-10-02), and set the replacement with the
token as the **value**. In a terminal, not a chat:
`gh secret set WINGET_TOKEN -R issinoho/loadbearer`, then paste when prompted.

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
  `winget` workflow go red but does **not** affect the release or the signing.
- The token owner's GitHub account is the one that appears as PR author on
  `winget-pkgs`.
- If `wingetcreate` complains the fork is stale, hit **Sync fork** on
  `github.com/<you>/winget-pkgs` and re-run.

## First-PR checklist

Done once, for 1.5.2, to get `Issinoho.Loadbearer` into `winget-pkgs`. Kept
for reference; later versions go through `winget.yml` (above).

**Prep**

- [x] `PackageIdentifier` is `Issinoho.Loadbearer` — PascalCase `Publisher.Package`,
      **permanent** once merged, renames need a moderator.
- [x] The target release is public and **not a draft**; the asset
      `loadbearer-<v>-x86_64-pc-windows-msvc.zip` is attached.
- [x] Have the zip's SHA-256 (from the release `SHA256SUMS`) — `wingetcreate`
      will recompute and should match.
- [x] `winget install Microsoft.WingetCreate`.

**Build / validate the manifest**

- [x] `wingetcreate new https://github.com/issinoho/loadbearer/releases/download/v<v>/loadbearer-<v>-x86_64-pc-windows-msvc.zip`
- [x] Answer the prompts: Architecture `x64`; InstallerType `zip`;
      NestedInstallerType `portable`; nested file
      `loadbearer-<v>-x86_64-pc-windows-msvc\loadbearer.exe` (backslash, exact
      folder name — it embeds the version); PortableCommandAlias `loadbearer`.
- [x] Diff the generated YAML against the files in this directory — especially
      `NestedInstallerFiles.RelativeFilePath` and `InstallerSha256` (UPPERCASE).
- [x] Fill the locale fields from `Issinoho.Loadbearer.locale.en-US.yaml`
      (Publisher, Description, `License: MIT`, LicenseUrl, Tags, Moniker,
      PublisherSupportUrl, ReleaseNotesUrl).
- [x] `ManifestVersion: 1.6.0` in all three files; installer URL is the
      **versioned** asset (a `…/releases/latest/…` URL is rejected).
- [x] `winget validate --manifest <dir>` passes.
- [ ] Optional dry run: `winget install --manifest <dir>` → `loadbearer --version`
      → `winget uninstall Issinoho.Loadbearer`.

**Submit**

- [x] `wingetcreate submit --token <PAT> <dir>` (or add `--submit` to the
      `new` command). It forks, pushes
      `manifests/i/Issinoho/Loadbearer/<v>/`, and opens the PR
      (`New package: Issinoho.Loadbearer version <v>`).
- [x] Watch the PR: the `azure-pipelines` / wingetbot validation runs manifest
      checks, a sandbox install + uninstall, and a malware scan. Clear any
      `Needs-Author-Feedback`.
- [x] A human moderator approves and merges — a few days to ~two weeks for a
      brand-new package.

**After merge**

- [ ] `winget install Issinoho.Loadbearer` works from a clean machine (index can
      take ~30 min to propagate).
- [x] Add the `WINGET_TOKEN` secret (above) so future releases can be submitted.
- [ ] On the next release, confirm `publish-loadbearer.ps1 -UpdateWinget` got the
      `winget` workflow to open the update PR, and that it merged.

**Known snags**

- Portable-zip install just extracts — no execution — so an unsigned
  `loadbearer.exe` passes validation. That changes if the manifest ever moves to
  an installer type that runs code on install.
- Every release changes the nested folder name (it carries the version);
  `wingetcreate update` handles it, a hand-edit must not forget it.
