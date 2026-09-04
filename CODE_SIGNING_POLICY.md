# Code signing policy

Free code signing on Windows is provided by [SignPath.io](https://signpath.io),
certificate by the [SignPath Foundation](https://signpath.org).

## What is signed

- **`loadbearer.exe`** inside the `x86_64-pc-windows-msvc` release archive is
  Authenticode-signed **when Windows signing is enabled** (the CI
  `SIGNPATH_API_TOKEN` secret is set). The signature is applied in CI (GitHub
  Actions, `.github/workflows/release.yml`) via SignPath's origin-verified
  signing service — SignPath independently checks that the artifact was built by
  this repository's release workflow from a `v*` tag before signing it. Until a
  certificate is in place the Windows binary ships **unsigned**, and the build
  provenance attestation below is the trust anchor.
- The Linux binary is **not** Authenticode-signed (there is no equivalent trust
  anchor); verify it with the provenance attestation and `SHA256SUMS`.
- Every release publishes `SHA256SUMS` covering each archive and the bare
  executable inside it, **and** a signed build provenance attestation for each
  archive.

## Build provenance

Every release archive (`loadbearer-<version>-<target>.{zip,tar.gz}`) carries a
signed [build provenance
attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
produced by the release workflow. It cryptographically binds the archive to the
exact GitHub Actions run, commit and workflow that built it; the trust root is
GitHub's own Sigstore instance, so verification needs **no certificate** and
works on the signed and unsigned binaries alike.

Verify a download with the GitHub CLI (`gh` ≥ 2.49):

```
gh attestation verify loadbearer-<version>-<target>.zip --repo issinoho/loadbearer
```

A `PASS` (`sigstore.dev` issuer, `issinoho/loadbearer` source repo) means the
file is byte-for-byte what CI built from this repository — offline verification
is available with `gh attestation verify --bundle`.

## How to verify a Windows download

```powershell
# Provenance (works whether or not the binary is Authenticode-signed):
gh attestation verify .\loadbearer-<version>-x86_64-pc-windows-msvc.zip --repo issinoho/loadbearer

# Authenticode signature, when Windows signing is enabled for the release:
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
