# Security notes

This maps the weaknesses found in the MRTG 2.17.10 teardown to what
LinkScope does instead. It is defensive documentation: how the design closes
each issue, not how to exploit MRTG.

| # | MRTG 2.17.10 | LinkScope |
|---|---|---|
| 1 | **Config is code.** `Target[]` is `eval`'d as Perl; `SnmpOptions` and `ConversionCode` are evaluated; backtick targets run through `/bin/sh`. Write access to the config or any included file means code execution as the MRTG user. | Target expressions compile to an arithmetic tree; `SnmpOptions` is parsed as data; conversion code is not supported. External commands run without a shell unless `ExecShell: yes`. Config files that others can modify are refused. |
| 1b | **Credentials in output.** | Every string the web API returns (errors, warnings, Target text) passes through one redaction step that removes all configured communities, SNMPv3 passwords and external command lines; runtime errors name the target, not its Target text. A test greps every endpoint's JSON for planted secrets. Credentials are excluded from serialization. |
| 2 | **Stored XSS.** sysName, uptime and RouterName values from the agent are written into generated pages unescaped. | The UI renders agent data as text (React escapes it). Config HTML (PageTop, Title, Legends) is reduced to a tag allowlist without scripts, styles or event handlers. A strict Content-Security-Policy is sent with every response. |
| 3 | **cfgmaker trusts the device.** sysName goes into `Title[]` raw, and newlines or backticks in device strings end up in the generated config. | Control characters are removed from every device string; interface references that cannot be represented safely fall back to the ifIndex; HTML-bound values (sysName included) are escaped. A regression test feeds hostile strings through the generator and parses the result. |
| 4 | **Symlink following as root** (`--fhs` directory chown; the pid file before the 30e1921 fix). | Directories are opened with `O_DIRECTORY|O_NOFOLLOW` and changed with `fchown`; the pid file is created with `O_EXCL|O_NOFOLLOW`. Every output file (logs, cache, threshold state) is written to a fresh temp file with an unpredictable name, created `O_EXCL|O_NOFOLLOW`, and renamed into place. `Directory[]` subdirectories that are symlinks are refused. Logs are read with `O_NOFOLLOW|O_NONBLOCK`, only if they are regular files, and at most 4 MB, so a FIFO or device in place of a log cannot hang or exhaust the daemon. WorkDir itself is trusted: keep it writable only by the service account. |
| 5 | **Supplementary groups kept** after `--user`, and only the uid is verified. | `setgroups([gid])`, then `setgid`, `setuid`, and both ids plus the group list are verified. |
| 6 | **Lock lost in daemon mode**; stale-lock detection depends on `/proc`. | `flock` held by the process for its lifetime. |
| 7 | **Path traversal** through target names and `Directory[]`. | Target names may not contain path separators. `Directory[]` is reduced to plain relative components (a leading `/` is dropped, exactly as MRTG's string concatenation treats it; `..` is rejected), so every log stays under LogDir. |
| 8 | **Lenient SNMP reply matching** by default; v1/v2c communities in clear text. | Replies accepted only from the polled address (connected UDP socket), plus request-id and community checks. SNMPv3 authPriv (SHA-1/SHA-2, AES/DES) is supported and recommended. |
| 9 | **Memory safety.** `rateup` copies values into fixed C buffers. | Rust with no `unsafe` in the parser, evaluator and rate engine (`forbid(unsafe_code)` in the rate engine); over-long or malformed values are rejected instead of overflowing. Tested against 2,000 randomly corrupted logs. (Packets are decoded by the `snmp2` crate; it is not fuzzed as part of this project.) |
| 9b | **Hostile or slow agents.** | Each agent has a time budget per cycle (80% of Interval); table walks are limited by time, rows and bytes; agent numbers longer than 64 characters are rejected; a stuck agent delays the cycle by at most that budget and cannot block the others. Panics are contained per target. |
| 10 | **Fork failure** at the rateup pipe could replace the daemon with rateup. | No per-target processes: the rate engine runs in-process. |

`ExecShell: yes` only decides whether LinkScope itself interprets shell syntax;
it is not a sandbox. Anyone who can edit the config can run programs as the
service user, which is why the config files themselves are checked.

## Operating advice

* Run as an unprivileged user (`--user mrtg`), or let systemd do it. With
  `--user`, LinkScope reads the config, takes the lock and creates the pid
  file before dropping privileges, then creates the log directories as that
  user.
* Keep `mrtg.cfg` private (it holds SNMP credentials): `chown root:mrtg` and
  `chmod 640`, so the daemon can re-read it on reload after dropping
  privileges. (`chmod 600` owned by root works too, but then reloads fail
  and you restart instead.)
* Keep WorkDir writable only by the service account.
* Prefer SNMPv3 with `authPriv`; restrict agents to the poller's address.
* The web UI is read-only and has no login. It listens on loopback unless
  you choose otherwise; put a reverse proxy with authentication in front of
  it before exposing it.
