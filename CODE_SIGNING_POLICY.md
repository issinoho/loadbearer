# Verifying a download

## Code signing

The Windows executable is Authenticode-signed with a **Certum Open Source
Code Signing** certificate (cloud-held, via SimplySign; common name "Open
Source Developer Iain Smith") — but **not by CI**. Certum's cloud certificate
has no unattended-signing API: it only unlocks through an interactive
SimplySign Desktop session, which needs a persistent, already-authenticated
machine rather than a fresh GitHub-hosted runner. So the release workflow
(`.github/workflows/release.yml`) always publishes the Windows `.zip`
**unsigned**, and the maintainer re-signs `loadbearer.exe` locally afterward
from Windows with SimplySign Desktop + `signtool`
([`scripts/publish-loadbearer.ps1`](scripts/publish-loadbearer.ps1), a
preflight wrapper around
[`scripts/sign-windows-release.ps1`](scripts/sign-windows-release.ps1)),
which re-uploads the signed archive and an updated `SHA256SUMS` in place of
the unsigned ones. A Linux equivalent exists
([`scripts/sign-windows-release.sh`](scripts/sign-windows-release.sh), via
SimplySign Desktop for Linux + `osslsigncode`) but is currently blocked —
Certum's Linux PKCS#11 module segfaults reading the certificate against a
current OpenSSL, a bug in their module, not this tooling — so Windows is the
working path for now.

**Practically:** a release may be unsigned for a short window right after it's
tagged, until that manual step runs. Check for yourself with
`Get-AuthenticodeSignature` (below) rather than assuming either way. The Linux
binary has no equivalent signing mechanism; verify it with the build
provenance attestation and `SHA256SUMS` instead.

On Windows, an unsigned binary means SmartScreen shows an "unrecognized app"
prompt (**More info → Run anyway**), and Windows 11 **Smart App Control** — on
by default on clean installs — blocks it outright with no exception. Run it on
a machine without SAC (Windows Sandbox works), or build from source. A signed
binary clears Smart App Control immediately; SmartScreen reputation still
builds up over time (this is an OV certificate, not EV). See the
[README](README.md#windows) for the full rundown.

```powershell
Get-AuthenticodeSignature .\loadbearer.exe | Format-List Status, SignerCertificate
# Status: Valid   SignerCertificate: ... CN=Open Source Developer Iain Smith ...
```

## Build provenance

Every release archive (`loadbearer-<version>-<target>.{zip,tar.gz}`), as
first built by CI, carries a signed [build provenance
attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
produced by the release workflow. It cryptographically binds *that* archive to
the exact GitHub Actions run, commit and workflow that built it; the trust
root is GitHub's own Sigstore instance, so verification needs no certificate.

```
gh attestation verify loadbearer-<version>-<target>.zip --repo issinoho/loadbearer
```

A `PASS` (`sigstore.dev` issuer, `issinoho/loadbearer` source repo) means the
file is byte-for-byte what CI built. **This holds for the Linux tarball for
the life of the release** (it's never modified after CI builds it). **It does
not hold for the Windows `.zip` once it has been re-signed** — signing changes
the executable's bytes, so the re-signed archive has a different hash than the
one CI attested, and `gh attestation verify` on it will report no matching
attestation. That's expected, not a tampering signal: for the Windows archive,
the Authenticode signature (above) and the current `SHA256SUMS` are the
things to trust, not the original CI attestation.

## GPG signature

CI also detached-signs `SHA256SUMS` (as it first publishes it) with a GPG key
dedicated to loadbearer releases — not Iain's personal key:

- Fingerprint: `C348 2C91 6797 D77F 3892  6A77 273E 45FB 7B21 B6C2`
  (RSA 4096, expires 2028-09-04)
- Public key: [`loadbearer-release-signing.asc`](loadbearer-release-signing.asc)
  in this repo

```
gpg --import loadbearer-release-signing.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
```

`gpg` will warn the key "is not certified with a trusted signature" — normal
for a key you haven't personally vouched for in GPG's web-of-trust sense;
check the fingerprint above matches rather than relying on that warning.

**Only present for the checksums CI built.** Once the Windows archive is
manually re-signed, `SHA256SUMS` is regenerated to match it and
`SHA256SUMS.asc` is removed rather than left stale — its absence on a release
means the Windows re-sign already ran, not that anything is wrong. While
present, it covers the Linux tarball's line for as long as that file exists;
the build provenance attestation above covers the Linux tarball for the
*entire* life of the release regardless, so treat GPG as an alternative for
anyone who'd rather trust a key than GitHub's Sigstore instance, not the only
option. No-op in CI until the `GPG_PRIVATE_KEY` / `GPG_PASSPHRASE` secrets are
set.

## Checksums

Every release publishes `SHA256SUMS`, covering each archive **and** the bare
executable inside it. Whichever signing script runs keeps the Windows lines
in sync with whatever `.zip` is actually attached (updating them
automatically when it re-signs). Compare the matching line against:

```powershell
(Get-FileHash .\loadbearer.exe).Hash        # Windows
```
```
sha256sum loadbearer                        # Linux
```

## Locked-down estates (WDAC / AppLocker)

Once a release is signed, add a **publisher** rule for the Certum certificate:

- Subject: `CN=Open Source Developer Iain Smith, O=Open Source Developer, L=Carluke, S=South Lanarkshire, C=GB`
- Issuer: `CN=Certum Code Signing 2021 CA, O=Asseco Data Systems S.A., C=PL`

Before that, or as a fallback, use a **file-hash** rule for `loadbearer.exe`
— the exact SHA-256 is the `loadbearer.exe (inside …)` line in `SHA256SUMS`,
or `Get-FileHash`. Hash rules work on an unsigned binary; Smart App Control
ignores both kinds of rule.

## Roles

loadbearer is maintained by a single person (Iain Smith,
<iain@issinoho.com>), who holds the signing certificate and is the only one
who runs the manual re-signing step.

## Privacy

loadbearer collects no data and transfers no information to any networked
system unless specifically requested by the user or the person operating it
(the `network` benchmark is loopback-only; outbound connections happen only on
an explicit `--net-target`, and a listening socket only on `loadbearer
net-server`). See [PRIVACY.md](PRIVACY.md).

## Reporting a suspected violation

Email <iain@issinoho.com> or use GitHub
[private vulnerability reporting](https://github.com/issinoho/loadbearer/security/advisories/new).
