# Prometheus metrics

LinkScope serves `GET /metrics` on the web listener (`WebListen:` or `--listen`) in the
Prometheus text format (version 0.0.4). It holds every target's current rates, the counters as
last read, `MaxBytes`, the status and the last poll time, and the last polling cycle's outcome.

## Scrape it

```yaml
scrape_configs:
  - job_name: linkscope
    static_configs:
      - targets: ['127.0.0.1:8080']
```

Scrape at about the polling interval or faster. The values change once per cycle (`Interval`,
five minutes by default). Until the configuration is loaded, `/metrics` answers 503.

## Labels

Every per-target series carries these labels:

| Label | Meaning |
|---|---|
| `target` | The target's name, as in `Target[name]`. |
| `directory` | Its `Directory[]`, without the trailing slash (empty when none). |
| `source` | `snmp` for one SNMP source, `exec` for an external program, `expression` for arithmetic over several sources. |
| `host` | The SNMP host, for an `snmp` target. Empty otherwise. |
| `interface` | The interface selector as written, as `method:key`, for example `Name:ether1`, `Descr:uplink` or `If:3`. Empty when the target names an OID or index directly. |
| `ifindex` | The ifIndex the selector resolved to on the last poll. Empty until then. |
| `direction` | `in` or `out`, on the series that have a direction. |

Labels never hold a community, an SNMPv3 password or a command line. Every label value passes
through the same redaction as the [JSON API](api.md).

## Metrics

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `linkscope_info` | gauge | `version` | Always 1. The `version` label is LinkScope's version. |
| `linkscope_start_time_seconds` | gauge | none | When the daemon started, Unix time. |
| `linkscope_interval_seconds` | gauge | none | Seconds between polling cycles (`Interval`). |
| `linkscope_cycles_total` | counter | none | Polling cycles completed since the daemon started. |
| `linkscope_last_cycle_start_time_seconds` | gauge | none | When the last cycle started, Unix time. |
| `linkscope_last_cycle_duration_seconds` | gauge | none | How long the last cycle took, in seconds. |
| `linkscope_last_cycle_targets` | gauge | `outcome` (`ok`, `failed`) | Targets in the last cycle, by outcome. |
| `linkscope_last_cycle_log_write_failures` | gauge | none | Logs the last cycle could not read or write. |
| `linkscope_target_status` | gauge | target labels, `status` | 1 for the target's current status (`ok`, `warn`, `error`, `stale`, `pending`), 0 for the other four. |
| `linkscope_target_last_poll_time_seconds` | gauge | target labels | When the target's last sample was taken, Unix time. |
| `linkscope_target_rate` | gauge | target labels, `direction` | The newest rate per second, as the UI shows it before formatting. Bytes per second for interface counters. |
| `linkscope_target_max` | gauge | target labels, `direction` | `MaxBytes` for each direction, per second in the target's units. |
| `linkscope_target_counter_total` | counter | target labels, `direction` | The counter as last read: octets for an interface target. Absent for targets with `gauge` or `absolute`. |
| `linkscope_target_value` | gauge | target labels, `direction` | The value as last read, for targets with `Options[]: gauge` or `absolute`. |

The four `linkscope_last_cycle_*` series appear after the first cycle. A target's rate, counter
and last poll appear after its first poll.

Rates are per second even for targets with `perminute` or `perhour`: LinkScope divides by 60 or
3600 for you. The `bits` option changes only how the UI prints a rate; `linkscope_target_rate`
stays in bytes, so multiply by 8 for bits per second.

## Example queries

Inbound traffic of one target in bits per second, from LinkScope's own rate:

```promql
linkscope_target_rate{target="core-uplink", direction="in"} * 8
```

The same from the counter, averaged over ten minutes (the window must hold at least two polls):

```promql
rate(linkscope_target_counter_total{target="core-uplink", direction="in"}[10m]) * 8
```

Utilization in percent of `MaxBytes`, for every target and direction (the two metrics carry the
same labels, so they match one to one):

```promql
100 * linkscope_target_rate / linkscope_target_max
```

Targets that are not `ok`:

```promql
linkscope_target_status{status!="ok"} == 1
```

Targets not polled in the last 15 minutes:

```promql
time() - linkscope_target_last_poll_time_seconds > 900
```

## 32-bit counters

`cfgmaker` writes SNMPv1 targets unless you ask for another version, as MRTG's does. SNMPv1
counters are 32 bits: at 1 Gbit/s they wrap every 34 seconds, so a five-minute poll cannot tell how
many times a busy interface wrapped. Use SNMPv2c or v3 for interface targets, which read the 64-bit
counters:

```text
Target[core-uplink]: #ether1:public@192.0.2.1:::::2
```

or generate the config with `cfgmaker public@192.0.2.1:::::2`. See [cfgmaker](cfgmaker.md).
