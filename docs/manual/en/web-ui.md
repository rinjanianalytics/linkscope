# Web UI

LinkScope has a web UI built into the binary. It replaces the HTML and PNG pages that MRTG and
`indexmaker` wrote. The UI is read-only, needs no internet access, and draws graphs in the browser
only when you look at them.

This manual is also available in the UI at `/docs`, in English and Bahasa.

## Turn it on

Give LinkScope an address to listen on, either in the config or on the command line:

```text
WebListen: 127.0.0.1:8080
```

```sh
linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

`--listen` overrides `WebListen:`. LinkScope opens the port before it drops privileges, so it can
bind a low port and still run as `--user`. Run it as a daemon (`--daemon`, or `RunAsDaemon: yes`)
so the UI stays up between polling cycles.

The same listener serves:

| Path | What |
|---|---|
| `/` | The overview |
| `/target/<name>` | One target's page |
| `/docs` | This manual |
| `/api/...` | The [JSON API](api.md) the UI reads |
| `/metrics` | [Prometheus metrics](metrics.md) |
| `/healthz` | Answers `ok` while the server runs |

## Who can see it

The UI has no login. Anyone who can reach the port sees every target, its rates and its graphs.
Listen on `127.0.0.1` or on a management network, or put a reverse proxy with authentication in
front of it. Never expose it to the internet.

The UI never shows a secret. SNMP communities in a `Target[]` show as `•••`, and SNMPv3
passwords, external command lines, the webhook URL and the mail password never leave the process.
See [Security](security.md).

## The overview

The overview opens at `/`. At the top it shows:

- **Targets OK**: targets polled successfully in the last cycle.
- **With problems**: targets with the status warn, error or stale, broken down by status.
- **Pending**: targets not polled yet since LinkScope started.
- **Last cycle**: when the last polling cycle ran and how long it took.
- **Interval**: the time between polling cycles.
- **Uptime**: how long the daemon has run.

Below that is one card per target. A card shows the target's title, its status, the system name
the device reported, the current in and out rates (formatted the way MRTG prints them, with the
percentage of `MaxBytes`), a sparkline of the last day, and any threshold alert that is firing or
waiting.

You can narrow and order the list:

- **Search** by target name, title or system name.
- **Filter** by status: all, problems (warn, error and stale), or one status.
- **Sort** by name, title or status.
- **Group** by `Directory[]`. Grouping is offered when targets use more than one directory.

The search, filter, sort, grouping and page live in the address bar, so a link or the back button
restores them. The overview refreshes every 30 seconds.

## A target's page

Click a card to open `/target/<name>`. The page shows:

- The title, the status, and any alert on the target with how long it has been firing.
- The system name, the uptime and the maximum speed (`MaxBytes`), unless `Options[]: noinfo` is
  set, and the time of the last poll.
- The target's name and its `Target[]` text, with communities masked.
- `PageTop[]` and `PageFoot[]`, cleaned to a safe subset of HTML (no scripts, styles or images).
- One graph per period: the last day, week, month and year. `Suppress[]` hides periods.

Under each graph is MRTG's table of **Max**, **Average** and **Current** for in and out, with
**Average max** too when `Options[]: avgpeak` is set. The graphs honor the target's settings:

- `Options[]`: `bits`, `growright`, `logscale`, `perminute`, `perhour`, `integer`,
  `nopercent`, `noi`, `noo`, `nolegend` and the others MRTG uses for drawing.
- `kMG[]`, `Kilo[]`, `Factor[]`, `ShortLegend[]`, `YLegend[]`, `LegendI[]`, `LegendO[]` and
  `Legend1[]` to `Legend5[]`.
- `WithPeak[]` (the peak line for the periods it names) and `Unscaled[]` (a fixed y axis at
  `MaxBytes`).
- `Colours[]`.

The page reloads the target every 30 seconds and each graph every minute. A graph with no log yet
says "No data collected yet".

## Status

Each target has one status. The API and `/metrics` use the names in the first column.

| Status | UI label | Meaning |
|---|---|---|
| `ok` | OK | Polled successfully in the last cycle. |
| `warn` | Warning | Polled, but one direction was unknown, or a threshold alert is firing. |
| `error` | Error | No data this cycle, or the log could not be read or written. |
| `stale` | Stale | The last sample is older than two intervals plus a minute. |
| `pending` | Pending | Not polled yet since LinkScope started. |

A threshold bound that is broken but has not yet reached `ThreshFor[]` polls shows as waiting
under the target ("1 of 3 polls before it fires"), and the target stays `ok`. See
[Alerts](alerts.md).

## Theme

The button at the top right switches between light and dark. Without a saved choice, the UI
follows the system's setting.
