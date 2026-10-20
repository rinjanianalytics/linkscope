#!/usr/bin/env python3
"""Build 1's gate (docs/PLAN.md): `/metrics` against a real router and a real Prometheus.

    testenv/gate-metrics.py --cfg lab/mrtg.cfg --minutes 60 \\
        --chr http://10.10.0.72/rest            # a MikroTik's REST API (RouterOS 7)

For each poll in the window it reads, right after the poll, the router's own interface byte
counters (REST: `rx-byte`, `tx-byte`) and LinkScope's `/metrics`, and checks:

- every counter equals the router's, less at most the traffic since the poll (a 32-bit counter
  is compared modulo 2^32);
- every rate equals the JSON API's `cur_in`/`cur_out` for the same target;
- no community from the config appears anywhere in `/metrics`.

A Prometheus (the `prom/prometheus` image, host network, `--prom-port`) scrapes LinkScope every
15 s throughout; at the end: the scrape was up in every sample, and `rate()` over the counters
agrees with LinkScope's own rates within 5 %. The router's password comes from
`LINKSCOPE_GATE_CHR_PASSWORD` (default: the PathScope lab's).

Writes target/gate-metrics/result.json. Exit 0 when every check passed.
"""

import argparse
import base64
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "gate-metrics"
PROM_IMAGE = "prom/prometheus:v3.15.0"
ok = True


def check(cond, what):
    global ok
    ok &= bool(cond)
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    return cond


def get(url, auth=None, timeout=15):
    req = urllib.request.Request(url)
    if auth:
        req.add_header("Authorization", "Basic " + base64.b64encode(auth.encode()).decode())
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read().decode()


LINE = re.compile(r'^([a-z_]+)\{(.*)\} (\S+)$')
LABEL = re.compile(r'([a-z_]+)="((?:[^"\\]|\\.)*)"')


def parse(text):
    """The samples of a text exposition: [(name, {label: value}, float)]."""
    out = []
    for line in text.splitlines():
        m = LINE.match(line)
        if m:
            out.append((m[1], dict(LABEL.findall(m[2])), float(m[3])))
    return out


