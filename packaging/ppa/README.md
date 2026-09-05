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

   Under the PPA's *Change details* → *Processors*, enable `arm64` alongside
   `amd64` if you want Arm builds; PPAs default to amd64 only.

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
<https://launchpad.net/~issinoho/+archive/ubuntu/loadbearer/+packages>. A full
build takes roughly ten minutes per series.

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
- **It needs room.** The vendored tree is ~300 MB unpacked, and lintian expands
  every source package at once, so allow a couple of GB free on the output
  directory's filesystem. The script keeps lintian's scratch space there rather
  than under `/tmp`, which on a normal desktop is a tmpfs far too small for it.
