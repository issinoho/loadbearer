<#
.SYNOPSIS
  Signs a published loadbearer Windows release. The one command to run
  after the release workflow has finished.

.DESCRIPTION
  A front door to scripts/sign-windows-release.ps1, which does the real
  work. This script supplies the certificate thumbprint and checks what
  can be checked before a signing session is spent -- opening one costs a
  one-time code from the phone, so it is worth failing early rather than
  halfway through.

  Note that loadbearer's flow is the opposite way round from tvdinner's.
  CI here publishes the release immediately with an *unsigned* archive,
  and this replaces it in place afterwards; tvdinner leaves a draft and
  is published by its signing step. So the checks below expect a
  published release, not a draft.

  There is deliberately no "is the checkout at this version" check
  either: nothing is compiled locally, the archive is downloaded from
  the release and only the .exe inside it changes. Signing 1.2.2 from a
  checkout of main is fine.

.PARAMETER Version
  Version to sign, e.g. 1.2.2. A leading "v" is accepted. Defaults to
  the version in Cargo.toml.

.PARAMETER Thumbprint
  Signing certificate, defaulting to the one recorded in
  CODE_SIGNING_POLICY.md. Update both after a renewal.

.PARAMETER Repo
  GitHub repo in owner/name form.

.PARAMETER UpdateWinget
  Also re-submit the winget manifest with the signed archive's hash.
  Needs a PAT in $env:WINGET_TOKEN -- which this checks up front, since
  the underlying script otherwise discovers it missing only after the
  signing and upload are done.

.PARAMETER Force
  Sign even though the archive already carries a valid signature. See
  the guard in sign-windows-release.ps1 for why that is normally
  refused.

.EXAMPLE
  .\scripts\publish-loadbearer.ps1 1.2.3

.EXAMPLE
  .\scripts\publish-loadbearer.ps1 1.2.3 -UpdateWinget
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)][string]$Version,
    [string]$Thumbprint = '6B58FE5ED40A67A23A27BEB25C4337ADEA26B9F9',
    [string]$Repo = 'issinoho/loadbearer',
    [switch]$UpdateWinget,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

# Say what's wrong on one line rather than inside a stack frame; every
# throw below is written to be read on its own.
trap {
    Write-Host ""
    Write-Host "  $($_.Exception.Message)" -ForegroundColor Red
    Write-Host ""
    exit 1
}

# The checks below read $LASTEXITCODE, so a native command exiting
# nonzero must not throw before they run.
$PSNativeCommandUseErrorActionPreference = $false

$repoRoot = Split-Path -Parent $PSScriptRoot
$signScript = Join-Path $PSScriptRoot 'sign-windows-release.ps1'
if (-not (Test-Path $signScript)) {
    throw "sign-windows-release.ps1 isn't next to this script ($PSScriptRoot). Is the checkout complete?"
}

if (-not $Version) {
    $cargoLine = Select-String -Path (Join-Path $repoRoot 'Cargo.toml') -Pattern '^version' | Select-Object -First 1
    if (-not $cargoLine) { throw "Couldn't read the version from Cargo.toml; pass one explicitly." }
    $Version = ($cargoLine.Line -split '"')[1]
}
$Version = $Version.Trim().TrimStart('v', 'V')
if ($Version -notmatch '^\d+\.\d+\.\d+') {
    throw "'$Version' doesn't look like a version number -- expected something like 1.2.3."
}
$tag = "v$Version"
$zipName = "loadbearer-$Version-x86_64-pc-windows-msvc.zip"

Write-Host ""
Write-Host "Signing loadbearer $tag" -ForegroundColor Cyan
Write-Host "Repository: $Repo"
Write-Host ""
Write-Host "Preflight" -ForegroundColor Cyan

# 1. gh, since every release operation goes through it.
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI (gh) isn't on PATH. Install it from https://cli.github.com/."
}
& gh auth status 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw "gh isn't authenticated. Run: gh auth login" }
Write-Host "  gh authenticated"

