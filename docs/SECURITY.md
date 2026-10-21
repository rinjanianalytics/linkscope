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

## Found and fixed before going public (2026-10-21)

An assessment of LinkScope itself, by attacker position (anyone who reaches the web UI, a polled
device or a spoofed reply, whoever influences the configuration, a local user, the outbound
alerts), with `cargo audit` and `npm audit`. Fixed:

| What | Who | Now |
|---|---|---|
| Half-sent HTTP requests held every file descriptor; the UI stopped and polling opened no socket and wrote no log | anyone who reaches the UI | at most 256 connections (the rest wait in the kernel's queue), a 10 s header timeout, 120 s per connection |
| A value written with a huge exponent (`1e999999999`) passed MRTG's number check and was expanded to gigabytes, or aborted the daemon | a polled device | values judged by width (MRTG's 190 digits) before anything expands them |
| snmp2 sets a v3 session's engine boots and time before checking a reply's authentication: one forged datagram broke the session until a restart | a spoofed reply | a v3 session whose replies are refused, or whose authentication fails, is opened again |
| The `.ok` cache, which names devices by SNMP community, was readable by every local user | local users | `.ok` and `.alerts` written owner only, read without following symlinks, with a size cap |
| A non-ASCII character at byte 6 or 7 of a config line panicked the reader; on a reload, the daemon died | the config | fixed; a reload that fails for any reason keeps the old configuration |
| No Host check on a UI without a login (DNS rebinding) | a web page in a browser on the network | only IP addresses, `localhost` and `WebHosts:` names are answered |
| The trust check looked only at the config's own directory, and only for world-writable | local users in a shared group | every directory on the way: root's or the user's, writable by no one else |
| rustls-webpki 0.103.15, RUSTSEC-2026-0382 | a TLS man in the middle | 0.103.16 |
| A slow mail server or webhook receiver made alert notices be dropped, though the alert was recorded as fired | whoever slows the receiver | notices wait in a bounded queue |
| SNMPv3 settings with a privacy password but no authentication polled in clear | a config mistake | refused, as is an empty password |
| A cached view keyed on a memory address could be stale after reloads (an out-of-bounds index, a new secret not redacted) | none (a latent bug) | the cache holds the configuration it was made for |
| Unbounded expression and filter chains overflowed the stack; huge literals exhausted memory | the config, cfgmaker's operator | 1,000 parts at most; literals within 190 digits |
| The costliest request (a graph's log summary) had no limit | anyone who reaches the UI | four at a time |
| GET values were placed by position, so a stray or stale reply could put one interface's counter on another | a spoofed or late reply | values used only for the OID they name |
| After a timeout the library reused the request id, so a late reply could answer the next request | a late or spoofed reply | the next request goes out from a new socket |
| Walks counted values loosely and held five full walks; the `.ok` cache could grow to tens of MB per device | a polled device | 100,000 rows per walk with per-row cost, 8 MB of cache per device, rows turned into cache entries as each walk ends |
| cfgmaker had no interface cap | a polled device | 50,000 interfaces at most |
| sysName and uptime had no length cap | a polled device | 256 characters |
| An EXEC target's background processes outlived it, one more each cycle | the command | its process group is ended when it finishes |
| `PathAdd` with `PATH` unset put the current directory on the search path | the environment | `PathAdd` alone |
| cfgmaker's header showed keys and communities | readers of the generated file | masked |
