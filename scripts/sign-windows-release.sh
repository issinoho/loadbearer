#!/usr/bin/env bash
# Re-sign an already-published loadbearer Windows release with the Certum
# Open Source Code Signing certificate (SimplySign, cloud-held), from Linux.
#
# CI (.github/workflows/release.yml) always ships the Windows binary
# unsigned -- the Certum cert has no unattended-CI API, only an interactive
# SimplySign Desktop session. So signing is a manual, local, post-publish
# step: run this once SimplySign Desktop (Linux build) is installed and
# logged in, some time after `git push --follow-tags` has triggered the
# release workflow and the GitHub Release exists.
#
# What it does:
#   1. Downloads that release's Windows .zip and SHA256SUMS with `gh`.
#   2. Extracts it, Authenticode-signs loadbearer.exe with osslsigncode
#      against the SimplySign PKCS#11 token, and verifies the result.
#   3. Re-zips the archive (same internal layout, only the .exe changed).
#   4. Recomputes the two Windows lines in SHA256SUMS (zip + the bare exe
#      "inside" line); the Linux tar.gz lines are left untouched.
#   5. Re-uploads the .zip and SHA256SUMS to the release with
#      `gh release upload --clobber`, replacing the unsigned assets.
#   6. With --update-winget, also re-submits the winget manifest with the
#      new hash -- see the note in the usage text below on why that matters.
#
# Requirements: osslsigncode >= 2.10 (older versions use a raw key-ID
# syntax instead of PKCS#11 URIs -- this script assumes the new one), the
# GitHub CLI (`gh`, authenticated with repo write access), `unzip`/`zip`,
# and a running, logged-in SimplySign Desktop session so the certificate is
# reachable as a PKCS#11 token.
#
# Certum/SimplySign specifics are supplied as environment variables, since
# they depend on your distro's packaging and are only knowable once
# SimplySign Desktop is installed and logged in:
#
#   CERTUM_PKCS11_MODULE   path to the PKCS#11 module .so (SimplySign's own,
#                          or a p11-kit client proxy -- find it under
#                          /usr/lib*/pkcs11/ once SimplySign Desktop is
#                          installed)
#   CERTUM_CERT_URI        pkcs11: URI for the certificate object
#   CERTUM_KEY_URI         pkcs11: URI for the private key object
#   CERTUM_PIN             optional; PIN/password for the token, if it
#                          prompts for one
#
# To discover the URIs once the module path is known and a SimplySign
# session is open:
#   pkcs11-tool --module "$CERTUM_PKCS11_MODULE" --list-objects
#   # or: p11tool --list-all "pkcs11:module-path=$CERTUM_PKCS11_MODULE"
#
# Usage:
#   CERTUM_PKCS11_MODULE=/usr/lib64/pkcs11/p11-kit-client.so \
#   CERTUM_CERT_URI='pkcs11:model=SimplySign%20C;object=...;type=cert' \
#   CERTUM_KEY_URI='pkcs11:model=SimplySign%20C;object=...;type=private' \
#   ./scripts/sign-windows-release.sh --version 1.2.2 [--update-winget]

set -euo pipefail

version=""
repo="issinoho/loadbearer"
update_winget=0

while [ $# -gt 0 ]; do
  case "$1" in
    --version) version="$2"; shift 2 ;;
    --repo) repo="$2"; shift 2 ;;
    --update-winget) update_winget=1; shift ;;
    -h|--help) sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$version" ]; then
  version="$(grep -m1 '^version' "$(dirname "$0")/../Cargo.toml" | cut -d'"' -f2)"
fi
if [ -z "$version" ]; then
  echo "Could not determine a version; pass --version X.Y.Z" >&2
  exit 2
fi

for v in CERTUM_PKCS11_MODULE CERTUM_CERT_URI CERTUM_KEY_URI; do
  if [ -z "${!v:-}" ]; then
    echo "Missing \$$v -- see the header of this script for how to find it." >&2
    exit 2
  fi
done

tag="v$version"
target="x86_64-pc-windows-msvc"
archive_name="loadbearer-$version-$target"
zip_name="$archive_name.zip"

echo "== loadbearer $tag : re-signing the Windows release =="

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

echo "-- Downloading $zip_name and SHA256SUMS from $repo release $tag"
gh release download "$tag" --repo "$repo" --pattern "$zip_name" --pattern "SHA256SUMS" --clobber

echo "-- Extracting $zip_name"
unzip -q -o "$zip_name" -d extracted
exe_path="extracted/$archive_name/loadbearer.exe"
[ -f "$exe_path" ] || { echo "$exe_path not found inside the archive -- unexpected layout" >&2; exit 1; }

echo "-- Signing $exe_path"
osslsigncode_args=(
  sign
  -pkcs11module "$CERTUM_PKCS11_MODULE"
  -pkcs11cert "$CERTUM_CERT_URI"
  -key "$CERTUM_KEY_URI"
  -h sha256
  -t http://time.certum.pl/
  -in "$exe_path"
  -out "$exe_path.signed"
)
[ -n "${CERTUM_PIN:-}" ] && osslsigncode_args+=(-pass "$CERTUM_PIN")
osslsigncode "${osslsigncode_args[@]}"
mv "$exe_path.signed" "$exe_path"

echo "-- Verifying the signature"
osslsigncode verify -in "$exe_path"

echo "-- Re-packaging $zip_name"
rm -f "$zip_name"
( cd extracted && zip -qr "../$zip_name" "$archive_name" )

echo "-- Recomputing checksums"
zip_hash="$(sha256sum "$zip_name" | cut -d' ' -f1)"
exe_hash="$(sha256sum "$exe_path" | cut -d' ' -f1)"
awk -v zip="$zip_name" -v zh="$zip_hash" -v eh="$exe_hash" '
  $0 ~ (zip "$")               { print zh "  " zip; next }
  $0 ~ ("inside " zip "\\)$")  { print eh "  loadbearer.exe  (inside " zip ")"; next }
  { print }
' SHA256SUMS > SHA256SUMS.new
mv SHA256SUMS.new SHA256SUMS
cat SHA256SUMS

echo "-- Uploading the signed archive + updated checksums to the release"
gh release upload "$tag" "$zip_name" "SHA256SUMS" --repo "$repo" --clobber

cat <<EOF

Done. $zip_name on the $tag release is now signed; SHA256SUMS matches it.

Note: the build-provenance attestation on this release still points at the
original *unsigned* archive's hash (that's what CI attested) -- it will not
verify against this signed replacement. The Authenticode signature is the
trust anchor for this file now; SHA256SUMS and gh attestation verify still
work for the untouched Linux tarball.
EOF

if [ "$update_winget" -eq 1 ]; then
  if [ -z "${WINGET_TOKEN:-}" ]; then
    echo "--update-winget needs a PAT in \$WINGET_TOKEN" >&2
    exit 2
  fi
  echo ""
  echo "-- Re-submitting the winget manifest with the signed archive's hash"
  wingetcreate_cmd=(wingetcreate)
  if ! command -v wingetcreate >/dev/null 2>&1; then
    if command -v dotnet >/dev/null 2>&1; then
      dotnet tool install --global wingetcreate >/dev/null 2>&1 || true
      wingetcreate_cmd=("$HOME/.dotnet/tools/wingetcreate")
    else
      echo "wingetcreate not found and no dotnet to install it with" >&2
      exit 2
    fi
  fi
  url="https://github.com/$repo/releases/download/$tag/$zip_name"
  "${wingetcreate_cmd[@]}" update Issinoho.Loadbearer --version "$version" --urls "$url" --submit --token "$WINGET_TOKEN"
fi
