#!/usr/bin/env bash
#
# Cut a loadbearer release, end to end: version bump, changelogs, tag, and
# once CI has published the GitHub release, the PPA source packages, their
# upload, and the Launchpad builds that follow.
#
# The front door for a release, the way scripts/publish-loadbearer.ps1 is the
# front door for Windows signing -- and like it, the point is to fail before
# anything is spent rather than halfway through. Everything checked below is
# something that has actually gone wrong: a passphrase prompt dismissed after
# five minutes of vendoring, dput authenticating as the wrong user after the
# build, lintian filling /tmp, a version Launchpad had already seen.
#
# The irreversible steps -- the tag push and the upload -- are each confirmed
# separately, so stopping at either leaves nothing to undo.
#
#   scripts/cut-release.sh 1.2.3              # the whole thing
#   scripts/cut-release.sh 1.2.3 --dry-run    # say what would happen
#   scripts/cut-release.sh 1.2.2 --ppa-only   # PPA for a release already out
#
# See CLAUDE.md "Cutting a release" and packaging/ppa/README.md.

set -euo pipefail

REPO="issinoho/loadbearer"
DPUT_TARGET="ssh-ppa:issinoho/loadbearer"
LP_USER="issinoho"
LP_PPA="loadbearer"
# The release signing key from CODE_SIGNING_POLICY.md. Update both together.
KEY="C3482C916797D77F38926A77273E45FB7B21B6C2"
SERIES="jammy,noble,resolute"
BRANCH="main"

VERSION=""
DRY_RUN=0
ASSUME_YES=0
PPA_ONLY=0
SKIP_PPA=0
SKIP_CHECKS=0
FORCE=0

usage() {
	cat <<EOF
Usage: $0 X.Y.Z [options]

  -k, --key FPR      signing key (default: the release key)
  -s, --series LIST  Ubuntu series (default: $SERIES)
      --ppa-only     skip bump/tag/wait; just do the PPA for an existing release
      --skip-ppa     stop once the GitHub release is published
      --skip-checks  don't run fmt/clippy/test before tagging
      --dry-run      print what would happen, change nothing
  -y, --yes          don't prompt before the tag push or the upload
      --force        downgrade preflight failures that are advisory to warnings
  -h, --help         this
EOF
}

while [ $# -gt 0 ]; do
	case "$1" in
		-k|--key) KEY="$2"; shift 2 ;;
		-s|--series) SERIES="$2"; shift 2 ;;
		--ppa-only) PPA_ONLY=1; shift ;;
		--skip-ppa) SKIP_PPA=1; shift ;;
		--skip-checks) SKIP_CHECKS=1; shift ;;
		--dry-run) DRY_RUN=1; shift ;;
		-y|--yes) ASSUME_YES=1; shift ;;
		--force) FORCE=1; shift ;;
		-h|--help) usage; exit 0 ;;
		-*) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
		*) [ -z "$VERSION" ] || { echo "unexpected argument: $1" >&2; exit 2; }
		   VERSION="${1#v}"; shift ;;
	esac
done

# ------------------------------------------------------------------ plumbing

BOLD=""; RED=""; DIM=""; RESET=""
if [ -t 1 ]; then BOLD=$'\e[1m'; RED=$'\e[31m'; DIM=$'\e[2m'; RESET=$'\e[0m'; fi

die()  { printf '\n%s  %s%s\n\n' "$RED" "$*" "$RESET" >&2; exit 1; }
step() { printf '\n%s%s%s\n' "$BOLD" "$*" "$RESET"; }
ok()   { printf '  %s\n' "$*"; }
warn() { printf '  %s! %s%s\n' "$DIM" "$*" "$RESET"; }

# An advisory check: fatal normally, a warning under --force. For the ones
# that can be wrong about the world rather than about the repo.
advisory() {
	if [ "$FORCE" -eq 1 ]; then warn "$1 (ignored: --force)"; else die "$1"; fi
}

run() {
	if [ "$DRY_RUN" -eq 1 ]; then printf '  %swould run: %s%s\n' "$DIM" "$*" "$RESET"; return 0; fi
	"$@"
}

confirm() {
	[ "$ASSUME_YES" -eq 1 ] && return 0
	[ "$DRY_RUN" -eq 1 ] && return 0
	printf '\n  %s [y/N] ' "$1"
	read -r reply </dev/tty
	case "$reply" in [yY]|[yY][eE][sS]) return 0 ;; *) die "Stopped. Nothing has been pushed or uploaded." ;; esac
}

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "Not in a git repository."
cd "$REPO_ROOT"

