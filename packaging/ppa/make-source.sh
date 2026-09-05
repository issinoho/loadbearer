#!/usr/bin/env bash
#
# Build signed source packages for a Launchpad PPA, one per Ubuntu series.
#
# Launchpad's builders have no network, so the source package has to carry
# every crate it needs. This script exports a git ref, vendors the crates into
# it, and wraps the result as loadbearer_<version>.orig.tar.gz -- then, for
# each series, unpacks that same tarball, drops debian/ back in with a
# series-suffixed version, and runs dpkg-buildpackage -S over it.
#
# The orig tarball is built once and reused for every series on purpose:
# Launchpad keys it by name and rejects a second upload of the same filename
# with different bytes, so all the series uploads for one release must share
# it exactly.
#
# See packaging/ppa/README.md for the surrounding procedure.

set -euo pipefail

REF="HEAD"
SERIES="jammy,noble,resolute"
PPA_BUILD=1
KEY=""
OUTDIR=""

usage() {
	cat <<EOF
Usage: $0 [options]

  -r, --ref REF        git ref to package (default: HEAD)
  -s, --series LIST    comma-separated Ubuntu series (default: $SERIES)
  -n, --ppa-build N    PPA build number within the series (default: 1);
                       bump it to re-upload the same release to the same series
  -k, --key KEYID      GPG key to sign with; without it the packages are
                       unsigned and Launchpad will not accept them
  -o, --output DIR     where to write (default: <repo>/../ppa-<version>)
  -h, --help           this

Example:
  $0 --ref v1.2.2 --key 0xDEADBEEF
EOF
}

while [ $# -gt 0 ]; do
	case "$1" in
		-r|--ref) REF="$2"; shift 2 ;;
		-s|--series) SERIES="$2"; shift 2 ;;
		-n|--ppa-build) PPA_BUILD="$2"; shift 2 ;;
		-k|--key) KEY="$2"; shift 2 ;;
		-o|--output) OUTDIR="$2"; shift 2 ;;
		-h|--help) usage; exit 0 ;;
		*) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
	esac
done

for tool in git cargo dpkg-buildpackage dpkg-source python3 tar; do
	command -v "$tool" >/dev/null || {
		echo "error: $tool is not installed (dpkg-* come from dpkg-dev)" >&2
		exit 1
	}
done

REPO="$(git rev-parse --show-toplevel)"
cd "$REPO"

git rev-parse --verify --quiet "$REF^{commit}" >/dev/null || {
	echo "error: '$REF' is not a commit in this repository" >&2
	exit 1
}

# Tags cut before the Debian packaging landed have no debian/ at all, and the
# failure otherwise surfaces as an opaque complaint from git show.
git cat-file -e "$REF:debian/changelog" 2>/dev/null || {
	echo "error: '$REF' contains no debian/changelog -- the Debian packaging was added" >&2
	echo "       after that ref, so there is nothing there to build a source package from." >&2
	echo "       Use a ref that has it (--ref main), or a later tag." >&2
	exit 1
}

# Read both versions out of the ref rather than the working tree, so that
# packaging an old tag from a dirty checkout still describes that tag.
VERSION="$(git show "$REF:Cargo.toml" | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
DEB_VERSION="$(git show "$REF:debian/changelog" | sed -n '1s/^[^(]*(\([^)]*\)).*/\1/p')"
DEB_UPSTREAM="${DEB_VERSION%-*}"

[ -n "$VERSION" ] || { echo "error: no version in Cargo.toml at $REF" >&2; exit 1; }

# The same guard CI applies to the .deb: the version comes from the changelog,
# so a forgotten entry would upload a package labelled with the last release.
if [ "$VERSION" != "$DEB_UPSTREAM" ]; then
	echo "error: debian/changelog is at $DEB_UPSTREAM but Cargo.toml is at $VERSION at ref $REF" >&2
	echo "       add a changelog entry for $VERSION-1 (see CLAUDE.md, 'Cutting a release')" >&2
	exit 1
fi

[ -n "$OUTDIR" ] || OUTDIR="$REPO/../ppa-$VERSION"
mkdir -p "$OUTDIR"
OUTDIR="$(cd "$OUTDIR" && pwd)"

ORIG="$OUTDIR/loadbearer_${VERSION}.orig.tar.gz"
SRCDIR="$OUTDIR/loadbearer-$VERSION"

echo "==> loadbearer $VERSION from $REF -> $OUTDIR"

# ---------------------------------------------------------------- orig tarball

rm -rf "$SRCDIR"
mkdir -p "$SRCDIR"
git archive --format=tar "$REF" | tar -x -C "$SRCDIR"

# debian/ belongs to the .debian.tar.xz, not to the upstream tarball; it goes
# back in per-series below, with the series-suffixed version.
rm -rf "$SRCDIR/debian"

echo "==> vendoring crates (this needs network; the builders will not have it)"
( cd "$SRCDIR" && cargo vendor --locked --versioned-dirs vendor >/dev/null )

