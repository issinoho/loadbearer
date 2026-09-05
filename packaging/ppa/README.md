# Publishing loadbearer to a Launchpad PPA

The `.deb` attached to each GitHub release is a one-off download. A PPA is the
version people actually want: `apt` knows about it, so upgrades arrive with
everything else on the machine.

`debian/` is shared between the two. The only thing a PPA needs on top is a
*source* package that a Launchpad builder can compile — and Launchpad builders
have **no network**, so the source has to carry every crate with it.
`make-source.sh` in this directory is what produces that.

## One-time setup

1. **Launchpad account** with a **GPG key registered and confirmed** at
   <https://launchpad.net/~/+editpgpkeys>. Launchpad verifies the key by
   emailing an encrypted token, so this is not instant — do it before you need
   it. This is the same key already used to sign `SHA256SUMS` on the GitHub
   releases (see `CODE_SIGNING_POLICY.md`); nothing separate is needed.

2. **Create the PPA** at <https://launchpad.net/~issinoho/+activate-ppa>, named
   `loadbearer`. That gives `ppa:issinoho/loadbearer`.

   Under the PPA's *Change details* → *Processors*, enable `arm64` alongside
   `amd64` if you want Arm builds; PPAs default to amd64 only.

3. **Local tools**:

   ```
   sudo apt install dpkg-dev dput lintian
   ```

   `devscripts` is not required — the script drives `dpkg-buildpackage`
   directly and signs through it, so there is no `debuild`/`debsign` in the
   path. `dput` understands the `ppa:owner/name` shorthand out of the box, so
   there is no `.dput.cf` to write.

## Per release

After the tag is pushed and CI has published the GitHub release:

```
packaging/ppa/make-source.sh --ref v1.2.2 --key <your-key-id>
```

That writes to `../ppa-1.2.2/` and finishes by printing the upload commands:

```
dput ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~jammy1_source.changes
dput ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~noble1_source.changes
dput ppa:issinoho/loadbearer ../ppa-1.2.2/loadbearer_1.2.2-1~resolute1_source.changes
```

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

## When the PPA is live

Add this to `README.md`, above the Debian/Ubuntu `.deb` section — it is left
out until the PPA exists so the instructions never point at a 404:

````markdown
### Ubuntu (PPA)

```
sudo add-apt-repository ppa:issinoho/loadbearer
sudo apt install loadbearer
```

Upgrades then arrive through `apt` like anything else. Supported on 22.04
(jammy), 24.04 (noble) and 26.04 (resolute).
````