[ -n "$VERSION" ] || { usage >&2; die "Which version? e.g. $0 1.2.3"; }
printf '%s' "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' \
	|| die "'$VERSION' doesn't look like a version -- expected something like 1.2.3."
TAG="v$VERSION"

CARGO_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n1)"

printf '\n%sloadbearer %s%s\n' "$BOLD" "$TAG" "$RESET"
printf 'Repository: %s   PPA: ppa:%s/%s\n' "$REPO" "$LP_USER" "$LP_PPA"
[ "$DRY_RUN" -eq 1 ] && printf '%sDry run -- nothing will be changed, pushed or uploaded.%s\n' "$DIM" "$RESET"

# ----------------------------------------------------------------- preflight

step "Preflight"

for tool in git gh cargo gpg python3 curl; do
	command -v "$tool" >/dev/null || die "$tool isn't on PATH."
done
if [ "$SKIP_PPA" -eq 0 ]; then
	for tool in dpkg-buildpackage dpkg-source dput; do
		command -v "$tool" >/dev/null \
			|| die "$tool isn't on PATH. Install it: sudo apt install dpkg-dev dput lintian"
	done
fi
ok "tools present"

gh auth status >/dev/null 2>&1 || die "gh isn't authenticated. Run: gh auth login"
ok "gh authenticated"

if [ "$PPA_ONLY" -eq 0 ]; then
	current_branch="$(git rev-parse --abbrev-ref HEAD)"
	[ "$current_branch" = "$BRANCH" ] \
		|| advisory "On branch '$current_branch', not '$BRANCH'. Releases are cut from $BRANCH."

	# Not "clean", but "clean apart from what this script is going to commit
	# anyway". Writing the release notes and then running this is the intended
	# order, so refusing the uncommitted CHANGELOG.md would refuse the normal
	# case; anything else uncommitted would be swept into the release commit,
	# which is what actually needs preventing.
	RELEASE_FILES="Cargo.toml Cargo.lock CHANGELOG.md debian/changelog"
	stray=""
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		path="${line:3}"
		case " $RELEASE_FILES " in
			*" $path "*) ;;
			*) stray="$stray $path" ;;
		esac
	done <<< "$(git status --porcelain)"
	[ -z "$stray" ] \
		|| die "Uncommitted changes outside the release files:$stray

  They'd be swept into the release commit. Commit or stash them first.
  (Uncommitted $RELEASE_FILES is fine -- this script commits those.)"

	git fetch -q origin "$BRANCH" 2>/dev/null || true
	if [ -n "$(git rev-list "origin/$BRANCH..HEAD" 2>/dev/null)" ] || \
	   [ -n "$(git rev-list "HEAD..origin/$BRANCH" 2>/dev/null)" ]; then
		advisory "$BRANCH has diverged from origin/$BRANCH. Push or pull first, so the tag lands on what everyone else sees."
	else
		ok "on $BRANCH, clean, in step with origin"
	fi

	dpkg --compare-versions "$VERSION" gt "$CARGO_VERSION" \
		|| die "Cargo.toml is at $CARGO_VERSION, so $VERSION isn't an increase. Releases only go forwards."
	ok "$CARGO_VERSION -> $VERSION"

	git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
		&& die "Tag $TAG already exists locally. Delete it, or pick another version."
	git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1 \
		&& die "Tag $TAG already exists on origin. A published version can't be recut -- bump the patch version instead."
	ok "$TAG is free"
fi

