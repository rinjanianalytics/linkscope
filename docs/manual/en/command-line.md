# Command line

```text
linkscope [options] [config]
linkscope cfgmaker [cfgmaker options] community@router ...
```

`config` is the configuration file in MRTG's format. It defaults to
`/etc/mrtg/mrtg.cfg`.

Without `--daemon` (and without `RunAsDaemon: yes` in the configuration),
LinkScope polls every target once and exits, as MRTG does from cron. With it,
LinkScope keeps running and polls every `Interval`.

`linkscope cfgmaker` runs the configuration generator; so does a symlink to
`linkscope` named `cfgmaker`. See [cfgmaker](cfgmaker.md).

## Options from MRTG

LinkScope accepts every option of MRTG's `mrtg` command.

| Option | What it does |
|---|---|
| `--user USER` | Run as this user (name or uid) after startup. LinkScope reads the configuration, takes the lock, writes the PID file and opens the web listener first, then drops privileges |
| `--group GROUP` | Run as this group (name or gid). Needs `--user`. Without it, the user's own group is used |
| `--lock-file FILE` | The lock file. Default: the configuration's path followed by `_l` |
| `--confcache-file FILE` | The interface cache. Default: the configuration's path with the extension `.ok`. The alert state is kept beside it, with the extension `.alerts` |
| `--logging FILE` | Write messages to this file instead of standard error. `eventlog` is accepted and means standard error |
| `--check` | Check the configuration, print how many targets and data sources it has, and exit. It does not take the lock |
| `--fhs` | Use Filesystem Hierarchy Standard paths for the cache, PID file, lock and messages (see [Where files go](install.md#where-files-go)) |
| `--daemon` | Keep running and poll every `Interval` |
| `--pid-file FILE` | The PID file in daemon mode. Default: the configuration's path with the extension `.pid` |
| `--debug VALUE` | Accepted for MRTG's debug categories. Any value raises the log level to debug |
| `--log-only` | Accepted and has no effect: LinkScope never writes HTML pages |

## Options LinkScope adds

| Option | What it does |
|---|---|
| `--listen ADDR:PORT` | Serve the web UI, the JSON API and `/metrics` on this address. Overrides `WebListen:` in the configuration |
| `--concurrency N` | How many agents are polled at the same time. Default `256` |
| `--foreground` | With `--daemon`: stay attached to the terminal, for systemd and other supervisors. `NoDetach: yes` in the configuration does the same |
| `--allow-insecure-config` | Load configuration files that other accounts could change, with a warning instead of an error |
| `--help`, `--version` | Print the usage or the version |

Paths given to these options are made absolute when LinkScope starts, so
nothing depends on the working directory.

## Environment

| Variable | What it does |
|---|---|
| `LINKSCOPE_LOG` | The log level, for example `debug` or `warn`. Overrides the default `info` and `--debug` |
| `LINKSCOPE_FOREGROUND` | When set, the daemon does not detach, as with `--foreground` |
| `SSL_CERT_FILE` | A file of certificate authorities for alert mail and webhooks, instead of the system's (see [Alerts](alerts.md)) |

## Exit codes

The exit codes match MRTG's.

| Code | Meaning |
|---|---|
| `0` | Every target was polled. In daemon mode: the daemon stopped normally |
| `91` | Some targets could not be polled this cycle |
| `92` | No target could be polled this cycle |
| `1` | LinkScope could not start (for example a configuration error, a lock already held, or a file it may not use), or a single cycle was interrupted by `SIGTERM` or `SIGINT` |

A log that cannot be written is reported as an error in the UI and in the
messages, but it does not change the exit code. As with MRTG, the exit code
says only whether values were collected.

## Examples

Poll once with the default configuration:

```sh
linkscope
```

Check a configuration while the daemon is running:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

Run under systemd with the web UI on loopback:

```sh
linkscope --daemon --foreground --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

Poll a large network with more agents at once:

```sh
linkscope --daemon --concurrency 512 --logging /var/log/linkscope.log /etc/mrtg/mrtg.cfg
```