# A raw vendor tree is ~440 MB, of which ~154 MB is prebuilt Windows import
# libraries that a Launchpad builder will never link against. Drop every such
# blob: it keeps the upload to a sane size and keeps precompiled binaries out
# of a source package, which is what a source package is for.
#
# Cargo.toml.orig goes too. cargo vendor writes one beside each crate's
# normalised Cargo.toml and nothing in a build reads it, but dh_clean deletes
# every *.orig in the tree during the clean step -- so leaving them in makes
# the build tree diverge from the tarball the moment anything is built from it,
# and buries the real dpkg-source output under one deletion warning per crate.
echo "==> pruning prebuilt binaries from vendor/"
find "$SRCDIR/vendor" -type f \
	\( -name '*.a' -o -name '*.lib' -o -name '*.dll' -o -name '*.exe' -o -name '*.pdb' \
	   -o -name 'Cargo.toml.orig' \) \
	-delete

# Removing files invalidates the per-file hashes cargo checks on every build.
# An empty "files" map is cargo's documented way of saying "this crate was
# repackaged, verify the .crate hash only", which is exactly the situation.
python3 - "$SRCDIR/vendor" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
n = 0
for path in root.glob("*/.cargo-checksum.json"):
    data = json.loads(path.read_text())
    path.write_text(json.dumps({"package": data.get("package"), "files": {}}))
    n += 1
print(f"    neutralised {n} .cargo-checksum.json files")
PY

echo "==> writing $(basename "$ORIG")"
# Fixed ownership and a mtime taken from the commit, so that regenerating the
# tarball from the same ref gives the same bytes -- Launchpad compares them.
COMMIT_TS="$(git log -1 --format=%ct "$REF")"
rm -f "$ORIG"
tar --sort=name --owner=0 --group=0 --numeric-owner \
	--mtime="@$COMMIT_TS" \
	-czf "$ORIG" -C "$OUTDIR" "loadbearer-$VERSION"
ls -l "$ORIG" | awk '{printf "    %.1f MB\n", $5/1048576}'

# ------------------------------------------------------------ source packages

if [ -n "$KEY" ]; then
	# dpkg-buildpackage warns on anything shorter than a fingerprint, so accept
	a short id, long id or email and hand it the fingerprint regardless.
	FPR="$(gpg --with-colons --list-keys "$KEY" 2>/dev/null | awk -F: '$1=="fpr"{print $10; exit}')"
	if [ -z "$FPR" ]; then
		echo "error: no GPG key in your keyring matches '$KEY'" >&2
		exit 1
	fi
	echo "==> signing with $FPR"
	SIGN_ARGS=(-k"$FPR")
else
	SIGN_ARGS=(-us -uc)
	echo "==> WARNING: no --key given; the source packages will be unsigned"
	echo "    and Launchpad will reject them. This is a dry run."
fi

CHANGES=()
IFS=',' read -r -a SERIES_LIST <<< "$SERIES"
for series in "${SERIES_LIST[@]}"; do
	echo "==> building source package for $series"
	rm -rf "$SRCDIR"
	tar -xzf "$ORIG" -C "$OUTDIR"
	git archive --format=tar "$REF" debian | tar -x -C "$SRCDIR"

	# 1.2.2-1~noble1 sorts below the plain 1.2.2-1, so a user who later gets
	# the package from the archive proper is upgraded rather than held back.
	sed -i "1s/^loadbearer ([^)]*) [^;]*;/loadbearer (${VERSION}-1~${series}${PPA_BUILD}) ${series};/" \
		"$SRCDIR/debian/changelog"
	head -n1 "$SRCDIR/debian/changelog" | sed 's/^/    /'

	# -d skips the build-dependency check: this machine has no reason to have
	# rustc-1.91 installed, and a source-only build never compiles anything.
	( cd "$SRCDIR" && dpkg-buildpackage -S -sa -d "${SIGN_ARGS[@]}" ) >/dev/null
	CHANGES+=("$OUTDIR/loadbearer_${VERSION}-1~${series}${PPA_BUILD}_source.changes")
done

rm -rf "$SRCDIR"

if command -v lintian >/dev/null; then
	echo "==> lintian"
	# lintian unpacks each source package in full -- some 300 MB apiece once the
	# vendored crates are expanded -- and does it under TMPDIR, which on a normal
	# desktop is a tmpfs a fraction of RAM in size. Point it at the output
	# directory's filesystem instead, or three series in one run exhausts /tmp
	# and dpkg-source fails with a bare non-zero status.
	LINTIAN_TMP="$OUTDIR/.lintian-tmp"
	rm -rf "$LINTIAN_TMP"
	mkdir -p "$LINTIAN_TMP"
	# Warnings are expected (a PPA upload closes no bug and vendors its
	# dependencies); only errors should stop an upload.
	TMPDIR="$LINTIAN_TMP" lintian --fail-on error "${CHANGES[@]}" || {
		rm -rf "$LINTIAN_TMP"
		echo "error: lintian found errors -- not printing upload commands" >&2
		exit 1
	}
	rm -rf "$LINTIAN_TMP"
else
	echo "==> lintian not installed, skipping the check"
fi

echo
echo "Done. Upload with:"
for c in "${CHANGES[@]}"; do
	echo "  dput ppa:issinoho/loadbearer $c"
done
