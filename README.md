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
    --output /etc/mrtg/mrtg.cfg public@router1 public@switch2::::2
# SNMPv3
linkscope cfgmaker --username mrtg --authprotocol sha --authpassword '...' \
    --privprotocol aes --privpassword '...' router1::::3
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

New config keywords: `WebListen: 127.0.0.1:8080`, `ExecShell: yes`.

## Web UI

The overview lists every target with its status, current rates and a day
sparkline, filterable and grouped by `Directory[]`. Each target page shows
the day, week, month and year graphs with MRTG's Max/Average/Current figures,
honouring `Options[]` (bits, growright, logscale, perminute, ...), `kMG`,
`Kilo`, `Factor`, legends, `WithPeak`, `Unscaled` and `Suppress`. The UI is
read-only, embedded in the binary, and needs no internet access.

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
| `crates/mrtg` | SNMP client, poller, cycle engine, thresholds, daemon, cfgmaker, CLI |
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
