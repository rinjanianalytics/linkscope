# Moving from MRTG

LinkScope reads the same configuration and continues the same `.log` files as
MRTG 2.17.10, so you can replace `mrtg` in place: same config, same log
directory, same history. This page walks through the switch.

## Before you start

Read [Compatibility with MRTG](compatibility.md) and check your configuration
for the few things LinkScope does not support:

- conversion subroutines (`|name` in a `Target[]`, `ConversionCode:`);
- `LogFormat: rrdtool`, `RRDCached`, `SendToGraphite` and the Holt-Winters
  thresholds (`HWThresh*`);
- external commands (backtick targets) that need a shell, unless you set
  `ExecShell: yes`;
- `SnmpOptions` written as Perl code instead of plain `key => value` pairs;
- SNMPv3 localized keys (`authkey`, `privkey`): use `authpassword` and
  `privpassword` instead.

LinkScope reports each of these when it loads the configuration, so the
check below finds them for you.

## Step by step

### 1. Install LinkScope

Build and install the binary as described in [Install and run](install.md).
Do not remove MRTG yet.

### 2. Check the configuration

Point LinkScope at your existing configuration with `--check`:

```sh
linkscope --check /etc/mrtg/mrtg.cfg
```

`--check` does not take the lock, so it is safe while MRTG is still running.
Fix anything it reports. If it refuses the file because of its permissions,
see [File permissions](install.md#file-permissions).

### 3. Stop MRTG

Only one program may write the logs. Stop MRTG before LinkScope runs:

- if MRTG runs from cron, comment out its crontab line;
- if MRTG runs as a daemon, stop it (for example `systemctl stop mrtg`) and
  disable it.

Wait until any cycle in progress has finished.

### 4. Start LinkScope

From cron, replace `mrtg` with `linkscope` on the same line:

```text
*/5 * * * * mrtg /usr/local/bin/linkscope /etc/mrtg/mrtg.cfg
```

Or run it as a daemon with the web UI (see
[Run as a daemon](install.md#run-as-a-daemon-with-the-web-ui)):

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

LinkScope continues each target's `.log` file where MRTG left it. The interface
cache (the `.ok` file) has MRTG's format, so LinkScope uses MRTG's cache as it
is.

### 5. Replace the HTML pages

LinkScope writes no HTML pages and no PNG graphs. The built-in web UI replaces
the per-target pages and `indexmaker`. Set the address it listens on, either
with `--listen` or in the configuration:

```text
WebListen: 127.0.0.1:8080
```

The UI has no login. Keep it on loopback, or put a reverse proxy with
authentication in front of it. `HtmlDir`, `ImageDir` and the page keywords are
still accepted, so the configuration needs no edit; the files MRTG generated
there are simply no longer updated.

### 6. Check the result

- Open the web UI and see every target polled and `ok` (see [Web UI](web-ui.md)).
- After a cycle, the `.log` files have new lines at the top, as before.
- From cron, the exit code is `0` when every target was polled
  (see [Exit codes](command-line.md#exit-codes)).

## Going back

The logs stay in MRTG's format, so you can return to MRTG: stop LinkScope and
start MRTG again on the same configuration.

Two things to keep in mind:

- MRTG rejects the keywords LinkScope adds (`WebListen`, `ExecShell`,
  `AlertWebhook`, `AlertWebhookSecret`, `AlertMailUser`, `AlertMailPassword`,
  `ThreshFor[]`). Remove them first, or keep them out of configurations you
  share with MRTG.
- `ThreshMailServer` with a port (`host:port`) is a LinkScope addition.

## What changes for you

| With MRTG | With LinkScope |
|---|---|
| Static HTML and PNG files, `indexmaker` | The built-in [web UI](web-ui.md) |
| Perl code in the configuration | Compiled expressions; Perl code is refused at load time |
| `rateup` started for each target | The rate engine runs in the process |
| Thresholds run programs and mails | The same, plus webhooks and authenticated mail ([Alerts](alerts.md)) |
| No metrics endpoint | [`/metrics`](metrics.md) for Prometheus |
| Lock lost when the daemon detached | The lock is held for the life of the process |