if [ "$SKIP_PPA" -eq 0 ]; then
	# The whole reason the PPA source package exists is that Launchpad builders
	# have no network, so the signing key has to be here, not in CI.
	gpg --list-secret-keys "$KEY" >/dev/null 2>&1 \
		|| die "No secret key for $KEY in your keyring. Uploads must be signed by it -- see packaging/ppa/README.md."
	ok "signing key present"

	if grep -qs '^\[ssh-ppa\]' "$HOME/.dput.cf"; then
		ok "~/.dput.cf overrides the ssh-ppa login"
	else
		advisory "~/.dput.cf has no [ssh-ppa] section. dput would authenticate as '$USER' rather than '$LP_USER' and fail after the build. See packaging/ppa/README.md."
	fi

	# Read-only, so it runs in a dry run too -- validating the setup is most of
	# what a dry run is for, and this is the check with the longest fuse.
	#
	# ssh -T rather than an sftp session: Launchpad authenticates, answers "No
	# shells on this server." and is done in half a second, whereas an sftp
	# session never closes its channel after quit, so it only ends when the
	# timeout kills it -- a 30-second stall reported as a failure.
	lp_ssh="$(timeout 20 ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
		-T "$LP_USER@ppa.launchpad.net" 2>&1 || true)"
	if printf '%s' "$lp_ssh" | grep -qi 'permission denied\|no supported authentication'; then
		advisory "Launchpad refused your SSH key for $LP_USER, so dput can't upload. Register it at https://launchpad.net/~/+editsshkeys and make sure it's in your agent (ssh-add -l)."
	elif printf '%s' "$lp_ssh" | grep -q 'No shells on this server'; then
		ok "Launchpad accepts your SSH key"
	else
		warn "couldn't verify Launchpad SSH access: ${lp_ssh%%$'\n'*}"
	fi

	# lintian unpacks every source package; the vendored tree is ~300 MB each.
	avail_mb="$(df -Pm "$(dirname "$REPO_ROOT")" | awk 'NR==2 {print $4}')"
	if [ "${avail_mb:-0}" -lt 3000 ]; then
		advisory "Only ${avail_mb} MB free where the source packages are written. Allow a few GB -- lintian expands all of them at once."
	else
		ok "${avail_mb} MB free for the build"
	fi

	# Launchpad accepts a given version once, full stop.
	existing="$(curl -sfG "https://api.launchpad.net/1.0/~$LP_USER/+archive/ubuntu/$LP_PPA" \
		--data-urlencode "ws.op=getPublishedSources" 2>/dev/null \
		| python3 -c "
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
v='$VERSION'
print(' '.join(sorted({e['source_package_version'] for e in d.get('entries',[])
                       if e['source_package_version'].startswith(v+'-')})))
" 2>/dev/null || true)"
	if [ -n "$existing" ]; then
		advisory "The PPA already has $existing. Launchpad won't take the same version twice -- re-upload needs packaging/ppa/make-source.sh --ppa-build 2 run by hand."
	else
		ok "$VERSION isn't in the PPA yet"
	fi
fi

# --------------------------------------------------------- prepare and tag

if [ "$PPA_ONLY" -eq 0 ]; then
	step "Changelogs"

	today_human="$(LC_ALL=C date '+%a, %-d %b %Y')"
	if grep -q "^## $VERSION " CHANGELOG.md; then
		ok "CHANGELOG.md already has a $VERSION section"
	else
		die "CHANGELOG.md has no '## $VERSION' section. Write the release notes first -- the release workflow lifts that section verbatim into the GitHub Release, so it isn't something to generate. Add:

  ## $VERSION - $today_human"
	fi

	deb_current="$(sed -n '1s/^[^(]*(\([^)]*\)).*/\1/p' debian/changelog)"
	if [ "${deb_current%-*}" = "$VERSION" ]; then
		ok "debian/changelog already at $deb_current"
	else
		maintainer="$(sed -n 's/^Maintainer: //p' debian/control)"
		ok "adding debian/changelog entry $VERSION-1"
		if [ "$DRY_RUN" -eq 0 ]; then
			{
				printf 'loadbearer (%s-1) unstable; urgency=medium\n\n' "$VERSION"
				printf '  * New upstream release %s; see CHANGELOG.md for the details.\n\n' "$VERSION"
				printf ' -- %s  %s\n\n' "$maintainer" "$(date -R)"
				cat debian/changelog
			} > debian/changelog.new
			mv debian/changelog.new debian/changelog
		fi
	fi

	step "Version bump"
	if [ "$CARGO_VERSION" = "$VERSION" ]; then
		ok "Cargo.toml already at $VERSION"
	else
		ok "Cargo.toml $CARGO_VERSION -> $VERSION"
		[ "$DRY_RUN" -eq 0 ] && sed -i "0,/^version = \"$CARGO_VERSION\"/s//version = \"$VERSION\"/" Cargo.toml
	fi
	# Cargo.lock carries the workspace version, and every CI command uses
	# --locked, so a stale lock fails the build rather than being refreshed.
	ok "refreshing Cargo.lock"
	run cargo build --quiet

	if [ "$SKIP_CHECKS" -eq 0 ]; then
		step "Checks (what CI runs)"
		run cargo fmt --all --check && ok "fmt"
		run cargo clippy --all-targets --locked -- -D warnings && ok "clippy"
		run cargo test --locked --quiet && ok "tests"
	else
		warn "checks skipped"
	fi

	step "Commit and tag"
	printf '  %s\n' "$(git diff --stat | tail -1)"
	confirm "Commit, tag $TAG and push to origin/$BRANCH?"
	run git add Cargo.toml Cargo.lock CHANGELOG.md debian/changelog
	run git commit -q -m "Release $VERSION"
	run git tag -a "$TAG" -m "loadbearer $VERSION"
	run git push -q origin "$BRANCH" --follow-tags
	[ "$DRY_RUN" -eq 0 ] && ok "pushed $TAG"

	step "Waiting for the release workflow"
	if [ "$DRY_RUN" -eq 1 ]; then
		ok "would wait for $TAG to appear as a published release"
	else
		printf '  '
		for _ in $(seq 1 120); do
			state="$(gh release view "$TAG" -R "$REPO" --json isDraft -q .isDraft 2>/dev/null || echo missing)"
			[ "$state" = "false" ] && break
			printf '.'
			sleep 30
		done
		printf '\n'
		[ "$state" = "false" ] \
			|| die "$TAG still isn't a published release after an hour. Check the workflow: gh run list -R $REPO"
		ok "release $TAG is published"
	fi
