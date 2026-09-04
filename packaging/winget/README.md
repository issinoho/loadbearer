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
automatically. It needs a repo secret **`WINGET_TOKEN`** — a GitHub personal
access token (classic, `public_repo` scope; or fine-grained with the ability to
fork and open PRs on public repos). The job is a no-op until that secret is set,
and it can only *update* a package that already exists, so the first PR above
must merge first.

Keep the `PackageVersion` in these files roughly current for reference, but the
automated PRs are generated from the published release, not from this directory.
