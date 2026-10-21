# Changelog

LinkScope follows [semantic versioning](https://semver.org/); before 1.0, a minor version may change
behavior, and every such change is listed under "Upgrading".

## 0.2.0 (2026-10-21)

The first public release. It adds metrics, alerts that reach people, a manual and a Docker image to
0.1.0, and fixes every finding of a security assessment made before the source was published.
MRTG compatibility holds: an existing `mrtg.cfg` keeps working, and normal `.log` files are written
exactly as MRTG 2.17.10 writes them.

### Upgrading from 0.1.0

Check these before you replace 0.1.0:

- **Opening the UI by a host name needs `WebHosts:`.** The UI answers IP addresses and `localhost`;
  a request for any other name gets `421`. List the names you use (behind a reverse proxy too):
  `WebHosts: noc.example.com`. `WebHosts: *` answers any name.
- **The configuration's directories are checked too.** Besides the file, every directory on the
  way to it must belong to root or LinkScope's account and must not let another account rename its
  entries (not writable by others, or by a group other than root's or the account's own, unless
  sticky). `--allow-insecure-config` still turns this into a warning.
- **SNMPv3 settings must be complete.** A privacy password without an authentication password, a
  protocol without its password, or a password shorter than 8 characters is an error (it used to
  poll with less security than it seemed to ask for).
- **A bare `ThreshProg*[]` name** is the file in the working directory LinkScope was started from,
  as checked when the configuration is loaded, not one found on `PATH`. Give a full path.
- **Processes an EXEC target leaves running** are ended after it exits (its process group).
  A helper meant to outlive it must start its own session (`setsid`).
- **The JSON API's summary** gives `config_name` (the configuration's file name) instead of
  `config_path`, and a target's `thresholds` say whether a program or a mail address is set
  (`programs`, `mail`) instead of naming them.
- **A PID file of another live process** no longer blocks startup; the lock decides.
- **Relative `Include:` names** resolve against the main configuration's directory only.
- **The cache and alert state** (`.ok`, `.alerts`) are written owner only (mode 600); a file
  readable by others is written again at the next save.
- **Building from source** needs Rust 1.88 or newer.

### Added

- **Prometheus metrics** on the web listener (`GET /metrics`): each target's rates, its counters as
  last read, `MaxBytes`, status and last poll, and each cycle's duration and outcome.
- **Alerts that reach people**, all off until set: `ThreshFor[]` (a bound broken for several polls
  before it fires), a signed webhook (`AlertWebhook:`, `AlertWebhookSecret:`, a delivery ID kept
  across retries), threshold mail over STARTTLS with a login (`AlertMailUser:`,
  `AlertMailPassword:`), a resolved notice when a bound clears, and the alert state in the UI and
  the API. MRTG's `ThreshProg*`, `ThreshMail*` and `ThreshDir` behave as before.
- **The manual**, served by LinkScope at `/docs`, in English and Bahasa: install and run, moving
  from MRTG, the command line, configuration, cfgmaker, the web UI, metrics, alerts, the JSON API,
  security, compatibility and troubleshooting.
- **Docker**: an image for amd64 and arm64 (`ghcr.io/rinjanianalytics/linkscope`), a compose file
  and optional `.env` settings. The container runs unprivileged on a read-only filesystem.
- **`WebHosts:`**, the host names the UI answers to.
- SNMPv3 AES-192 and AES-256 (`aes192`, `aes256` as net-snmp extends keys; `aes192c`, `aes256c` as
  Cisco does).

### Security

An assessment of LinkScope itself, by attacker position, before the source was published.
`docs/SECURITY.md` lists every finding and what changed. The main ones:

- Anyone who reached the UI could stop all polling by holding connections open until every file
  descriptor was used. The listener now caps connections (others wait in the kernel's queue) and
  times out slow requests.
- A polled device could make the daemon allocate gigabytes, or abort it, with a value written with
  a huge exponent. Values are judged by width before anything expands them.
- One forged datagram could break an SNMPv3 session until a restart (the SNMP library changed
  session state before authenticating a reply). The library is now `async-snmp`, which
  authenticates first, and replies are accepted only from the polled address.
- SNMP communities were readable by every local user in the `.ok` cache.
- No Host check on a UI without a login (DNS rebinding).
- A configuration character across byte 7 crashed the reader, and a reload with it killed the
  daemon.
- `rustls-webpki` RUSTSEC-2026-0382; the UI's build tools moved to Vite 8 and Tailwind 4
  (`npm audit` reports nothing).
- Many lower ones: caps on walks, caches, device strings and logs, owner-only state files, hardened
  reads, alert notices that are no longer dropped, secrets redacted from threshold mail, and more.

### Fixed

- The UI no longer goes blank after leaving a target page in newer Chrome.
- The README's cfgmaker examples set the SNMP version (the fifth field), not the backoff.

## 0.1.0

A memory-safe, drop-in reimplementation of MRTG 2.17.10 in Rust with a built-in web UI: MRTG's
configuration, `.log` files byte-identical to MRTG's, its command line and exit codes, cfgmaker,
SNMP v1, v2c and v3, concurrent polling with the rate engine in process.