fi

[ "$SKIP_PPA" -eq 1 ] && { step "Done"; ok "Stopped before the PPA (--skip-ppa)."; exit 0; }

# ------------------------------------------------------------------- the PPA

step "PPA"

# Prime the gpg agent now. dpkg-buildpackage signs at the very end of each
# source package, so a dismissed or timed-out prompt there throws away the
# vendoring -- five minutes of work -- with "signing failed: Operation
# cancelled". Better to ask before any of it.
if [ "$DRY_RUN" -eq 0 ]; then
	tmp_sig="$(mktemp)"
	if echo priming | gpg --batch --yes --local-user "$KEY" --detach-sign \
		--output "$tmp_sig" - 2>/dev/null; then
		ok "gpg agent primed (no prompt during the build)"
	else
		rm -f "$tmp_sig"
		die "Couldn't sign with $KEY -- wrong passphrase, or the prompt was dismissed."
	fi
	rm -f "$tmp_sig"
fi

OUTDIR="$REPO_ROOT/../ppa-$VERSION"
ppa_ref="$TAG"
# Tags cut before debian/ existed can't be packaged; make-source.sh says so
# too, but suggesting the fix here saves a round trip.
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1 \
	&& ! git cat-file -e "$TAG:debian/changelog" 2>/dev/null; then
	advisory "$TAG has no debian/ -- it predates the packaging. Falling back to $BRANCH."
	ppa_ref="$BRANCH"
elif [ "$DRY_RUN" -eq 1 ] && ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
	ok "would package $TAG (created earlier in the real run)"
fi

run "$REPO_ROOT/packaging/ppa/make-source.sh" \
	--ref "$ppa_ref" --key "$KEY" --series "$SERIES" --output "$OUTDIR"

step "Upload"
changes=()
if [ "$DRY_RUN" -eq 0 ]; then
	while IFS= read -r f; do changes+=("$f"); done < <(ls -1 "$OUTDIR"/*_source.changes 2>/dev/null || true)
	[ "${#changes[@]}" -gt 0 ] || die "No _source.changes files in $OUTDIR -- did make-source.sh fail?"
	for c in "${changes[@]}"; do printf '  %s\n' "$(basename "$c")"; done
fi
confirm "Upload these to ppa:$LP_USER/$LP_PPA? Launchpad accepts each version once."
for c in "${changes[@]}"; do
	run dput "$DPUT_TARGET" "$c"
done
if [ "$DRY_RUN" -eq 1 ]; then
	ok "would build the source packages, then upload each one"
else
	ok "uploaded"
fi

step "Builds"
if [ "$DRY_RUN" -eq 1 ]; then
	ok "would watch Launchpad until every build finishes"
	exit 0
fi

echo "  waiting for Launchpad (this takes about an hour per architecture)"
for _ in $(seq 1 240); do
	summary="$(curl -sfG "https://api.launchpad.net/1.0/~$LP_USER/+archive/ubuntu/$LP_PPA" \
		--data-urlencode "ws.op=getBuildRecords" 2>/dev/null \
		| python3 -c "
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
v='$VERSION'
rows=[e for e in d.get('entries',[]) if v in e.get('title','')]
if not rows: sys.exit(0)
live={'Needs building','Currently building','Uploading build'}
print('PENDING' if any(r.get('buildstate') in live for r in rows) else 'DONE')
for r in rows:
    t=r.get('title','').split(' in ubuntu')[0].replace(' build of loadbearer','')
    print(f\"  {t:44} {r.get('buildstate')}\")
" 2>/dev/null || true)"
	[ -z "$summary" ] && { sleep 60; continue; }
	[ "${summary%%$'\n'*}" = "DONE" ] && break
	sleep 60
done

printf '%s\n' "${summary#*$'\n'}"
if printf '%s' "$summary" | grep -q 'Failed\|Chroot problem\|Dependency wait'; then
	die "Some builds didn't succeed. Logs: https://launchpad.net/~$LP_USER/+archive/ubuntu/$LP_PPA/+packages"
fi

step "Done"
ok "loadbearer $VERSION is released and in the PPA."
