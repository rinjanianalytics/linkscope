# JSON API

The web UI reads everything it shows from a small JSON API on the same listener. You can read it
too, for scripts, dashboards or other tools. The API is read-only: nothing in it changes LinkScope
or its configuration.

| Method and path | Returns |
|---|---|
| `GET /api/summary` | The daemon's status and target counts |
| `GET /api/targets` | Every target with its live status |
| `GET /api/targets/{name}` | One target's details and display settings |
| `GET /api/targets/{name}/series` | One period's graph points and MRTG's Max, Average and Current |
| `GET /healthz` | `ok` as plain text while the server runs |

Rates are in the log's unit: bytes per second, or per minute with `Options[]: perminute`, or per
hour with `perhour`. Each target's `format` says how MRTG would print them. For rates always per
second, use [`/metrics`](metrics.md).

## Errors

An error is an HTTP status with a plain-text message:

| Status | When |
|---|---|
| 400 | An unknown `period` for a series. |
| 404 | No target with that name, or no data collected yet for a series. |
| 503 | The configuration is not loaded yet (just after start). |
| 500 | The target's log could not be read. |

## Secrets

The API never returns a secret. SNMP communities in a `Target[]` are masked
(`2:•••@192.0.2.1`), and SNMPv3 passwords and keys, external command lines, the webhook URL, its
secret and the mail password are replaced with `•••` wherever they would appear, including in
error messages. The paths of the threshold programs and the threshold mail address are not
given either, only whether they are set. HTML from the config (`Title[]`, `PageTop[]`, `PageFoot[]`, legends) is cleaned to
a safe subset: structure and text formatting, and links to `http` and `https` only.

## GET /api/summary

```json
{
  "version": "0.2.0",
  "started": 1760940000,
  "now": 1760943600,
  "cycles": 12,
  "last_cycle": { "started": 1760943300, "duration_ms": 412, "targets": 24, "ok": 23, "failed": 1, "write_failures": 0 },
  "interval_secs": 300,
  "config_name": "mrtg.cfg",
  "warnings": [],
  "targets_total": 24,
  "targets_ok": 22,
  "targets_error": 2,
  "targets_warn": 1,
  "targets_failed": 1,
  "targets_stale": 0,
  "targets_pending": 0
}
```

| Field | Type | Meaning |
|---|---|---|
| `version` | string | LinkScope's version. |
| `started` | number | When the daemon started, Unix time. |
| `now` | number | The server's time, Unix time. |
| `cycles` | number | Polling cycles completed. |
| `last_cycle` | object or null | The last cycle: `started`, `duration_ms`, `targets`, `ok`, `failed`, `write_failures` (logs not read or written). Null before the first. |
| `interval_secs` | number | Seconds between cycles. |
| `config_name` | string | The configuration's file name (not its path, which the UI does not reveal). |
| `warnings` | array of strings | Warnings from loading the configuration. |
| `targets_total` | number | Targets configured. |
| `targets_ok` | number | Targets with the status `ok`. |
| `targets_warn`, `targets_failed`, `targets_stale`, `targets_pending` | number | Targets with the status `warn`, `error`, `stale` and `pending`. |
| `targets_error` | number | `warn`, `error` and `stale` together (kept for older clients). |

## GET /api/targets

An array with one object per target, in the configuration's order:

```json
[
  {
    "name": "core-uplink",
    "title_html": "Core uplink",
    "directory": "core",
    "status": "ok",
    "error": null,
    "last_poll": 1760943300,
    "sys_name": "edge1.example.com",
    "uptime": "41 days, 3:12:08",
    "max_bytes1": 125000000,
    "max_bytes2": 125000000,
    "format": { "mul": 8.0, "factor": 1.0, "kilo": 1000.0, "units": ["b/s", "kb/s", "Mb/s", "Gb/s"], "integer": false, "nopercent": false },
    "legend_i": "In",
    "legend_o": "Out",
    "colours": [{ "name": "GREEN", "rgb": "#00cc00" }, { "name": "BLUE", "rgb": "#0000ff" }],
    "no_i": false,
    "no_o": false,
    "growright": true,
    "logscale": false,
    "cur_in": 6514625.0,
    "cur_out": 1203750.0,
    "cur_in_text": "52.1 Mb/s (5.2%)",
    "cur_out_text": "9.6 Mb/s (1.0%)",
    "alerts": []
  }
]
```

