# Publishing loadbearer to a Launchpad PPA

The `.deb` attached to each GitHub release is a one-off download. A PPA is the
version people actually want: `apt` knows about it, so upgrades arrive with
everything else on the machine.

`debian/` is shared between the two. The only thing a PPA needs on top is a
*source* package that a Launchpad builder can compile — and Launchpad builders
have **no network**, so the source has to carry every crate with it.
`make-source.sh` in this directory is what produces that.

## One-time setup

1. **A GPG key registered with Launchpad.** Use the loadbearer release-signing
   key — `273E45FB7B21B6C2`, the one CI signs `SHA256SUMS` with (see
   `CODE_SIGNING_POLICY.md`). Its private half lives in `~/.gnupg` on the
   development machine; `gpg --list-secret-keys 273E45FB7B21B6C2` should show a
   `sec` line.

   Launchpad fetches the key from a keyserver rather than taking it inline, so
   publish it first:

   ```
   gpg --keyserver keyserver.ubuntu.com --send-keys 273E45FB7B21B6C2
   ```

   Then paste the fingerprint at <https://launchpad.net/~/+editpgpkeys>.
   Launchpad confirms by emailing a token **encrypted to the key**, which is
   why this one carries an encryption subkey and not just a signing one — a
   sign-only key cannot complete the step. The mail goes to the address in the
   key's UID (`iain@issinoho.com`); decrypt it and follow the link. Not
   instant, so do it before you need it.

2. **Create the PPA** at <https://launchpad.net/~issinoho/+activate-ppa>, named
   `loadbearer`. That gives `ppa:issinoho/loadbearer`.

   Under the PPA's *Change details* → *Processors*, pick the architectures to
   build for. This is the setting that decides how much work an upload makes:
   every enabled processor gets its own build of every series, so three series
   against six processors is eighteen builds, not three. Don't take the list
   below as current — it's a live setting, changed in the web UI, and the
   authoritative answer is that page (or the build records an upload actually
   produces). Three things worth knowing before you tick a box:

   - **Only amd64 and arm64 are architectures loadbearer is tested on.**
     Anything else is along for the ride, and a failure there says nothing
     about the release — but see the `cut-release.sh` note under *Gotchas*,
     because the script doesn't know that.
   - **riscv64 is the one that will keep you waiting.** Launchpad's riscv64
     builder pool is small and usually backlogged; on 1.2.4 every other
     architecture finished within about an hour while all three riscv64
     builds were still sitting in `Needs building`. It doesn't hold up
     publication of the others, but it does mean "all builds green" may be
     hours away. Note that *unticking a processor does not cancel builds it
     has already queued* — 1.2.4's three riscv64 records stayed `Needs
     building` after riscv64 was disabled. Cancel them on the build pages if
     you want the version to ever read as finished.
   - **i386 can be ticked but never builds.** Ubuntu dropped i386 as a build
     architecture; on jammy and later the checkbox produces no build records
     at all. Don't expect i386 packages from it.

3. **An SSH key registered with Launchpad**, at
   <https://launchpad.net/~/+editsshkeys>. This is not optional: Launchpad has
   retired anonymous FTP uploads — `ppa.launchpad.net:21` still accepts a TCP
   connection but never sends a banner, so `dput ppa:...` hangs and then
   reports `Connection failed, aborting. Check your network`, which is
   misleading. Uploads go over SFTP, authenticated by this key.

4. **Local tools**:

   ```
   sudo apt install dpkg-dev dput lintian
   ```

   `devscripts` is not required — the script drives `dpkg-buildpackage`
   directly and signs through it, so there is no `debuild`/`debsign` in the
   path.

   `dput` ships an `ssh-ppa` profile that is correct except for `login = *`,
   which it resolves to `$USER` — the local account name, not the Launchpad
   one. Override it once in `~/.dput.cf`:

   ```
   [ssh-ppa]
   login = issinoho
   ```

## Per release

Normally this is `scripts/cut-release.sh X.Y.Z`, which runs everything below
behind a preflight — see CLAUDE.md. What follows is what it does, and what to
run when something needs doing by hand.

After the tag is pushed and CI has published the GitHub release:

```
packaging/ppa/make-source.sh --ref v1.2.2 --key 273E45FB7B21B6C2
```

`--key` takes any spelling gpg understands — short id, long id, fingerprint or
email — and the script resolves it to a fingerprint before handing it on,
because `dpkg-buildpackage` warns about anything shorter.

That writes to `../ppa-1.2.2/` and finishes by printing the upload commands:

```
dput ssh-ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~jammy1_source.changes
dput ssh-ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~noble1_source.changes
dput ssh-ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~resolute1_source.changes
```

Note `ssh-ppa:`, not `ppa:` — the latter is the dead FTP path.

The script itself takes about five minutes -- roughly half vendoring and
writing the tarball, half lintian walking the 267 vendored crates. Run the
`dput` lines it prints. Launchpad emails an acceptance or rejection within a
minute or two, then queues the builds; watch them at
<https://launchpad.net/~issinoho/+archive/ubuntu/loadbearer/+packages>.

Budget about **an hour and a quarter from upload to `apt`**, most of it out of
your hands. On 1.2.4 (three series, six or seven processors each — nineteen
builds) the upload finished at 11:04, every architecture except riscv64 had
built by 11:57, and the publisher made them live at about 12:20. That last
step is the one people forget: a build reaching *Successfully built* does not
mean anyone can install it. The binaries sit at status `Pending` until
Launchpad's publisher next runs, which is what actually writes
`dists/<series>/main/binary-<arch>/Packages.gz`. Until then `apt` still offers
the previous version. `cut-release.sh` has a *Publication* step that waits for
this, so it doesn't call a release done while `apt` is still handing out the
last one — but if you uploaded by hand, that wait is yours to do.

To check whether a release is genuinely live, read the index rather than the
build page:

```
curl -sfL https://ppa.launchpadcontent.net/issinoho/loadbearer/ubuntu/dists/noble/main/binary-amd64/Packages.gz \
  | gunzip -c | awk '/^Version:/{print $2}' | sort -u
