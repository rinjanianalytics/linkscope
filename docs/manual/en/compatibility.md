# Compatibility with MRTG

LinkScope reads MRTG 2.17.10 configuration files and reads and writes MRTG's `.log` files. It can
replace `mrtg` in place: the same configuration, the same log directory, the same history. This
page lists what is identical, what differs on purpose, and what is not supported.

## What is identical

| Area | How it was checked |
|---|---|
| The configuration grammar: defaults `[_]`, prefixes `[^]`, suffixes `[$]`, `NoSpaceChar`, continuation lines, `Include:` | Compared with MRTG's own Perl on a hand-written set of files, 1,500 random configurations and the 26 configurations in MRTG's `contrib/` |
| Validation and defaults (MRTG's `cfgcheck`) and Target parsing (`targparser`) | The same comparison: identical keyword maps, data sources, rewritten Target expressions, and the same configurations rejected |
| Rate math and the log format (`rateup`) | Byte-identical `.log` and `.old` files over 50,000 random updates against the C program, and 2,000 corrupted logs |
| The whole pipeline: polling, interface lookup by name, description, IP, MAC and type, arithmetic, gauge, perminute, AbsMax, unknown values, 32-bit wraps, 64-bit counters, external commands | MRTG and LinkScope run against the same agent at the same (faked) time: 18 targets, 6 cycles, every `.log` byte-identical (`testenv/parity.sh`) |
| cfgmaker's output | Identical to MRTG's cfgmaker for a version 2c device, apart from the header |

MRTG's quirks are kept where they change data. For example:

- a continuation line after a global keyword is ignored (LinkScope warns about it);
- a multi-line `[^]` prefix that ends in `NoSpaceChar` keeps only its last line;
- `ThreshHyst` without `ThreshDir` fires only inside the hysteresis band.

## What differs on purpose

### Features that needed Perl

- **Target expressions** are compiled, not run as Perl. They support data sources, numbers
  (decimal, hex, octal, `_` separators), `+ - * / % **`, parentheses, unary minus, `int()` and
  `abs()`, with Perl's precedence and arbitrary precision (like `Math::BigFloat`). Anything else is
  an error when the file is loaded, not a failure while polling.
- **`SnmpOptions`** must be plain `key => value` pairs (numbers or quoted strings). MRTG ran it
  as Perl.
- **Conversion subroutines** (`|name` in a Target, `ConversionCode:`) are not supported. A Target
  that uses one is reported when the file is loaded.
- **`LibAdd:`** is ignored, with a warning.
- **cfgmaker's `--if-filter`** takes a safe expression language (variables, comparisons,
  `=~ /re/i`, `&& || !`) that covers MRTG's documented examples. `--if-template` and
  `--host-template` run Perl and are refused. See [cfgmaker](cfgmaker.md#interface-filters).

### External commands (backtick targets)

- They run **directly, without a shell**. A command that needs a shell (pipes, redirection,
  `$VAR`, globs) is an error when the file is loaded, unless you set `ExecShell: yes`.
- They time out after 120 s, and their output is capped at 64 KB.

### Output

- **No HTML pages or PNG files are written.** The built-in [web UI](web-ui.md) replaces the
  per-target pages and `indexmaker`. `HtmlDir`, `ImageDir` and the page keywords are still
  accepted; HTML in `PageTop`, `PageFoot`, `Title` and the legends is shown after sanitizing.
  `AddHead` and `BodyTag` are not used.
- `LogFormat: rrdtool`, `RRDCached`, `SendToGraphite` and the Holt-Winters thresholds
  (`HWThresh*`) are not supported.

### Security

- Configuration files, and the files they include, must be owned by root or by the running
  account and not writable by group or others. `--allow-insecure-config` turns this into a
  warning.
- SNMP replies are accepted only from the address that was polled. MRTG matched replies
  leniently by default.
- Target names and `Directory[]` may not contain `..` or path separators that leave the work
  directory.
- Directories are created with mode 0755 (MRTG used 0777 minus the umask).

See [Security](security.md) for the full model.

### Bugs fixed

- **The daemon keeps its lock.** MRTG's daemonizing parent deleted the lock files the child
  relied on. LinkScope holds an `flock` for the life of the process.
- **A reload no longer duplicates targets.** MRTG kept adding to its target list on every reload.
- **The interface cache (`.ok`) keeps every host.** MRTG dropped hosts that were not scanned again
  in a run whenever any host was scanned, and its daemon never wrote the cache.
- **`Include:` with a glob reads files in sorted order** (MRTG used reverse order), and include
  cycles are detected.
- **TimeTicks values used as data are numbers.** MRTG turned them into text such as
  "3 days, 1:02:03" and then rejected them.
- **cfgmaker:** a stale error from a vendor-specific walk no longer disables the first interface
  of SNMPv1 Cisco-like devices.
- **cfgmaker:** device strings (sysName, ifAlias and the rest) can no longer add configuration
  lines or `Target[]` commands.

### Limits that keep LinkScope responsive

- Each agent may take at most 80 % of `Interval` per cycle. After that, its targets are unknown
  for the cycle and the others carry on.
- External commands: only the first four lines are read (as in MRTG), then the command's process
  group is stopped; the timeout is 120 s. The group is also stopped when the command finishes, so
  something it started in the background does not keep running (MRTG left it). `PathAdd` with
  no `PATH` set gives that directory alone, never the current directory.
- Threshold programs and mails run in the background, eight at a time.
- Values longer than 64 characters from agents or commands are rejected.
- A log that cannot be written is reported as an error (in the UI and the log). As in MRTG, it
  does not change the exit code, which only says whether values were collected.

### Other differences

- `--daemon` detaches like MRTG. `--foreground` (or `NoDetach: yes`) keeps it attached, for
  systemd.
- New options: `--listen`, `--concurrency`, `--foreground`, `--allow-insecure-config`. New global
  keywords: `WebListen`, `ExecShell`, `AlertWebhook`, `AlertWebhookSecret`, `AlertMailUser`,
  `AlertMailPassword`. New per-target keyword: `ThreshFor[]`. MRTG rejects these keywords, so keep
  them out of configurations you still share with MRTG.
- `ThreshMailServer` also takes `host:port`.
- Thresholds of a target without `ThreshFor[]` run their programs and mails exactly as MRTG does.
  With `ThreshFor[]`, they run once when an alert fires and once when it clears. Either way, the
  alert state is also kept in `<config>.alerts`, for the UI and `AlertWebhook`. See
  [Alerts](alerts.md).
- `--log-only` is accepted and does nothing (LinkScope writes nothing but logs).
- `--check` does not take the lock, so it works while the daemon runs.
- `Directory[x]: /routers/` means `WorkDir/routers/`, as MRTG's string joining treats it.
- After `SIGHUP`, the daemon reloads and keeps its polling schedule. `SIGTERM` stops a cycle in
  progress. A reload does not apply changes to `WebListen`, `LoadMIBs` or `RunAsDaemon` (restart
  for those), and it needs the configuration to be readable by the service account.
- Squid cache targets (`1.3.6.1.4.1.3495.1`): MRTG read `cacheUptime` and `cacheSoftware` for the
  uptime line; LinkScope reads `sysUptime` and `sysName`. Graph data is the same.
- If a `RouterUptime[]` read fails, the uptime and name from the data source are kept (MRTG
  blanked them).

## Not supported

- Perl features: conversion subroutines, `ConversionCode`, cfgmaker's `--if-template` and
  `--host-template`.
- RRDtool: `LogFormat: rrdtool`, `RRDCached`, the `derive` option, `HWThresh*`.
- `SendToGraphite`.
- HTML and PNG output, and `indexmaker`.
- In cfgmaker: Cisco VLAN columns (`$if_vlan_id`, `$if_cisco_trunk` in filters) and the
  `catname` and `ppname` descriptions.
- SNMPv3 localized keys (`authkey`, `privkey`). Use `authpassword` and `privpassword`.