| Field | Type | Meaning |
|---|---|---|
| `name` | string | The target's name. |
| `title_html` | string | `Title[]`, cleaned HTML. |
| `directory` | string | `Directory[]`, without the trailing slash. |
| `status` | string | `ok`, `warn`, `error`, `stale` or `pending`. See [Web UI](web-ui.md#status). |
| `error` | string or null | Why data was missing on the last poll. |
| `last_poll` | number or null | Unix time of the last sample. |
| `sys_name`, `uptime` | string or null | The device's name and uptime as it reported them. |
| `max_bytes1`, `max_bytes2` | number | `MaxBytes` for in and out. |
| `format` | object | How MRTG prints rates: `mul` (8 with `bits`), `factor` (`Factor[]`), `kilo` (`Kilo[]`), `units` (labels by power of `kilo`), `integer`, `nopercent`. |
| `legend_i`, `legend_o` | string | `LegendI[]` and `LegendO[]`, cleaned HTML. |
| `colours` | array | `Colours[]`: `name` and `rgb`. |
| `no_i`, `no_o`, `growright`, `logscale` | boolean | The matching `Options[]`. |
| `cur_in`, `cur_out` | number or null | The newest rates, in the log's unit. |
| `cur_in_text`, `cur_out_text` | string or null | The same, formatted as MRTG prints them. |
| `alerts` | array | Threshold bounds broken now, firing or waiting. |

Each item of `alerts`:

| Field | Type | Meaning |
|---|---|---|
| `direction` | string | `in` or `out`. |
| `bound` | string | `min` or `max`. |
| `threshold` | string | The bound as configured, for example `80%`. |
| `threshold_rate`, `rate` | number | The bound and the current rate, in the log's unit. |
| `percent` | number or null | The rate in percent of `MaxBytes`, for a percent bound. |
| `firing` | boolean | True once broken for `polls_needed` polls. |
| `polls`, `polls_needed` | number | Broken polls in a row, and the `ThreshFor[]` it fires at. |
| `since` | number | Unix time of the first broken poll. |
| `fired_at` | number or null | Unix time of the poll that fired it. |

## GET /api/targets/{name}

Everything in a `/api/targets` item, plus the target's display settings:

| Field | Type | Meaning |
|---|---|---|
| `target_text` | string | The `Target[]` text, communities masked. |
| `page_top_html`, `page_foot_html` | string | `PageTop[]` and `PageFoot[]`, cleaned HTML. |
| `y_legend` | string | `YLegend[]`. |
| `legends` | array of 5 strings | `Legend1[]` to `Legend5[]`, cleaned HTML. |
| `periods` | array of strings | The graphs to show: `day`, `week`, `month`, `year`, less `Suppress[]`. |
| `with_peak`, `unscaled` | array of strings | The periods named in `WithPeak[]` and `Unscaled[]`. |
| `growright`, `logscale`, `expscale`, `secondmean`, `dorelpercent`, `avgpeak`, `noinfo`, `nolegend` | boolean | The matching `Options[]`. |
| `xsize`, `ysize`, `ytics`, `yticsfactor` | number | `XSize[]`, `YSize[]`, `YTics[]`, `YTicsFactor[]`. |
| `abs_max` | number | `AbsMax[]`. |
| `timezone` | string or null | `Timezone[]`. |
| `thresholds` | object | The target's threshold settings: `min_i`, `min_o`, `max_i`, `max_o`, `desc` and `for_polls` (`ThreshFor[]`), null when unset; `programs` (boolean: a `ThreshProg*[]` is set) and `mail` (boolean: `ThreshMailAddress[]` is set). Which program and which address are not given. |

## GET /api/targets/{name}/series

Query parameters:

| Parameter | Default | Meaning |
|---|---|---|
| `period` | `day` | `day`, `week`, `month` or `year`. |
| `maxx` | the target's `XSize[]` | Points wanted, from 8 to 2000. |

```json
{
  "period": "day",
  "step": 300,
  "now": 1760943300,
  "points": [
    { "t": 1760857200, "in": 6013500.0, "out": 1100250.0, "in_max": 6201000.0, "out_max": 1180500.0 }
  ],
  "peak": false,
  "fixed_max": null,
  "stats": {
    "max_in": 9120000, "max_out": 2040000, "avg_in": 5210000, "avg_out": 1150000,
    "cur_in": 6514625, "cur_out": 1203750, "avg_max_in": 0, "avg_max_out": 0,
    "max_pct": 7, "avg_pct": 4, "cur_pct": 5,
    "text": {
      "max_in": "73.0 Mb/s (7.3%)", "max_out": "16.3 Mb/s (1.6%)",
      "avg_in": "41.7 Mb/s (4.2%)", "avg_out": "9.2 Mb/s (0.9%)",
      "cur_in": "52.1 Mb/s (5.2%)", "cur_out": "9.6 Mb/s (1.0%)",
      "avg_max_in": "0.0 b/s (0.0%)", "avg_max_out": "0.0 b/s (0.0%)"
    }
  }
}
```

| Field | Type | Meaning |
|---|---|---|
| `period` | string | The period returned. |
| `step` | number | Seconds per point. |
| `now` | number | The time of the log's last update, Unix time. |
| `points` | array | Oldest first: `t` (Unix time), `in`, `out`, and the peaks `in_max`, `out_max`, in the log's unit. |
| `peak` | boolean | True when `WithPeak[]` covers this period. |
| `fixed_max` | number or null | A fixed y axis maximum when `Unscaled[]` covers this period. |
| `stats` | object | MRTG's Max, Average, Current and Average max for in and out (log units), the percentages of `MaxBytes` (`max_pct`, `avg_pct`, `cur_pct`, each a number or null), and `text`, the same figures formatted as MRTG prints them. |

## Example

```sh
curl -s http://127.0.0.1:8080/api/targets \
  | jq -r '.[] | select(.status != "ok") | "\(.name) \(.status) \(.error // "")"'
```
