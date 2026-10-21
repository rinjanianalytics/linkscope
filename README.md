# LinkScope

Link utilization monitoring by [Rinjani Analytics](https://rinjanianalytics.com),
part of the FlowScope / PathScope family.

LinkScope is a memory-safe, drop-in reimplementation of
[MRTG](https://github.com/oetiker/mrtg) (the Multi Router Traffic Grapher by
Tobias Oetiker and contributors) in Rust, with a built-in web UI. It is not
affiliated with or endorsed by the MRTG project. See [NOTICE](NOTICE).

* **Drop-in.** Reads your existing `mrtg.cfg` and continues your existing
  `.log` files. It produces byte-identical logs to MRTG 2.17.10 (see
  [Verification](#verification)).
* **Safer.** No Perl `eval` of configuration, no shell for external commands
  unless you allow it, sanitized web output, symlink-safe file handling,
  proper privilege drop. See [docs/SECURITY.md](docs/SECURITY.md).
* **Faster.** Agents are polled concurrently with batched requests and the
  rate engine runs in-process (MRTG forked `rateup` once per target).
* **Modern.** SNMP v1, v2c and v3 (SHA-2 and AES), and a web UI built into
  the single binary, replacing the static HTML/PNG pages and `indexmaker`.

## Manual

The manual is built into LinkScope: open `/docs` on the web UI (the **Docs** link in the header),
in English or Bahasa. It always matches the version you run and needs no internet access. Its
pages are Markdown in [docs/manual](docs/manual) (`en/`, `id/`), in mdBook's layout, so they
also read well here on GitHub.

## Quick start

```sh
cargo build --release
# one cycle, like MRTG from cron
./target/release/linkscope /etc/mrtg/mrtg.cfg
# or as a daemon with the web UI
./target/release/linkscope --daemon --user mrtg --listen 127.0.0.1:8080 /etc/mrtg/mrtg.cfg
```

Generate a config the way you did with cfgmaker:

```sh
linkscope cfgmaker --global 'WorkDir: /var/lib/mrtg' --ifref=name \
    --output /etc/mrtg/mrtg.cfg public@router1 public@switch2:::::2
# SNMPv3
linkscope cfgmaker --username mrtg --authprotocol sha --authpassword '...' \
    --privprotocol aes --privpassword '...' router1:::::3
```

A symlink named `cfgmaker` pointing at `linkscope` also works.

### Command line

All of MRTG's options are accepted: `--user`, `--group`, `--lock-file`,
`--confcache-file`, `--logging`, `--check`, `--fhs`, `--daemon`,
`--pid-file`, `--debug`, `--log-only`. Exit codes match MRTG (0, 91 when some
targets failed, 92 when all failed).

New options:

| Option | Purpose |
|---|---|
| `--listen ADDR:PORT` | Serve the web UI (or `WebListen:` in the config) |
| `--concurrency N` | Agents polled at the same time (default 256) |
| `--foreground` | Stay attached in daemon mode (systemd) |
| `--allow-insecure-config` | Load config files others can write (warns) |

New config keywords: `WebListen: 127.0.0.1:8080`, `ExecShell: yes`, and the alert
keywords below.

## Web UI

The overview lists every target with its status, current rates and a day
sparkline, filterable and grouped by `Directory[]`. Each target page shows
the day, week, month and year graphs with MRTG's Max/Average/Current figures,
honouring `Options[]` (bits, growright, logscale, perminute, ...), `kMG`,
`Kilo`, `Factor`, legends, `WithPeak`, `Unscaled` and `Suppress`. The UI is
read-only, embedded in the binary, and needs no internet access.

### Prometheus

The same listener serves `GET /metrics` for Prometheus: every target's current rates
(`linkscope_target_rate`), the counters as last read (`linkscope_target_counter_total`, so
`rate()` works), `MaxBytes`, status and last poll time, and the last cycle's duration and
outcome. Labels name the target, its `Directory[]`, the SNMP host and the interface (the
selector as written and the ifIndex it resolved to); never a community or a password.

```yaml
scrape_configs:
  - job_name: linkscope
    static_configs: [{ targets: ['127.0.0.1:8080'] }]
```

Interface targets need SNMPv2c or v3 (`router:::::2` in the Target, `cfgmaker router:::::2`)
for 64-bit counters; SNMPv1 counters are 32 bits and wrap every 34 seconds at 1 Gbit/s.

## Alerts

MRTG's thresholds work as they always have: `ThreshMinI[]`, `ThreshMaxO[]` and the rest,
`ThreshProgI[]`, `ThreshProgOKI[]`, `ThreshMailAddress[]`, `ThreshDir`, `ThreshHyst`. LinkScope
adds, all off until set:

```
ThreshMaxI[core-uplink]: 80%
ThreshFor[core-uplink]: 3            # broken for 3 polls in a row before it fires

AlertWebhook: https://alerts.example.com/linkscope
AlertWebhookSecret: 3f7c9a1e5b2d4c6f8a0e   # at least 16 characters

ThreshMailServer: smtp.example.com:587
ThreshMailSender: linkscope@example.com
AlertMailUser: linkscope@example.com
AlertMailPassword: app-password
```

- **The UI shows it.** A target with a firing alert shows as warn, with the bound that is
  broken (`In above 80%, now 86.2%`). A bound broken for fewer polls than `ThreshFor[]` shows
  as waiting, and the target stays ok.
- **`ThreshFor[x]: N`** fires after N broken polls in a row and resolves at the first poll that
  is not broken. With it set, the target's `ThreshProg*` and `ThreshMailAddress[]` run once when
  the alert fires and once when it resolves, in MRTG's message format; without it, they run
  exactly as MRTG runs them.
- **`AlertWebhook`** gets a POST for every alert that fires or resolves, on every target with
  thresholds (a target without `ThreshFor[]` fires at its first broken poll). Three attempts
  (now, after 2 s, after 10 s) on a network error, a 5xx or a 429; no redirects.
- **`AlertMailUser`** and **`AlertMailPassword`** make the threshold mail log in, over STARTTLS
  that must succeed (port 587 unless `ThreshMailServer` names one). Without them, mail goes as
  MRTG sends it: plain SMTP, port 25.
- Mail and webhooks trust the system's certificate authorities; set `SSL_CERT_FILE` to use a
  file of your own instead.
- Alert state is kept in `<config>.alerts` beside the `.ok` file, so a reload or a restart
  neither repeats nor loses a notice. A missed poll changes nothing.

The webhook body:

```json
{
  "event": "threshold", "state": "firing", "reason": null,
  "target": "core-uplink", "title": "Core uplink", "directory": "core",
  "direction": "in", "bound": "max", "threshold": "80%",
  "threshold_rate": 100000.0, "rate": 107750.0, "percent": 86.2,
  "polls": 3, "since": 1760940000, "time": 1760940600,
  "description": null, "linkscope": "0.1.0"
}
```

`state` is `firing` or `resolved`; a resolved alert's `reason` is `cleared` (back within the
bound) or `removed` (its threshold left the configuration). `polls` counts the broken polls
in a row, up to the one that fired it or cleared it. Rates are per second, bytes for interface
counters. Each request carries `X-LinkScope-Timestamp` (Unix time) and
`X-LinkScope-Signature: sha256=<hex>`, the HMAC-SHA256 of the timestamp, a full stop and the
body, keyed with `AlertWebhookSecret`. To check it:

```python
import hashlib, hmac, time

def verify(secret: bytes, timestamp: str, body: bytes, signature: str) -> bool:
    want = "sha256=" + hmac.new(secret, timestamp.encode() + b"." + body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(want, signature) and abs(time.time() - int(timestamp)) < 300
```

The webhook URL, its secret and the mail password never appear in the UI, the API or
`/metrics`.

## Verification

| What | Result |
|---|---|
| Config parser vs MRTG's Perl (`readcfg`, `cfgcheck`, `targparser`) | identical on 1,500 random configs, a corpus and MRTG's 26 contrib configs |
| Rate engine vs C `rateup` | byte-identical logs over 50,000 random updates; 2,000 corrupted logs |
| End to end vs MRTG (`testenv/parity.sh`) | 18 targets, 6 cycles, every `.log` byte-identical |
| SNMP interop | net-snmp `snmpd`: v1, v2c, v3 authPriv SHA/AES, SHA-256/AES, authNoPriv MD5 |
| cfgmaker vs MRTG's cfgmaker | identical output for a v2c device |

Run them yourself:

```sh
cargo test --workspace                 # unit, differential and robustness tests
testenv/start.sh                       # snmpd + simulated devices on 127.0.0.1
LINKSCOPE=target/release/linkscope testenv/parity.sh
LINKSCOPE=target/release/linkscope testenv/bench.sh
```

The differential tests need the MRTG sources (Perl) and a compiled `rateup`;
they skip cleanly when those are missing.

## Performance

One polling cycle, same targets, same simulated agents (snmpsim), median of
three runs on a 2-vCPU VM. CPU includes child processes (MRTG's `rateup`).

| Targets | Implementation | Wall time | CPU time | Peak memory |
|---|---|---|---|---|
| 1,200 | MRTG 2.17.10 | 10.6 s | 8.6 s | 36 MB |
| 1,200 | MRTG, `Forks: 4` | 10.3 s | 9.0 s | 36 MB |
| 1,200 | **LinkScope** | **2.3 s** | **0.4 s** | **20 MB** |
| 10,000 | MRTG 2.17.10 | 117.6 s | 79.9 s | 110 MB |
| 10,000 | MRTG, `Forks: 4` | 103.4 s | 88.3 s | 108 MB |
| 10,000 | **LinkScope** | **6.6 s** | **3.2 s** | **85 MB** |

At 10,000 targets LinkScope uses 25 times less CPU and finishes a cycle 16 to
18 times sooner. Most of its remaining time is spent waiting for the
simulated agents (a Python program) and on a single `syncfs` per cycle that
makes every updated log durable. MRTG's time is mostly `rateup` processes
drawing PNG graphs; LinkScope draws graphs in the browser, only when viewed.

## Compatibility

See [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) for the complete list of
differences. In short: Perl-only features (conversion subroutines,
cfgmaker templates) are not supported, `LogFormat: rrdtool` is not
supported, and no HTML/PNG files are written because the web UI replaces
them.

## Layout

| Crate | Contents |
|---|---|
| `crates/config` | Config reader, validator, Target compiler, safe evaluator |
| `crates/rate` | Port of `rateup`: log format, rate math, consolidation, graph statistics |
| `crates/linkscope` | SNMP client, poller, cycle engine, thresholds, daemon, cfgmaker, CLI |
| `crates/web` | JSON API and embedded React UI (`crates/web/ui`) |
| `testenv` | Test agents, parity and benchmark scripts |

## License and credits

LinkScope is free software under the GNU General Public License, version 2
or (at your option) any later version. See [LICENSE](LICENSE).

LinkScope is derived from MRTG 2.17.10, Copyright (C) Tobias Oetiker and the
MRTG contributors, licensed GPL-2.0-or-later. The configuration grammar, the
`.log` format, the rate engine (a port of `rateup`) and cfgmaker's logic come
from MRTG; the code is a rewrite in Rust by Rinjani Analytics.
[NOTICE](NOTICE) lists what was derived and what changed.
