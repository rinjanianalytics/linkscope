# Introduction

LinkScope monitors how busy your links are. It polls routers and switches over
SNMP, keeps the history of each interface, and shows it in a web UI. It is part
of the FlowScope and PathScope family by Rinjani Analytics.

LinkScope is a reimplementation of [MRTG](https://oss.oetiker.ch/mrtg/), the
Multi Router Traffic Grapher by Tobias Oetiker and contributors, written in
Rust. It is derived from MRTG 2.17.10 and is free software under the GNU
General Public License, version 2 or (at your option) any later version.
LinkScope is not affiliated with or endorsed by the MRTG project.

## A drop-in replacement

LinkScope reads your existing `mrtg.cfg` and continues your existing `.log`
files. The logs it writes are byte-identical to the ones MRTG 2.17.10 writes,
so you can switch an install over without losing history, and switch back if
you need to.

It accepts MRTG's command line options and exit codes, and it includes
`cfgmaker` to generate a configuration from a device.

## What it adds

- **A web UI built into the binary.** An overview of every target and a page
  per target with day, week, month and year graphs. It replaces the static
  HTML and PNG pages and `indexmaker`. It needs no internet access.
- **Prometheus metrics.** `GET /metrics` on the same listener as the UI.
- **Alerts that reach people.** Threshold alerts by signed webhook and by mail
  over STARTTLS with a login, with a firing and a resolved notice.
- **Modern SNMP.** SNMP v1, v2c and v3, with SHA-2 authentication and AES
  privacy.
- **Speed.** Agents are polled concurrently with batched requests, and the
  rate engine runs inside the process instead of starting `rateup` for every
  target.
- **Safety.** The configuration is never run as code, external commands run
  without a shell unless you allow it, and web output is sanitized. See
  [Security](security.md).

## How this manual is organized

| Page | What it covers |
|---|---|
| [Install and run](install.md) | Building, installing, running from cron or as a service |
| [Moving from MRTG](migrating-from-mrtg.md) | Switching an existing MRTG install to LinkScope |
| [Command line](command-line.md) | Every option and exit code |
| [Configuration](configuration.md) | The `mrtg.cfg` keywords LinkScope reads, old and new |
| [cfgmaker](cfgmaker.md) | Generating a configuration from a device |
| [Web UI](web-ui.md) | The overview and target pages |
| [Prometheus metrics](metrics.md) | The `/metrics` endpoint |
| [Alerts](alerts.md) | Thresholds, webhooks and mail |
| [JSON API](api.md) | The API the web UI reads |
| [Security](security.md) | How LinkScope closes MRTG's known weaknesses |
| [Compatibility with MRTG](compatibility.md) | Everything that behaves differently from MRTG |
| [Troubleshooting](troubleshooting.md) | Common problems and how to solve them |
