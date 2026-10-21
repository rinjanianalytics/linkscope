# Troubleshooting

Start with two checks. They answer most questions.

```sh
# Is the configuration valid?
linkscope --check /etc/mrtg/mrtg.cfg
# Is the daemon up? (with the web UI on 127.0.0.1:8080)
curl http://127.0.0.1:8080/healthz
```

`--check` prints the number of targets and data sources and `OK`, or the
errors it found. It does not take the lock, so you can run it while the daemon
runs. `/healthz` answers `ok` while the web listener is up.

## Where to look

LinkScope writes its messages to standard error, or to the file named by
`--logging` (`/var/log/mrtg/mrtg.log` with `--fhs`). Under systemd, read them
with `journalctl -u linkscope`. A daemon that detached without `--logging`
writes its messages nowhere, so set `--logging FILE` or run it with
`--foreground`.

For more detail, raise the level with `--debug all` or `LINKSCOPE_LOG=debug`.

After each cycle the daemon logs one line:

```text
cycle: 120 targets in 1830 ms (2 failed, 0 log write failures)
```

## The configuration is refused because of its permissions

LinkScope refuses a configuration file, or a file it includes, that another
account could change, because the configuration decides what LinkScope runs.
The message says why, for example:

```text
writable by group or others (mode 664); run chmod go-w on it
```

Fix the file rather than the check:

```sh
sudo chown root:mrtg /etc/mrtg/mrtg.cfg
sudo chmod 640 /etc/mrtg/mrtg.cfg
```

The file must be owned by root or by the user LinkScope runs as. Every
directory on the way to it must belong to root or that user and must not be
writable by others, or by a group other than root's or that user's own,
unless it is sticky. The message names the directory. See
[File permissions](install.md#file-permissions). `--allow-insecure-config`
turns the error into a warning, for tests only.

## Rates look too low, or jump around, on fast links

SNMPv1 has only 32-bit counters. At 1 Gbit/s a 32-bit octet counter wraps
every 34 seconds, so a 5 minute poll cannot see how often it wrapped. Poll
interface targets with SNMPv2c or v3, which use 64-bit counters. The version is
the fifth field after the host:

```text
Target[core-uplink]: #ether1:public@192.0.2.1:::::2
```

Generate the configuration the same way:

```sh
linkscope cfgmaker --ifref=name public@192.0.2.1:::::2
```

`cfgmaker` without a version writes SNMPv1 targets, as MRTG's cfgmaker does.

## A target shows Error, Stale or Pending

The [web UI](web-ui.md) gives each target a status:

| Status | Meaning | What to check |
|---|---|---|
| Pending | Not polled yet since LinkScope started | Wait one `Interval` |
| Error | The last poll returned no data | The agent is reachable from the poller, the community or SNMPv3 user is right, the interface still exists |
| Stale | The last good poll is older than two intervals | The daemon is still running and its cycles finish (see below) |
| Warn | One direction had no value, or a threshold alert is firing | The target page shows which bound is broken |

LinkScope accepts SNMP replies only from the address it polled. An agent that
answers from another address (a router with several addresses, a NAT on the
way) looks like a timeout. Poll the address the agent answers from.

Interface targets found by name (`#ether1`), description (`\eth0`), IP address
(`/192.0.2.1`) or MAC address (`!0a-1b-2c-3d-4e-5f`) are resolved through the
interface cache (the `.ok` file). When a cached mapping no longer matches (a
router renumbered its interfaces), LinkScope forgets it and scans the device
once more. If the interface is still not found, the target reports
`could not match ...`; check that the name or address still exists on the
device. A scan that fails is logged as `interface scan of ... failed`.

## Cycles take longer than the Interval

A message like this means the cycle did not finish in time:

```text
polling took longer than Interval (300 s); consider raising --concurrency
```

LinkScope polls up to `--concurrency` agents at once (256 by default). Each
agent may use at most 80% of the `Interval` per cycle; after that its targets
are unknown for that cycle and the others go on:

```text
192.0.2.1 did not finish within 240 s; its targets are unknown this cycle
```

Raise `--concurrency` for many agents, and look at slow or unreachable agents.

## Another LinkScope is running

```text
another LinkScope instance is running with this configuration (lock /etc/mrtg/mrtg.cfg_l is held)
```

Only one LinkScope (or MRTG) may poll a configuration at a time. Check for a
daemon, a cron job, or an MRTG still running on the same configuration. The
lock is a `flock` held for the life of the process, so it disappears when that
process exits; there is no stale lock to delete.

In daemon mode, a PID file that names a running process stops a second
daemon in the same way (`LinkScope is already running with pid ...`). A PID
file left by a process that has exited is replaced.

## Logs cannot be written

```text
3 of 120 logs could not be read or written this cycle; check permissions of /var/lib/mrtg/
```

The service user must be able to write `WorkDir` (or `LogDir`) and the
`Directory[]` below it. LinkScope also refuses a log that is a symlink, and a
`Directory[]` that is a symlink. A log that cannot be written is shown as an
error in the UI, but the exit code still reflects only whether values were
collected.

## A configuration change is not picked up

The daemon reloads on `SIGHUP` and when a configuration file changes. If the
new configuration has an error, it keeps the previous one and logs:

```text
keeping the previous configuration; reload failed: ...
```

Fix the error and save again. The service user must be able to read the
configuration to reload it after dropping privileges (mode 640 with the
service group). Changes to `WebListen`, `LoadMIBs` and `RunAsDaemon` need a
restart.

## The web UI does not answer

- Check that `WebListen:` or `--listen` is set; without it no listener runs.
- From cron (no `--daemon`), the UI runs only during the cycle.
- `cannot listen on ...` at startup means the address is wrong or the port is
  taken.
- The UI has no login. If you reach it through a reverse proxy, check the
  proxy's authentication and its upstream address.
- `421` and "does not answer to this host name" means you opened it by a name
  that is not in `WebHosts:`. Add the name (as the browser shows it, without
  the port) and reload the configuration.

## Exit codes from cron

| Code | Meaning |
|---|---|
| `0` | Every target was polled |
| `91` | Some targets failed |
| `92` | Every target failed |
| `1` | LinkScope could not start, or the cycle was interrupted |

See [Command line](command-line.md#exit-codes).
