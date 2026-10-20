# Compatibility with MRTG 2.17.10

LinkScope reads MRTG configuration files and reads and writes MRTG's `.log`
files, so it can replace `mrtg` in place: same config, same log directory,
same history. This page lists everything that behaves differently.

## Verified to behave identically

| Area | How it was verified |
|---|---|
| Config grammar (`readcfg`): defaults `[_]`, prefixes `[^]`, suffixes `[$]`, `NoSpaceChar`, continuation lines, `Include:` | Differential test against MRTG's own Perl on a hand-written corpus, 1,500 random configs and the 26 configs in MRTG's `contrib/` (`crates/config/tests/perl_parity.rs`) |
| Validation and defaults (`cfgcheck`), Target parsing (`targparser`) | Same test: identical keyword maps, source tables, rewritten Target expressions and unique-source flags, and the same configs rejected |
| Rate math and log format (`rateup`) | Byte-identical `.log`/`.old` files over 50,000 randomized updates against the C binary, plus 2,000 corrupted logs (`crates/rate/tests`) |
| Whole pipeline (polling, interface lookup by name/descr/IP/MAC/type, arithmetic, gauge, perminute, AbsMax, unknown values, 32-bit wraps, 64-bit counters, EXEC targets) | `testenv/parity.sh` runs original MRTG and LinkScope against the same agent at the same (faked) time: 18 targets, 6 cycles, every `.log` byte-identical |
| cfgmaker output | Identical to MRTG's cfgmaker for a v2c device (header aside) |

MRTG quirks are reproduced on purpose where they affect data, for example:
continuation lines after a *global* keyword are ignored (with a warning);
a multi-line `[^]` prefix ending in `NoSpaceChar` keeps only its last line;
`ThreshHyst` without `ThreshDir` fires only inside the hysteresis band.

## Deliberate differences

### Things that needed Perl
* **Target expressions** are compiled, not `eval`'d. Supported: data
  sources, numbers (decimal, hex, octal, `_` separators), `+ - * / % **`,
  parentheses, unary minus, `int()` and `abs()`, with Perl precedence and
  arbitrary precision (like `Math::BigFloat`). Anything else is a config
  error at load time instead of a runtime failure.
* **`SnmpOptions`** must be plain `key => value` pairs (numbers or quoted
  strings). It was evaluated as Perl.
* **Conversion subroutines** (`|name` in a Target, `ConversionCode:`) are not
  supported; a Target that uses one is reported at load time.
* **`LibAdd:`** is ignored (warning).
* **cfgmaker `--if-filter`** takes a safe expression language (variables,
  comparisons, `=~ /re/i`, `&& || !`), which covers the documented examples.
  `--if-template` and `--host-template` run Perl and are refused.

### External commands (backtick targets)
* Run **directly, without a shell**. A command that needs shell features
  (pipes, redirection, `$VAR`, globs) is rejected at load time unless the
  new global keyword `ExecShell: yes` is set.
* Commands time out after 120 s and their output is capped at 64 KB.

### Output
* No HTML pages or PNG files are written. The built-in web UI
  (`--listen 127.0.0.1:8080` or `WebListen:`) replaces the per-target pages
  and `indexmaker`. `HtmlDir`, `ImageDir` and the page keywords are still
  accepted; PageTop/PageFoot/Title/Legend HTML is shown after sanitizing.
  `AddHead` and `BodyTag` are not used.
* `LogFormat: rrdtool`, `RRDCached`, `SendToGraphite` and the Holt-Winters
  thresholds (`HWThresh*`) are not supported.

### Security-related behaviour
* Config files (and included files) must be owned by root or the running
  user and not writable by group/others. `--allow-insecure-config` downgrades
  this to a warning.
* SNMP replies are accepted only from the address that was polled. MRTG's
  default was lenient source matching.
* Target names and `Directory[]` may not contain `..` or path separators
  that escape the work directory.
* Directories are created with mode 0755 (MRTG: 0777 minus umask).

### Bugs fixed
* The daemon kept its lock: MRTG's daemonizing parent deleted the lock files
  the child relied on. LinkScope uses `flock` for the life of the process.
* Config reload no longer duplicates targets (MRTG kept appending to its
  target array on every reload).
* The interface cache (`.ok`) keeps every host. MRTG dropped hosts that were
  not re-scanned in a run whenever any host was re-scanned, and its daemon
  never wrote the cache.
* `Include:` with a glob reads files in sorted order (MRTG: reverse order)
  and include cycles are detected.
* TimeTicks values used as data are numbers. MRTG turned them into
  "3 days, 1:02:03" and then rejected them.
* cfgmaker: a stale error from a vendor-specific walk no longer disables the
  first interface of SNMPv1 Cisco-like devices.
* cfgmaker: device strings (sysName, ifAlias, ...) can no longer add config
  lines or `Target[]` commands; see [SECURITY.md](SECURITY.md).

### Robustness limits (new)
* Each agent may take at most 80% of Interval per cycle; after that its
  targets are unknown for the cycle and the rest proceed.
* External commands: only the first four lines are read (as in MRTG), then
  the command's process group is stopped; 120 s timeout.
* Threshold programs and mails run in the background, eight at a time.
* Values longer than 64 characters from agents or commands are rejected.
* A log that cannot be written is reported as an error (UI and log), but,
  like MRTG, does not change the exit code, which reflects only whether
  values were collected.

### Other
* `--daemon` detaches like MRTG; `--foreground` (or `NoDetach: yes`) keeps it
  attached for systemd.
* New options: `--listen`, `--concurrency`, `--foreground`,
  `--allow-insecure-config`. New global keywords: `WebListen`, `ExecShell`,
  `AlertWebhook`, `AlertWebhookSecret`, `AlertMailUser`, `AlertMailPassword`;
  new per-target keyword: `ThreshFor[]`. MRTG itself would reject these
  keywords, so keep them out of configs you still share with MRTG.
* `ThreshMailServer` also takes `host:port`.
* Thresholds of a target without `ThreshFor[]` run their programs and mails
  exactly as MRTG does (`crates/linkscope/tests/thresh_mrtg.rs` pins it). With
  `ThreshFor[]`, they run once when an alert fires and once when it clears.
  Either way the alert state is also kept in `<config>.alerts`, for the UI
  and `AlertWebhook`.
* `--log-only` is accepted and does nothing (nothing but logs is written).
* `--check` does not take the lock, so it works while the daemon runs.
* `Directory[x]: /routers/` means `WorkDir/routers/`, as with MRTG's string
  concatenation.
* After SIGHUP the daemon reloads and keeps its polling schedule. SIGTERM
  stops a cycle in progress. Reloading does not apply changes to
  `WebListen`, `LoadMIBs` or `RunAsDaemon` (restart for those), and needs the
  config to be readable by the service user.
* Squid cache targets (`1.3.6.1.4.1.3495.1`): MRTG read `cacheUptime` and
  `cacheSoftware` for the uptime line; LinkScope reads `sysUptime` and
  `sysName`. Graph data is unaffected.
* If a `RouterUptime[]` fetch fails, the uptime and name from the data
  source are kept (MRTG blanked them).
* cfgmaker does not walk Cisco VLAN columns, so `$if_vlan_id` and
  `$if_cisco_trunk` are not available in filters, and the `catname`/`ppname`
  descriptions are not supported.
* SNMPv3 localized keys (`authkey`, `privkey`) are not supported; use
  `authpassword` / `privpassword`.
