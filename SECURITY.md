# Security Policy

## Supported versions

loadbearer has no maintained release branches — only the **latest
release** is supported. Please upgrade before reporting anything; there's
a good chance it's already fixed.

## Reporting a vulnerability

Please **don't** open a public issue for a security report.

The preferred way is GitHub's own
[private vulnerability reporting](https://github.com/issinoho/loadbearer/security/advisories/new):
open the [Security tab](https://github.com/issinoho/loadbearer/security)
and click "Report a vulnerability". This reaches only the maintainer,
lets you attach details and reproduction steps privately, and keeps the
conversation off the public tracker until there's a fix.

If you'd rather not use that, email **iain@issinoho.com** instead.

This is a solo-maintained project, so there's no formal SLA — but a
genuine security report gets priority over everything else. Expect an
initial response within a few days.

## Attack surface

loadbearer is a local command-line tool. It:

- runs benchmark workloads in-process (no plugins, no code download);
- writes a scratch file of a few GiB to `--target-dir` (default: the
  working directory) for the disk benchmark, and deletes it when the run
  ends — a killed run may leave `.loadbearer-scratch.<pid>` behind;
- binds **loopback** TCP/UDP sockets for the network benchmark; nothing
  leaves the machine;
- reads `/sys/devices/system/cpu/…` (Linux, for core-affinity and
  filesystem detection) and the target directory's filesystem type;
- makes an **outbound** connection only when you pass `--net-target`, and
  opens a **listening** socket only when you run `loadbearer net-server`.

It stores no credentials or secrets and needs no elevated privileges.

## Before you report: known, by-design behavior

- **`loadbearer net-server` binds `0.0.0.0:47913` by default and has no
  authentication.** It is a bare TCP data sink / TCP echo / UDP sink,
  intended to be run briefly on a trusted network for a `--net-target`
  link test and then stopped. Pass `--bind 127.0.0.1:<port>` (or a
  specific interface) to restrict it, and don't leave it running.
- **Result JSON files embed a machine inventory** — hostname, CPU model,
  OS version, disk models / mount points / capacities. That's deliberate
  (it's what makes `compare` and `baseline` meaningful), but treat a
  result file like any other machine fingerprint before posting it
  publicly.
- **The disk benchmark can fill `--target-dir`.** It surfaces the OS
  "no space left" error rather than pre-checking free space.

If you're unsure whether something is a genuine vulnerability or one of
the above, report it anyway — that's a reasonable thing to ask.

## Other measures already in place

- [Dependabot](https://github.com/issinoho/loadbearer/security/dependabot)
  is configured for both GitHub Actions and Cargo dependencies (see
  `.github/dependabot.yml`).
- Secret scanning and push protection are enabled on this repository.