def communities(cfg):
    """Every community written in the config's Target lines (to look for leaks)."""
    found = set()
    for line in pathlib.Path(cfg).read_text().splitlines():
        if line.startswith("Target["):
            for m in re.finditer(r":([^:@\s`]+)@", line):
                if len(m[1]) >= 3:
                    found.add(m[1])
    return found


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cfg", required=True)
    ap.add_argument("--linkscope", default="http://127.0.0.1:8090")
    ap.add_argument("--chr", default="http://10.10.0.72/rest")
    ap.add_argument("--minutes", type=float, default=60)
    ap.add_argument("--prom-port", type=int, default=19090)
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    auth = "admin:" + os.environ.get("LINKSCOPE_GATE_CHR_PASSWORD", "lab-chr-admin")
    secrets = communities(args.cfg)
    host = urllib.parse.urlparse(args.linkscope)
    results = {"polls": [], "prometheus": {}}

    # A Prometheus of the gate's own, scraping LinkScope every 15 s.
    prom_dir = tempfile.mkdtemp(prefix="gate-metrics-prom-")
    pathlib.Path(prom_dir, "prometheus.yml").write_text(
        "global: { scrape_interval: 15s }\n"
        f"scrape_configs:\n  - job_name: linkscope\n    static_configs: [{{ targets: ['{host.netloc}'] }}]\n")
    os.chmod(prom_dir, 0o755)
    name = "gate-metrics-prometheus"
    subprocess.run(["docker", "rm", "-f", name], capture_output=True)
    subprocess.run(["docker", "run", "-d", "--name", name, "--network", "host", "-v", f"{prom_dir}:/etc/prometheus:ro",
                    PROM_IMAGE, "--config.file=/etc/prometheus/prometheus.yml",
                    f"--web.listen-address=127.0.0.1:{args.prom_port}", "--storage.tsdb.path=/prometheus"],
                   check=True, capture_output=True)
    prom = f"http://127.0.0.1:{args.prom_port}"
    started = time.time()
    try:
        end = started + args.minutes * 60
        last_seen = None
        while time.time() < end:
            targets = json.loads(get(args.linkscope + "/api/targets"))
            polls = {t["last_poll"] for t in targets if t["last_poll"]}
            newest = max(polls) if polls else None
            if newest is None or newest == last_seen:
                time.sleep(5)
                continue
            last_seen = newest
            # Right after the poll: the router's counters, then LinkScope's.
            t_router = time.time()
            ifs = {i["name"]: i for i in json.loads(get(args.chr + "/interface", auth))}
            text = get(args.linkscope + "/metrics")
            samples = parse(text)
            leaks = sorted(s for s in secrets if s in text)
            check(not leaks, f"poll {time.strftime('%H:%M:%S', time.localtime(newest))}: no community in /metrics"
                  + (f" (found {len(leaks)})" if leaks else ""))
            rates = {(n, l["target"], l["direction"]): v for n, l, v in samples if n == "linkscope_target_rate"}
            for t in targets:
                for d in ("in", "out"):
                    api_v = t["cur_" + d]
                    m_v = rates.get(("linkscope_target_rate", t["name"], d))
                    if api_v is not None:
                        check(m_v is not None and abs(m_v - api_v) <= 1e-9 * max(1.0, abs(api_v)),
                              f"  {t['name']} {d}: /metrics rate {m_v} equals the API's {api_v}")
            row = {"poll": newest, "router_read": t_router, "targets": {}}
            for n, l, v in samples:
                if n != "linkscope_target_counter_total" or not l.get("interface", "").startswith("Name:"):
                    continue
                ifname = l["interface"].split(":", 1)[1]
                i = ifs.get(ifname)
                if not i:
                    check(False, f"  {l['target']}: the router has no interface {ifname}")
                    continue
                router = int(i["rx-byte" if l["direction"] == "in" else "tx-byte"])
                wide = v >= 2 ** 32 or router < 2 ** 32
                expect = router if wide else router % 2 ** 32
                behind = (expect - v) % (2 ** 64 if wide else 2 ** 32)
                rate = rates.get(("linkscope_target_rate", l["target"], l["direction"])) or 0.0
                allow = rate * (t_router - newest) * 2 + 65536
                row["targets"][f"{l['target']} {l['direction']}"] = {"linkscope": v, "router": router, "behind": behind,
                                                                      "allowed": round(allow), "64bit": wide}
                check(behind <= allow, f"  {l['target']} {l['direction']}: counter {int(v)}, router {expect}"
                      f" ({'64' if wide else '32'}-bit), {behind} bytes behind, at most {allow:.0f} allowed")
            results["polls"].append(row)
        # Prometheus over the whole window.
        span = max(1, int((time.time() - started) // 60))

        def q(expr):
            r = json.loads(get(prom + "/api/v1/query?" + urllib.parse.urlencode({"query": expr})))
            return r["data"]["result"]

        ups = q(f'count_over_time(up{{job="linkscope"}}[{span}m])')
        downs = q(f'count_over_time((up{{job="linkscope"}} == 0)[{span}m:15s])')
        n_up = int(float(ups[0]["value"][1])) if ups else 0
        n_down = int(float(downs[0]["value"][1])) if downs else 0
        results["prometheus"]["scrapes"] = n_up
        results["prometheus"]["down"] = n_down
        check(n_up >= span * 3 and n_down == 0, f"Prometheus scraped /metrics {n_up} times in {span} min, never down")
        # The counter moves only at a poll, so rate() over a window would depend on where the
        # window cuts the steps. Compare across exact polls instead: what the counter grew from
        # the first poll Prometheus saw to the last, over the time between them, against the
        # mean of LinkScope's own rates for those polls.
        def by_key(expr):
            return {(s["metric"]["target"], s["metric"]["direction"]): float(s["value"][1]) for s in q(expr)}

        def by_target(expr):
            return {s["metric"]["target"]: float(s["value"][1]) for s in q(expr)}

        grew = by_key(f"max_over_time(linkscope_target_counter_total[{span}m]) - min_over_time(linkscope_target_counter_total[{span}m])")
        polls = by_target(f"max_over_time(linkscope_target_last_poll_time_seconds[{span}m]) - min_over_time(linkscope_target_last_poll_time_seconds[{span}m])")
        own = by_key(f"avg_over_time(linkscope_target_rate[{span}m])")
        results["prometheus"]["rates"] = {}
        for k, v in sorted(own.items()):
            secs = polls.get(k[0]) or 0
            p = grew[k] / secs if k in grew and secs else None
            results["prometheus"]["rates"][f"{k[0]} {k[1]}"] = {"from_counters": p, "linkscope": v, "seconds": secs}
            # Below 1 kB/s both are noise around zero: compare absolute bytes there.
            close = p is not None and (abs(p - v) <= 0.05 * v if v >= 1000 else abs(p - v) <= 50)
            check(close, f"{k[0]} {k[1]}: {p if p is None else round(p, 1)} B/s from the counters in Prometheus "
                         f"over {secs / 60:.0f} min, LinkScope's rates {round(v, 1)} B/s")
    finally:
        subprocess.run(["docker", "rm", "-f", name], capture_output=True)
        results["ok"] = ok
        (OUT / "result.json").write_text(json.dumps(results, indent=1))
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
