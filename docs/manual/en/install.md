# Install and run

LinkScope is a single binary. The web UI is prebuilt and embedded in it, so a
normal Rust build is all you need.

## Build from source

You need Rust 1.88 or newer.

```sh
git clone https://github.com/rinjanianalytics/linkscope.git
cd linkscope
cargo build --release
```

The binary is `target/release/linkscope`. Install it somewhere on the `PATH`:

```sh
sudo install -m 0755 target/release/linkscope /usr/local/bin/linkscope
```

To run `cfgmaker` by its own name, add a symlink:

```sh
sudo ln -s /usr/local/bin/linkscope /usr/local/bin/cfgmaker
```

You do not need Node.js to build LinkScope. Only a change to the UI itself
needs it (the UI's sources are in `crates/web/ui`).

## Prepare the configuration

LinkScope reads MRTG's configuration format. If you do not have a
configuration yet, generate one with [cfgmaker](cfgmaker.md):

```sh
linkscope cfgmaker --global 'WorkDir: /var/lib/mrtg' --ifref=name \
    --output /etc/mrtg/mrtg.cfg public@192.0.2.1:::::2
```

The configuration must set `WorkDir:` (or `LogDir:`, `HtmlDir:` and
`ImageDir:`). Logs go under it.

Check it before the first run:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

`--check` reads and validates the configuration, prints the number of
targets and data sources, and exits. It does not poll anything.

### File permissions

The configuration holds SNMP credentials, and LinkScope refuses a
configuration file (or an included file) that other accounts could change:

- it must be owned by root or by the user LinkScope runs as;
- it must not be writable by group or others;
- its directory must not be world-writable without the sticky bit.

A good setup for a service user named `mrtg`:

```sh
sudo chown root:mrtg /etc/mrtg/mrtg.cfg
sudo chmod 640 /etc/mrtg/mrtg.cfg
```

With mode 640 and group `mrtg`, the daemon can read the file again on a
reload after it has dropped its privileges. `--allow-insecure-config` turns
the refusal into a warning; avoid it outside tests.

Keep `WorkDir` writable only by the service account.

## Run once, like MRTG from cron

Without `--daemon`, LinkScope polls every target once and exits, as MRTG does
from cron:

```sh
linkscope /etc/mrtg/mrtg.cfg
```

A crontab entry for a 5 minute `Interval`:

```text
*/5 * * * * mrtg /usr/local/bin/linkscope /etc/mrtg/mrtg.cfg
```

The exit code says how the cycle went (see [Command line](command-line.md#exit-codes)).
No web UI runs in this mode once the cycle ends.

## Run as a daemon with the web UI

In daemon mode LinkScope keeps running, polls every `Interval`, and serves the
web UI and `/metrics`:

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

`--listen` overrides the `WebListen:` keyword in the configuration. Without
either, no web UI runs. `RunAsDaemon: yes` in the configuration has the same
effect as `--daemon`.

By default the daemon detaches from the terminal, as MRTG does. When it
detaches, its messages go nowhere unless you set `--logging FILE`. Under
systemd or another supervisor, use `--foreground` (or `NoDetach: yes` in the
configuration).

### A systemd unit

```ini
[Unit]
Description=LinkScope link utilization monitoring
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/linkscope --daemon --foreground --user mrtg --group mrtg /etc/mrtg/mrtg.cfg
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Messages go to standard error, so the journal keeps them
(`journalctl -u linkscope`). Set `WebListen:` in the configuration, or add
`--listen 127.0.0.1:8080` to `ExecStart`.

LinkScope starts as root here only to read the configuration, take its lock,
write its PID file and open the web listener. It then drops to the user and
group you name, keeping no other groups, and checks that the change took.

### Signals

| Signal | Effect |
|---|---|
| `SIGHUP` | Reload the configuration; the polling schedule is kept |
| `SIGTERM`, `SIGINT` | Stop, cancelling a cycle in progress (logs are replaced atomically, so none is left half written) |

The daemon also reloads on its own when a configuration file changes. If the
new configuration has an error, it keeps the previous one and logs why.
Changes to `WebListen`, `LoadMIBs` and `RunAsDaemon` need a restart.

## Where files go

| File | Default | Option |
|---|---|---|
| Logs (`<target>.log`, `.old`) | `WorkDir` (or `LogDir`), plus the target's `Directory[]` | |
| Interface cache | the configuration's name with `.ok` (for example `/etc/mrtg/mrtg.ok`) | `--confcache-file` |
| Alert state | beside the interface cache, with `.alerts` | follows `--confcache-file` |
| Lock file | the configuration's name with `_l` (for example `/etc/mrtg/mrtg.cfg_l`) | `--lock-file` |
| PID file (daemon mode) | the configuration's name with `.pid` | `--pid-file` |
| Messages | standard error | `--logging FILE` |

With `--fhs`, LinkScope uses the Filesystem Hierarchy Standard paths instead:

| File | Path |
|---|---|
| Interface cache | `/var/lib/mrtg/mrtg.ok` |
| PID file | `/run/mrtg/mrtg.pid` |
| Lock file | `/var/lock/mrtg/mrtg.lck` |
| Messages | `/var/log/mrtg/mrtg.log` |

Options you give explicitly still win over these. Run as root with `--fhs`,
LinkScope creates the directories and gives them to the service user.

## Next steps

- [Moving from MRTG](migrating-from-mrtg.md) if you are replacing an MRTG install
- [Web UI](web-ui.md) and [Prometheus metrics](metrics.md)
- [Alerts](alerts.md) to get notified when a link is too busy
