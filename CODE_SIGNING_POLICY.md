# Code signing policy

Free code signing on Windows is provided by [SignPath.io](https://signpath.io),
certificate by the [SignPath Foundation](https://signpath.org).

## What is signed

- **`loadbearer.exe`** inside the `x86_64-pc-windows-msvc` release archive is
  Authenticode-signed. The signature is applied in CI (GitHub Actions,
  `.github/workflows/release.yml`) via SignPath's origin-verified signing
  service — SignPath independently checks that the artifact was built by this
  repository's release workflow from a `v*` tag before signing it.
- The Linux binary is **not** signed (there is no equivalent trust anchor);
  verify it with `SHA256SUMS`.
- Every release publishes `SHA256SUMS` covering each archive and the bare
  executable inside it.

## How to verify a Windows download

```powershell
Get-AuthenticodeSignature .\loadbearer.exe | Format-List
# Status : Valid   SignerCertificate subject : SignPath Foundation
(Get-FileHash .\loadbearer.exe).Hash   # compare against SHA256SUMS on the release
```

## Roles

loadbearer is maintained by a single person (Iain Smith,
<iain@issinoho.com>), who is the sole **author**, **reviewer** and
**approver**. Every signing request is approved manually, per release, in the
SignPath UI — an unattended build cannot produce a signed binary.

## Privacy

loadbearer collects no data and transfers no information to any networked
system unless specifically requested by the user or the person operating it
(the `network` benchmark is loopback-only; outbound connections happen only on
an explicit `--net-target`, and a listening socket only on `loadbearer
net-server`). See [PRIVACY.md](PRIVACY.md).

## Reporting a suspected violation

Email <iain@issinoho.com> or use GitHub
[private vulnerability reporting](https://github.com/issinoho/loadbearer/security/advisories/new).
