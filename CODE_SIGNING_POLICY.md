# Verifying a download

## Code signing

loadbearer's release binaries are **not code-signed**. There is no Authenticode
signature on the Windows executable and no equivalent on the Linux binary.
Downloads are verified instead by their **build provenance attestation** and by
`SHA256SUMS` (below).

On Windows this means SmartScreen shows an "unrecognized app" prompt
(**More info → Run anyway**), and Windows 11 **Smart App Control** — on by
default on clean installs — blocks the binary outright with no exception. Run it
on a machine without SAC (Windows Sandbox works), or build from source. See the
[README](README.md#windows) for the full rundown.

## Build provenance

Every release archive (`loadbearer-<version>-<target>.{zip,tar.gz}`) carries a
signed [build provenance
attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
produced by the release workflow (`.github/workflows/release.yml`). It
cryptographically binds the archive to the exact GitHub Actions run, commit and
workflow that built it; the trust root is GitHub's own Sigstore instance, so
verification needs **no certificate**.

Verify a download with the GitHub CLI (`gh` ≥ 2.49):

```
gh attestation verify loadbearer-<version>-<target>.zip --repo issinoho/loadbearer
```

A `PASS` (`sigstore.dev` issuer, `issinoho/loadbearer` source repo) means the
file is byte-for-byte what CI built from this repository. Offline verification is
available with `gh attestation verify --bundle`.

## Checksums

Every release also publishes `SHA256SUMS`, covering each archive **and** the
bare executable inside it:

```powershell
(Get-FileHash .\loadbearer.exe).Hash        # Windows
```
```
sha256sum loadbearer                        # Linux
```

Compare against the matching line in the release's `SHA256SUMS`.

## Locked-down estates (WDAC / AppLocker)

With no publisher signature there is no publisher rule to write. Use a
**file-hash** allow rule for `loadbearer.exe` — the exact SHA-256 is the
`loadbearer.exe (inside …)` line in `SHA256SUMS`, or `Get-FileHash`. Hash rules
work on an unsigned binary; Smart App Control ignores them.

## Privacy

loadbearer collects no data and transfers no information to any networked
system unless specifically requested by the user or the person operating it
(the `network` benchmark is loopback-only; outbound connections happen only on
an explicit `--net-target`, and a listening socket only on `loadbearer
net-server`). See [PRIVACY.md](PRIVACY.md).

## Reporting a suspected violation

Email <iain@issinoho.com> or use GitHub
[private vulnerability reporting](https://github.com/issinoho/loadbearer/security/advisories/new).
