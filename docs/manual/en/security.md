# Security

MRTG treats its configuration as code and trusts what devices send it. LinkScope keeps the same
configuration format but closes those gaps. This page explains how, and how to run LinkScope
safely.

## The configuration is data

In MRTG, `Target[]` is run as Perl, `SnmpOptions` and `ConversionCode` are evaluated, and
backtick targets run through `/bin/sh`. Anyone who can write the configuration, or any file it
includes, can run code as the MRTG account.

In LinkScope:

- Target expressions compile to an arithmetic tree. Nothing is run as code.
- `SnmpOptions` is parsed as data.
- Conversion code is not supported.
- External commands run without a shell, unless you set `ExecShell: yes`.
- Configuration files that other accounts can change are refused (see below).

`ExecShell: yes` only decides whether LinkScope itself reads shell syntax. It is not a sandbox.
Anyone who can edit the configuration can still run programs as the service account (external
commands, threshold programs). That is why the files themselves are checked.

### Trusted configuration files

Every configuration file, including each included file, must be owned by root or by the account
that runs LinkScope, and not writable by group or others. Every directory on the way to it must
belong to root or that account and must not let anyone else rename its entries: not writable by
others, nor by a group other than root's or the account's own, unless it is sticky. LinkScope
checks the file it actually opened, not just the name.
`--allow-insecure-config` loads such a file with a warning instead.

## Credentials stay out of every output

The configuration holds SNMP communities, SNMPv3 passwords, a webhook URL that may carry a token,
the webhook's signing secret and the mail password. None of them leave LinkScope:

- Every string the JSON API returns (errors, warnings, Target text) passes through one step that
  removes all configured communities, SNMPv3 passwords and external command lines. Runtime
  errors name the target, not its Target text.
- Credentials are left out when settings are serialized.
- The webhook URL, its secret and the mail password never appear in the UI, the API, `/metrics`,
  webhooks or error messages.
- The threshold programs' paths and the threshold mail address are not in the API either: it says
  only whether a target has them.
- Threshold mail, a threshold program's arguments and its `THRESH_DESC` have every configured
  secret replaced with `•••`, as the webhook body has, when a title, description or `PageTop[]`
  quotes one.
- The webhook client ignores proxy settings in its environment, so no proxy sees the signed body.
- LinkScope warns at startup when alerts would travel in clear: threshold mail as plain SMTP to
  another host, or an `http://` webhook to another host.
- A test plants secrets in a configuration and searches every endpoint's output for them.

## The web UI

- The UI shows device data (sysName, uptime, router names) as text, so a device cannot inject
  HTML or scripts.
- HTML from the configuration (`PageTop`, `Title`, legends) is reduced to a list of allowed
  tags, without scripts, styles or event handlers.
- Every response carries a strict Content-Security-Policy (scripts only from LinkScope itself,
  no framing, no inline scripts), `X-Content-Type-Options: nosniff`, `Referrer-Policy:
  no-referrer` and `X-Frame-Options: DENY`.
- The UI is read-only and has **no login**. There is no web listener until you set `WebListen`
  or `--listen`. Bind it to loopback or a trusted management network, and put a reverse proxy
  with authentication in front of it before you expose it further.
- It answers only to IP addresses, `localhost` and the names in `WebHosts:`, so a web page that
  rebinds its own name to the UI's address (DNS rebinding) cannot read it from a browser.
