<#
.SYNOPSIS
  Re-sign an already-published loadbearer Windows release with the Certum
  Open Source Code Signing certificate, and update the GitHub Release in
  place.

.DESCRIPTION
  CI (.github/workflows/release.yml) always ships the Windows binary
  unsigned -- the Certum SimplySign cloud certificate has no unattended-CI
  API, only a SimplySign Desktop session on an already-authenticated
  machine. So signing is a manual, local, post-publish step: run this
  script on the machine where SimplySign Desktop is installed and logged
  in, some time after `git push --follow-tags` has triggered the release
  workflow and the GitHub Release exists.

  What it does:
    1. Downloads that release's Windows .zip and SHA256SUMS with `gh`.
    2. Extracts it, Authenticode-signs loadbearer.exe with signtool against
       the SimplySign-backed certificate in the Windows certificate store,
       and verifies the result.
    3. Re-zips the archive (same internal layout, only the .exe changed).
    4. Recomputes the two Windows lines in SHA256SUMS (zip + the bare exe
       "inside" line); the Linux tar.gz lines are left untouched.
    5. Re-uploads the .zip and SHA256SUMS to the release with
       `gh release upload --clobber`, replacing the unsigned assets.
    6. Optionally re-submits the winget manifest with the new hash
       (-UpdateWinget) -- see the note below on why this matters.

  Requirements: Windows PowerShell 5.1+ / pwsh, the Windows SDK's
  signtool.exe on PATH, the GitHub CLI (`gh`, authenticated with repo
  write access), and a working SimplySign Desktop session so the
  certificate appears in the Windows certificate store.

.PARAMETER Version
  Release version to sign, e.g. "1.2.2" (no leading "v"). Defaults to the
  version in Cargo.toml.

.PARAMETER Thumbprint
  SHA-1 thumbprint of the signing certificate. Omit to let signtool pick
  automatically (`/a` -- fine when SimplySign's is the only cert loaded);
  pass this if more than one certificate is available in the store.

.PARAMETER Repo
  GitHub repo in "owner/name" form. Defaults to issinoho/loadbearer.

.PARAMETER UpdateWinget
  After a successful re-sign, also re-submit the winget manifest
  (wingetcreate update --submit) with the new archive hash. Needs
  wingetcreate.exe on PATH and a GitHub PAT with public_repo scope (pass
  it via the WINGET_TOKEN environment variable). Only meaningful once the
  package already exists in microsoft/winget-pkgs (see packaging/winget/).

.EXAMPLE
  .\scripts\sign-windows-release.ps1 -Version 1.2.2

.EXAMPLE
  .\scripts\sign-windows-release.ps1 -Version 1.2.2 -Thumbprint AB12CD34... -UpdateWinget
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Thumbprint,
    [string]$Repo = "issinoho/loadbearer",
    [switch]$UpdateWinget
)

$ErrorActionPreference = "Stop"

if (-not $Version) {
    $cargoLine = Select-String -Path (Join-Path $PSScriptRoot "..\Cargo.toml") -Pattern '^version' | Select-Object -First 1
    if (-not $cargoLine) { throw "Could not read version from Cargo.toml; pass -Version explicitly." }
    $Version = ($cargoLine.Line -split '"')[1]
}
$tag = "v$Version"
$target = "x86_64-pc-windows-msvc"
$archiveName = "loadbearer-$Version-$target"
$zipName = "$archiveName.zip"

Write-Host "== loadbearer $tag : re-signing the Windows release =="

