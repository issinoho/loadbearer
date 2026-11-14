# Privacy

**loadbearer collects no data, and sends nothing anywhere unless you tell it
to.**

It is a local command-line tool. It has no telemetry, no analytics, no crash
reporting, no update check, and no account or licence system. There is no
server side of its own. The only things it will send are a result file and,
when a scheduled run is skipped, a small status record, both to a
[loadbearer-fleet](https://github.com/issinoho/loadbearer-fleet) server you
name with `run --upload URL` — see [Network](#network).

## What it writes, and where — all local

- **Result files** (`--output result.json`, or `--json` to stdout). These
  contain a machine inventory — hostname, CPU model/vendor, OS and kernel
  version with the OS build, release and edition, RAM with its memory-module
  layout (slots filled, and each module's size, type and speed), disk models /
  mount points / capacities, and the GPU and battery details where present —
  alongside the benchmark numbers. That is deliberate: it is what makes
  `loadbearer compare` and `loadbearer baseline` meaningful.

  Each result also carries a `run` block: when the benchmarks started, how
  long they took, and a run ID. The ID is **random, generated fresh for every
  run from the operating system's random-number source, and derived from
  nothing** — not from the machine, the time or the results — so it can't be
  recomputed and identifies one run, never the machine. If the OS can't
  supply randomness the ID is left out rather than made from something
  guessable.

  Since 1.3.0 the inventory also carries a `machine.identity` block: the
  machine's **SMBIOS system UUID, chassis serial number and asset tag**, and
  the OS install's own identifier (Windows `MachineGuid`, Linux
  `/etc/machine-id`). These are read, never written, and exist so that a fleet
  collector can tell repeat runs of one machine apart from a machine it hasn't
  seen — `hostname` is renameable and gets reissued. They are also the most
  identifying thing in the file: a serial number ties the result to a specific
  physical unit and, through a purchase or asset record, potentially to a
  person. Fields are absent where the firmware doesn't report them or the OS
  won't say without root, and the whole block is absent when none could be
  read. Nothing suppresses it selectively today; if you need a result file
  without it, strip `machine.identity` before sharing.

  The inventory also carries `machine.firmware` (the system vendor and model,
  and the BIOS version and release date), `machine.drives` (each NVMe or SATA
  drive's model and health counters: power-on hours, power cycles, wear, data
  written, error counts) and `machine.age`, an estimate of how old the machine is,
  derived from those and from the CPU model and battery cycle count, and
  `machine.power` (mains or battery and the power settings in force — on
  Windows that includes the power plan's name, which an IT department may
  have chosen), `machine.memory` (how many memory slots are filled, and each
  module's size, type, form factor and speed) and `machine.os_detail` (the OS
  build, release and edition). None of these is per-unit: every machine of a model reports the same model and
  firmware strings, and no drive or memory-module serial number is read.

  Three further things **are** per-unit, and that is exactly why they are
  not collected unless you ask: when the machine last booted, whether the run
  was elevated, and its security posture (Secure Boot, TPM, disk
  encryption). See [Collected only when asked for](#collected-only-when-asked-for).

  Anything a `--tag` puts in the file is text you supplied, so don't put
  personal data in one.

  Treat a result file like any other machine fingerprint before you share it.
  loadbearer uploads one only when `run --upload` says where to; otherwise
  you choose what to do with it.
- **A diagnostic log** at `%LOCALAPPDATA%\loadbearer\loadbearer.log` (Windows) /
  `$XDG_CACHE_HOME/loadbearer/loadbearer.log` (Linux), or wherever `--log-file`
  points. It records the run's settings, per-benchmark timings, and errors. It
  contains no personal data beyond the local file path in its header —
  deliberately not the hostname, and not the `machine.identity` values. At
  `--log-level debug` it records whether each of those was *readable*, not what
  it said, because a log is the artefact people paste into a bug report while a
  result file is one they choose to share. The exception is an identifier
  rejected as an OEM placeholder ("To Be Filled By O.E.M." and friends), which
  is logged as-is: it describes the firmware, not the machine. For what is
  [collected only when asked for](#collected-only-when-asked-for), the log
  records which categories were asked for and, at `debug`, whether each part
  was readable — never the values, and nothing at all about a category that
  wasn't asked for. `--no-log` disables the log entirely.
- **A scratch file** in `--target-dir` for the disk benchmark, deleted when the
  run ends.

Nothing is written to the registry. No elevated privileges are needed — the
few reads that want them are skipped without.

## What it reads

Beyond the CPU / memory / disk / GPU / battery inventory above, collecting
`machine.identity` reads:

- **Windows** — the `MachineGuid` value under
  `HKLM\SOFTWARE\Microsoft\Cryptography` (read-only), and the raw SMBIOS
  firmware table via `GetSystemFirmwareTable`. No COM, no WMI, no `wmic`
  subprocess, and no child process of any kind.
- **Linux** — `/etc/machine-id` (or `/var/lib/dbus/machine-id`) and
  `/sys/class/dmi/id/`. The UUID and serial there are root-only by default, so
  an unprivileged run simply doesn't get them.

Collecting `machine.firmware` reads the same SMBIOS table on Windows, and the
world-readable `sys_vendor`, `product_name`, `product_version`, `bios_version`
and `bios_date` files under `/sys/class/dmi/id/` on Linux.

Collecting `machine.memory` takes the Memory Device entries (SMBIOS type 17),
and the Physical Memory Array entries (type 16) that say which of them are
system RAM, from the same raw SMBIOS table: on Windows through
`GetSystemFirmwareTable` as above, and on Linux from
`/sys/firmware/dmi/tables/DMI`, which is root-only by default, so an
unprivileged run simply doesn't get it. Each module's serial number and asset
tag sit in those entries and are **not read**; nor are its manufacturer and
part number. No `dmidecode` or other subprocess is run.

Collecting `machine.os_detail` reads:

- **Windows** — exactly four values under
  `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` (read-only):
  `CurrentBuildNumber`, `UBR`, `DisplayVersion` and `EditionID`. Nothing
  else in that key is read — not the product ID, the registered owner or
  organisation, or the install date.
- **Linux** — the `ID`, `VERSION_ID` and `VARIANT_ID` lines of
  `/etc/os-release` (or `/usr/lib/os-release`), which is world-readable.

Collecting `machine.drives` asks each drive for a read-only health log — the
SMART / Health Information page from an NVMe controller, the Device Statistics
log from a SATA drive:

- **Windows** — `IOCTL_STORAGE_QUERY_PROPERTY` on `\\.\PhysicalDriveN`,
  opened with no read or write access.
- **Linux** — the `NVME_IOCTL_ADMIN_CMD` ioctl on `/dev/nvmeN`, and an ATA
  PASS-THROUGH `READ LOG EXT` via `SG_IO` on `/dev/sdX` (removable devices
  are skipped). Both nodes are root-only by default, so an unprivileged run
  simply doesn't get them.

Only with `--collect` (see [below](#collected-only-when-asked-for)) —
**opt-in**, never read otherwise:

- **`uptime`** — *Windows* `GetTickCount64`; *Linux* `/proc/uptime` (or the
  `sysinfo(2)` call). Both through the `sysinfo` crate.
- **`elevation`** — *Windows* `GetTokenInformation(TokenElevation)` on this
  process's own token, opened for `TOKEN_QUERY`; *Unix* `geteuid()`.
- **`security`** — *Windows*: `GetFirmwareType`; the single value
  `UEFISecureBootEnabled` under
  `HKLM\SYSTEM\CurrentControlSet\Control\SecureBoot\State` (read-only);
  and `Tbsi_GetDeviceInfo` from `tbs.dll`, which is loaded from System32
  only when `security` is asked for, so a run that doesn't ask never maps
  it. *Linux*: whether `/sys/firmware/efi` exists; the `SecureBoot` variable
  in efivarfs
  (`/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c`,
  world-readable); `/sys/class/tpm/tpm0/tpm_version_major` and whether
  `/sys/class/tpmrm/tpmrm0` exists; `/proc/self/mountinfo` for the device
  under `/`, then `/sys/dev/block/<major:minor>/dm/uuid` and `slaves/` down
  the device chain.

All of these are ordinary reads of local system information, and all of them
fail quietly.

## Collected only when asked for

Everything above is hardware and OS inventory, collected on every run. Three
more things are **off unless you name them** — `run --collect
uptime,elevation,security` (or `info --collect …` to see them first, or
`soak --collect …`), or `collect = [...]` in a `--config` file, which `info` and
`soak` read as well — because each says something about
one particular machine or the person using it, which the default inventory
deliberately doesn't:

- **`uptime`** adds `machine.boot`: when the machine last booted and how long
  it has been up. That says something about how a person uses the machine —
  when they start work, whether they ever restart.
- **`elevation`** adds `run.elevated`: whether this run had administrator or
  root rights. That is about the invocation, not the hardware, which is why
  it sits in the `run` block.
- **`security`** adds `machine.security`: whether the machine booted through
  UEFI, whether Secure Boot is on, whether there is a TPM and which version,
  and (Linux only) whether the root filesystem is on dm-crypt. A list of the
  machines *without* Secure Boot or disk encryption is useful to an attacker
  if a results share, a collector or a backup leaks.

Unlike the hardware blocks above, these are per-unit: two machines of the same
model report different values. That is why they are opt-in.

Nothing in a category is read, attempted or logged unless its name was given —
the code that reads it does not run. What was asked for is written into the
result as `run.collect`, so a reader can tell "not asked" from "could not be
read". A `soak` document records it in a `run` block holding only that and `run.elevated`. Each part is absent where it couldn't be read; nothing is guessed.

**Deliberately not read:** the TPM's endorsement key, manufacturer, firmware
revision or anything else that identifies it; the device-mapper UUID of an
encrypted volume (it is read, to see its type prefix, and neither kept nor
logged); any volume but the root filesystem; BitLocker recovery or key
protector information.

**Not read, because it can't be within the promises above:**

- **BitLocker state on Windows.** Every documented way to read it —
  `Win32_EncryptableVolume` (WMI), `manage-bde` and `Get-BitLockerVolume`
  (child processes) — needs administrator rights, and the shell's
  `System.Volume.BitLockerProtection` property is COM. What a standard user
  can read is undocumented and has no stated meaning, so there is no
  `disk_encryption` part on Windows.
- **Other kinds of Linux encryption** — ZFS native encryption, fscrypt,
  eCryptfs, self-encrypting drives — aren't looked for: "no dm-crypt under
  `/`" is reported as `"none_detected"`, never as "not encrypted".

## Network

- The `network` benchmark is **loopback only** (`127.0.0.1`) — nothing leaves
  the machine.
- An **outbound** connection is made only when you pass `--net-target HOST:PORT`
  (a deliberate two-machine link test) or `--upload URL`, and a **listening**
  socket is opened only when you run `loadbearer net-server`.
- `--upload` sends **the result file** — the same document `--output`
  writes, inventory and `machine.identity` included — in one HTTPS `POST` to
  the URL you give, when the run finishes. It carries the ingest token you
  supply, read from a file or `LOADBEARER_UPLOAD_TOKEN`, as an
  `Authorization` header, and a `User-Agent` naming the loadbearer version.
  Plain `http://` is refused except to this machine, and redirects are not
  followed. The token is never logged or written into the result. The one
  other thing `--upload` can send is the run status record, next.
- **A run status record, when a run is skipped and `--upload` is set.** If
  `--not-on-battery` or `--if-idle` stops a run and an upload target is
  configured, loadbearer sends one small JSON document to that same URL, with
  the same token and under the same transport rules, so the fleet can tell a
  machine that keeps being skipped from one that has stopped reporting. Its
  fields are exactly: `schema` (`loadbearer.run-status/1`), `tool_version`,
  `timestamp`, `status` (`skipped`), `reason` (`on_battery` or `busy`),
  `detail` (the sentence the skip prints on stderr), `hostname`,
  `identity` and `tags`. `identity` is the same `machine.identity` block a
  result carries (machine ID, SMBIOS UUID, serial, asset tag, whichever could
  be read) and `tags` are your `--tag` values, so the server can file the
  record under the same machine and apply a site-scoped token. There is **no
  other inventory**: no CPU, memory, disks, firmware, power state, and nothing
  from `--collect`. The record is sent **only** with an upload target, **only**
  for a battery or busy skip, and **never** for `--skip-if-newer-than`. It is
  sent once, with a ten-second limit and no retry; if it cannot be delivered
  (the network is down, or an older server answers HTTP 400 because it does not
  know the document) that is one warning line on stderr and in the diagnostic
  log, and the run still exits 0. `--no-status`, or `status = false` under
  `[upload]` in the config file, turns it off. The identity is read only when
  a record is about to be sent. The record is not written to `--output` or to
  stdout, and the diagnostic log says only whether a skip was reported and, if
  not, why — never the identity values. **Without `--upload`, a skipped run
  still reads nothing, writes nothing and sends nothing.**
- The GPU probe and benchmark load the OS OpenCL driver locally; no network.

## The website

`loadbearer.issinoho.com` is a static page hosted on GitHub Pages. Its only
dynamic behaviour is a request from your browser to GitHub's public API to show
the latest release tag — governed by
[GitHub's Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
The site sets no cookies and runs no analytics.

## Contact

Questions: open an issue, or email **iain@issinoho.com**.
