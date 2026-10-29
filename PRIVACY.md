# Privacy

**loadbearer collects no data and sends nothing anywhere.**

It is a local command-line tool. It has no telemetry, no analytics, no crash
reporting, no update check, and no account or licence system. There is no
server side.

## What it writes, and where — all local

- **Result files** (`--output result.json`, or `--json` to stdout). These
  contain a machine inventory — hostname, CPU model/vendor, OS and kernel
  version, RAM, disk models / mount points / capacities, and the GPU and
  battery details where present — alongside the benchmark numbers. That is
  deliberate: it is what makes `loadbearer compare` and `loadbearer baseline`
  meaningful.

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
  have chosen). None of these is per-unit: every machine of a model reports the same model and
  firmware strings, and no drive serial number is read.

  Anything a `--tag` puts in the file is text you supplied, so don't put
  personal data in one.

  Treat a result file like any other machine fingerprint before you share it.
  loadbearer never uploads one; you choose what to do with it.
- **A diagnostic log** at `%LOCALAPPDATA%\loadbearer\loadbearer.log` (Windows) /
  `$XDG_CACHE_HOME/loadbearer/loadbearer.log` (Linux), or wherever `--log-file`
  points. It records the run's settings, per-benchmark timings, and errors. It
  contains no personal data beyond the local file path in its header —
  deliberately not the hostname, and not the `machine.identity` values. At
  `--log-level debug` it records whether each of those was *readable*, not what
  it said, because a log is the artefact people paste into a bug report while a
  result file is one they choose to share. The exception is an identifier
  rejected as an OEM placeholder ("To Be Filled By O.E.M." and friends), which
  is logged as-is: it describes the firmware, not the machine. `--no-log`
  disables the log entirely.
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

Collecting `machine.drives` asks each drive for a read-only health log — the
SMART / Health Information page from an NVMe controller, the Device Statistics
log from a SATA drive:

- **Windows** — `IOCTL_STORAGE_QUERY_PROPERTY` on `\\.\PhysicalDriveN`,
  opened with no read or write access.
- **Linux** — the `NVME_IOCTL_ADMIN_CMD` ioctl on `/dev/nvmeN`, and an ATA
  PASS-THROUGH `READ LOG EXT` via `SG_IO` on `/dev/sdX` (removable devices
  are skipped). Both nodes are root-only by default, so an unprivileged run
  simply doesn't get them.

All of these are ordinary reads of local system information, and all of them
fail quietly.

## Network

- The `network` benchmark is **loopback only** (`127.0.0.1`) — nothing leaves
  the machine.
- An **outbound** connection is made only when you pass `--net-target HOST:PORT`
  (a deliberate two-machine link test), and a **listening** socket is opened
  only when you run `loadbearer net-server`.
- The GPU probe and benchmark load the OS OpenCL driver locally; no network.

## The website

`loadbearer.issinoho.com` is a static page hosted on GitHub Pages. Its only
dynamic behaviour is a request from your browser to GitHub's public API to show
the latest release tag — governed by
[GitHub's Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
The site sets no cookies and runs no analytics.

## Contact

Questions: open an issue, or email **iain@issinoho.com**.