$work = Join-Path ([System.IO.Path]::GetTempPath()) "loadbearer-sign-$Version-$([guid]::NewGuid().ToString('N').Substring(0,8))"
New-Item -ItemType Directory -Path $work | Out-Null
Push-Location $work
try {
    Write-Host "-- Downloading $zipName and SHA256SUMS from $Repo release $tag"
    gh release download $tag --repo $Repo --pattern $zipName --pattern "SHA256SUMS" --clobber
    if ($LASTEXITCODE -ne 0) { throw "gh release download failed (exit $LASTEXITCODE)" }

    Write-Host "-- Extracting $zipName"
    Expand-Archive -Path $zipName -DestinationPath "extracted" -Force
    $exePath = Join-Path "extracted" "$archiveName\loadbearer.exe"
    if (-not (Test-Path $exePath)) { throw "$exePath not found inside the archive -- unexpected layout" }

    Write-Host "-- Signing $exePath"
    $signArgs = @(
        "sign", "/fd", "sha256", "/tr", "http://time.certum.pl", "/td", "sha256"
    )
    if ($Thumbprint) { $signArgs += @("/sha1", $Thumbprint) } else { $signArgs += "/a" }
    $signArgs += $exePath
    & signtool @signArgs
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed (exit $LASTEXITCODE) -- is a SimplySign session open?" }

    $sig = Get-AuthenticodeSignature $exePath
    $sig | Format-List Status, StatusMessage, SignerCertificate
    if ($sig.Status -ne "Valid") { throw "Authenticode status after signing: $($sig.Status)" }

    Write-Host "-- Re-packaging $zipName"
    Remove-Item $zipName -Force
    Compress-Archive -Path "extracted\$archiveName" -DestinationPath $zipName -CompressionLevel Optimal

    Write-Host "-- Recomputing checksums"
    $zipHash = (Get-FileHash $zipName -Algorithm SHA256).Hash.ToLower()
    $exeHash = (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower()

    $sums = Get-Content "SHA256SUMS"
    $sums = $sums | ForEach-Object {
        if ($_ -match [regex]::Escape($zipName) -and $_ -notmatch "inside") {
            "$zipHash  $zipName"
        } elseif ($_ -match "loadbearer\.exe\s+\(inside $([regex]::Escape($zipName))\)") {
            "$exeHash  loadbearer.exe  (inside $zipName)"
        } else {
            $_
        }
    }
    Set-Content -Path "SHA256SUMS" -Value $sums
    Get-Content "SHA256SUMS"

    Write-Host "-- Uploading the signed archive + updated checksums to the release"
    gh release upload $tag $zipName "SHA256SUMS" --repo $Repo --clobber
    if ($LASTEXITCODE -ne 0) { throw "gh release upload failed (exit $LASTEXITCODE)" }

    # CI's GPG signature (if any) covers the SHA256SUMS it built, which this
    # just replaced -- a stale .asc sitting on the release would silently fail
    # to verify against the new file instead of just being absent. Drop it if
    # present; harmless if it wasn't there (GPG signing is opt-in in CI).
    $existing = gh release view $tag --repo $Repo --json assets --jq ".assets[].name" 2>$null
    if ($existing -contains "SHA256SUMS.asc") {
        Write-Host "-- Removing the now-stale SHA256SUMS.asc (signed the pre-resign checksums)"
        gh release delete-asset $tag "SHA256SUMS.asc" --repo $Repo --yes
    }

    Write-Host ""
    Write-Host "Done. $zipName on the $tag release is now signed; SHA256SUMS matches it."
    Write-Host "Note: the build-provenance attestation on this release still points at the" -ForegroundColor Yellow
    Write-Host "original *unsigned* archive's hash (that's what CI attested) -- it will not" -ForegroundColor Yellow
    Write-Host "verify against this signed replacement. The Authenticode signature above is" -ForegroundColor Yellow
    Write-Host "the trust anchor for this file now; SHA256SUMS and gh attestation verify" -ForegroundColor Yellow
    Write-Host "still work for the untouched Linux tarball." -ForegroundColor Yellow

    if ($UpdateWinget) {
        if (-not $env:WINGET_TOKEN) { throw "-UpdateWinget needs a PAT in the WINGET_TOKEN environment variable." }
        Write-Host ""
        Write-Host "-- Re-submitting the winget manifest with the signed archive's hash"
        if (-not (Get-Command wingetcreate.exe -ErrorAction SilentlyContinue)) {
            Invoke-WebRequest https://aka.ms/wingetcreate/latest -OutFile wingetcreate.exe
        }
        $url = "https://github.com/$Repo/releases/download/$tag/$zipName"
        & .\wingetcreate.exe update Issinoho.Loadbearer --version $Version --urls $url --submit --token $env:WINGET_TOKEN
        if ($LASTEXITCODE -ne 0) { throw "wingetcreate update failed (exit $LASTEXITCODE)" }
    }
}
finally {
    Pop-Location
}