# 2. A bare environment read, so it goes early: the underlying script
#    asks for this only after signing and uploading, which is a miserable
#    place to discover a missing token.
if ($UpdateWinget -and -not $env:WINGET_TOKEN) {
    throw "-UpdateWinget needs a GitHub PAT with public_repo scope in `$env:WINGET_TOKEN. Set it and run again, or drop -UpdateWinget and update the manifest separately."
}
if ($UpdateWinget) { Write-Host "  WINGET_TOKEN set" }

# 3. sign-windows-release.ps1 invokes signtool bare, so it has to be on
#    PATH by the time we hand over. Find it and put it there rather than
#    making the caller edit their environment for us.
if (Get-Command signtool -ErrorAction SilentlyContinue) {
    Write-Host "  signtool on PATH"
}
else {
    $kitRoots = @(
        'C:\Program Files (x86)\Windows Kits\10\bin',
        'C:\Program Files\Windows Kits\10\bin'
    ) | Where-Object { Test-Path $_ }

    $found = $kitRoots | ForEach-Object {
        Get-ChildItem -Path $_ -Filter 'signtool.exe' -Recurse -ErrorAction SilentlyContinue
    } | Where-Object { $_.FullName -notmatch '\\arm' } |
        Sort-Object @{ Expression = { $_.FullName -match '\\x64\\' }; Descending = $true },
                    @{ Expression = { $_.LastWriteTime }; Descending = $true } |
        Select-Object -First 1

    if (-not $found) {
        throw "signtool.exe isn't on PATH and wasn't found under the Windows Kits directories. Install the Windows SDK's signing tools: winget install --id Microsoft.WindowsSDK.10.0.18362 --exact --override `"/features OptionId.SigningTools /quiet`""
    }
    $env:Path = "$($found.Directory.FullName);$env:Path"
    Write-Host "  signtool found: $($found.FullName)"
}

# 4. The release must exist and be published -- CI publishes immediately
#    here, so a draft means the workflow hasn't finished.
$json = & gh release view $tag --repo $Repo --json isDraft,assets 2>$null
if ($LASTEXITCODE -ne 0) {
    throw "There's no release $tag in $Repo. Push the tag and let the release workflow finish first."
}
$release = $json | ConvertFrom-Json
if ($release.isDraft) {
    throw "$tag is still a draft, so the release workflow probably hasn't finished. Wait for it, then run this again."
}

$missing = @($zipName, 'SHA256SUMS') | Where-Object { $name = $_; -not ($release.assets | Where-Object { $_.name -eq $name }) }
if ($missing) {
    throw "$tag is missing $($missing -join ' and '). Both are needed: the archive is re-signed, and SHA256SUMS is rewritten to match it."
}
Write-Host "  $tag is published, carrying $zipName and SHA256SUMS"

# 5. The certificate is only in the store while a SimplySign session is
#    open, so this doubles as the "are you logged in?" check.
$cert = Get-ChildItem Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
    Where-Object { $_.Thumbprint -eq $Thumbprint }
if (-not $cert) {
    throw "Certificate $Thumbprint isn't in your certificate store. Open SimplySign Desktop and log in, then try again. (List what is there with: Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert)"
}
$daysLeft = [int]($cert.NotAfter - (Get-Date)).TotalDays
if ($daysLeft -lt 0) {
    throw "Certificate $Thumbprint expired on $($cert.NotAfter.ToString('yyyy-MM-dd'))."
}
if ($daysLeft -lt 30) {
    Write-Warning "Certificate expires in $daysLeft days, on $($cert.NotAfter.ToString('yyyy-MM-dd')). Renew it, then update the default in this script and in CODE_SIGNING_POLICY.md."
}
Write-Host "  certificate present: $($cert.Subject.Split(',')[0]) (expires $($cert.NotAfter.ToString('yyyy-MM-dd')))"

Write-Host ""
Write-Host "Signing" -ForegroundColor Cyan

$signArgs = @('-Version', $Version, '-Thumbprint', $Thumbprint, '-Repo', $Repo)
if ($UpdateWinget) { $signArgs += '-UpdateWinget' }
if ($Force) { $signArgs += '-Force' }
& $signScript @signArgs
if ($LASTEXITCODE -ne 0) { throw "sign-windows-release.ps1 failed." }

Write-Host ""
Write-Host "Done. loadbearer $Version is signed on the $tag release." -ForegroundColor Green
