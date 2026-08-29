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
  meaningful. Treat a result file like any other machine fingerprint before you
  share it. loadbearer never uploads one; you choose what to do with it.
- **A diagnostic log** at `%LOCALAPPDATA%\loadbearer\loadbearer.log` (Windows) /
  `$XDG_CACHE_HOME/loadbearer/loadbearer.log` (Linux), or wherever `--log-file`
  points. It records the run's settings, per-benchmark timings, and errors. It
  contains no personal data beyond the local file path in its header. `--no-log`
  disables it.
- **A scratch file** in `--target-dir` for the disk benchmark, deleted when the
  run ends.

Nothing is written to the registry. No elevated privileges are needed.

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