- It serves at most 256 connections at once (more wait in the kernel's queue and hold none of the
  daemon's file descriptors, which the poller needs), a request's headers must arrive within 10
  seconds, and a connection is closed after 120 seconds. A client holding connections open
  cannot stop the polling. Logs are summarized for graphs a few at a time.

## Devices are not trusted

### cfgmaker

MRTG's cfgmaker wrote sysName into `Title[]` as it came, and newlines or backticks in device
strings ended up in the generated configuration. LinkScope's cfgmaker:

- removes control characters from every device string;
- falls back to the ifIndex for interface references that cannot be written safely;
- escapes values that end up in HTML, sysName included.

A test feeds hostile strings through cfgmaker and parses the result.

### SNMP replies

Replies are accepted only from the address that was polled (a connected UDP socket), and only
with the right request id and community. SNMPv3 authPriv (SHA-1 or SHA-2 with AES or DES) is
supported and recommended.

### Hostile or slow agents

- Each agent has a time budget per cycle (80 % of `Interval`).
- Table walks are limited by time, rows (100,000) and bytes, each row counting what it costs.
  The interface cache keeps at most 8 MB per device, and the `.ok` file stays as small.
- A GET answer is used only for the object it names, so a stray or stale reply cannot put one
  interface's counter on another.
- After a request that got no answer, the next one goes out from a new socket, so a late reply
  to the old request cannot be read as the new one's.
- Device strings (sysName, uptime) are kept to 256 characters.
- Numbers longer than 64 characters are rejected, and so is any value wider than 190 digits
  (MRTG's limit), judged before it is expanded: `1e999999999` is no number.
- An SNMPv3 session whose replies are refused, or whose authentication fails, is opened again
  with a new socket and a fresh discovery, so a forged reply cannot spoil it for good.
- SNMPv3 settings that ask for less than they seem to (a privacy password without an
  authentication password, a protocol without its password, an empty password) are an error,
  never a quieter security level.
- A stuck agent delays the cycle by at most its budget and cannot block the others.
- A failure in one target is contained to that target.

## Files

MRTG followed symlinks as root (when changing directory owners with `--fhs`, and for the pid
file before a later fix). LinkScope:

- opens directories with `O_DIRECTORY|O_NOFOLLOW` and changes their owner with `fchown`;
- takes its lock before it creates the pid file with `O_EXCL|O_NOFOLLOW`, and never reads a pid
  file left behind (the lock tells a running instance apart; a stale file is unlinked);
- writes every output file (logs, cache, threshold state) to a new temporary file with an
  unpredictable name, created `O_EXCL|O_NOFOLLOW`, then renames it into place;
- refuses `Directory[]` subdirectories that are symlinks, creates them one level at a time, and
  reads and writes a target's logs through the directory it opened (so a component swapped for a
  symlink after the check cannot redirect them);
- reads logs with `O_NOFOLLOW|O_NONBLOCK`, only if they are regular files, and at most 4 MB, so a
  FIFO or a device in place of a log cannot hang or exhaust the daemon; at most 16 times a full
  log's lines are kept, so a log of blank lines cannot take hundreds of MB;
- reads configuration files without blocking, regular files only, at most 1,000 files and 64 MB
  for one configuration with its includes.

The `.ok` cache and the `.alerts` state are written owner only (mode 600), because the cache, like
MRTG's, names each device by its SNMP community; a file left readable by others is written again
owner only. Both are read like the logs (no symlink, no FIFO, a size cap).

`WorkDir` itself is trusted: keep it writable only by the service account.

### Paths

Target names may not contain path separators. `Directory[]` is reduced to plain relative parts
(a leading `/` is dropped, as MRTG's string joining treats it, and `..` is rejected), so every log
stays under the log directory.

## Privileges and locking

- With `--user`, LinkScope calls `setgroups` with only the target group, then `setgid` and
  `setuid`, and checks both ids and the group list. MRTG kept the supplementary groups and only
  checked the user id.
- LinkScope holds an `flock` for its whole life, so the lock is never lost in daemon mode.
- There are no per-target processes: the rate engine runs inside LinkScope, so a failed fork
  cannot replace the daemon.

## Memory safety

LinkScope is written in Rust, with no `unsafe` code in the configuration parser, the expression
evaluator or the rate engine (the rate engine forbids it). Values that are too long or malformed
are rejected instead of overflowing a buffer. The rate engine is tested against 2,000 randomly
corrupted logs. SNMP packets are decoded by the `snmp2` library, which this project does not fuzz.

## Running LinkScope safely

- **Run as an unprivileged account**, with `--user mrtg` or with systemd's `User=`. With
  `--user`, LinkScope reads the configuration, takes the lock and creates the pid file before it
  drops privileges, then creates the log directories as that account.
- **Keep `mrtg.cfg` private.** It holds SNMP credentials. Use `chown root:mrtg` and `chmod 640`,
  so the daemon can read it again on reload after dropping privileges. (`chmod 600` owned by root
  also works, but then a reload fails and you restart instead.)
- **Keep `WorkDir` writable only by the service account.**
- **Prefer SNMPv3 with authPriv**, and let agents answer only the poller's address.
- **Keep the web listener on a trusted network**, behind a reverse proxy with authentication if
  anyone else can reach it.