```

(The uncompressed `Packages` is a 404 — only the compressed index is served.)

Omit `--key` for a dry run: everything is built unsigned, which is enough to
check that the source package assembles and passes lintian, but Launchpad will
not accept the result.

## What the script does, and why

- **Exports a git ref**, not the working tree, so packaging an old tag from a
  dirty checkout still describes that tag. Both the version and the changelog
  are read out of the ref, and it refuses to continue if `Cargo.toml` and
  `debian/changelog` disagree — the same guard CI applies to the `.deb`.

- **Vendors the crates** into the exported tree with `cargo vendor --locked`.
  `debian/rules` already switches to `--offline` whenever a `vendor/` directory
  is present and points cargo at it, so the same `debian/` works both on a
  networked CI runner and on a Launchpad builder.

- **Prunes prebuilt binaries** from the vendor tree. Raw it is about 440 MB,
  of which some 154 MB is Windows import libraries (`.a`/`.lib`) that a
  Launchpad builder will never link against; most of the rest is the generated
  Rust source of the `windows` crates, which compresses away to almost nothing.
  Removing files invalidates cargo's per-file hashes, so each
  `.cargo-checksum.json` is rewritten with an empty `files` map — cargo's
  documented way of saying "this crate was repackaged, check the `.crate` hash
  only". The result is a ~36 MB `.orig.tar.gz`.

  The per-crate `Cargo.toml.orig` files go the same way. Nothing reads them at
  build time, and `dh_clean` deletes every `*.orig` in the tree during the
  clean step — so keeping them would make the build tree diverge from the
  tarball as soon as anything was built from it.

- **Builds one orig tarball and reuses it for every series.** Launchpad keys
  the tarball by filename and rejects a second upload of the same name with
  different bytes, so all the series uploads for one release must share it
  exactly. The tarball is written with fixed ownership and the commit's
  timestamp so that regenerating it from the same ref gives the same bytes.

- **Versions per series** as `1.2.2-1~noble1`. The `~` sorts *below* the plain
  `1.2.2-1`, so anyone who later gets the package from the Ubuntu archive
  proper is upgraded onto it rather than held back on the PPA copy. Re-uploading
  the same release to the same series needs a fresh version: pass
  `--ppa-build 2`.

## Gotchas

- **Launchpad accepts a given version once.** A rejected build cannot be fixed
  by re-uploading the same version — bump `--ppa-build`.
- **The builders have no network and no rustup.** `rustc-1.91`/`cargo-1.91` is
  published in `universe` on jammy, noble and resolute, which is why
  `debian/control` lists it first among the build-dep alternatives. Check that
  a new series still has it before adding it to `--series`.
- **Source-only uploads.** Launchpad builds the binaries itself; never upload a
  `.deb`.
- **gpg prompts once per run.** Dismiss or time out the passphrase prompt and
  the build dies at `signfile` with `gpg: signing failed: Operation cancelled`,
  after the vendoring work is already done. Just run it again.
- **`cut-release.sh` judges the release on `SHIPPED_ARCHES` only.** That's
  `amd64 arm64` at the top of the script. It still prints every build record,
  marking the ones that count with a `*`, but only those gate the release — so
  a riscv64 build that fails, or sits queued forever against a processor that
  has since been disabled, no longer reports the release as broken or hangs
  the wait. Widen the list there if loadbearer ever gets tested somewhere new.
- **It needs room, and a little RAM.** The vendored tree is ~300 MB unpacked.
  The script keeps lintian's scratch space on the output directory's
  filesystem rather than under `/tmp`, which on a normal desktop is a tmpfs far
  too small for it, so allow a couple of GB free there. It also runs lintian
  one source package at a time: handing it all three at once makes it hold
  three expanded trees concurrently, which on a 7 GB machine got the 1.4.0 run
  OOM-killed at the lintian step — after the vendoring and signing were done,
  but before the upload commands were printed.
