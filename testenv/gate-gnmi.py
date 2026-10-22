#!/usr/bin/env python3
"""Build 4's gate (docs/PLAN.md): gNMI and SNMP reading the same SR Linux ports for an hour.

    testenv/gnmi/up.sh                     # the two-router lab
    testenv/gate-gnmi.py --minutes 60 --linkscope target/gnmi-release/release/linkscope

Starts a LinkScope of its own (WorkDir target/gate-gnmi, the UI on 127.0.0.1:18091, polling every
--interval minutes) with these targets:

- srl1 ethernet-1/1 three ways: SNMP (64-bit counters), gNMI with OpenConfig, gNMI with SR Linux's
  own model; srl2 ethernet-1/2 two ways: SNMP and gNMI (srl2 has OpenConfig off);
- three that must fail and say why: a wrong password, an untrusted CA, a device that is not there.

After every poll it reads `/metrics` and checks:

- each gNMI counter equals the SNMP one of the same port, within the traffic of SR Linux's SNMP
  refresh (its agent updates counters about every 20 s, gNMI every second: 25 s of the port's
  traffic is allowed, plus 64 kB);
- the two gNMI reads of srl1 (OpenConfig and native) agree within 2 s of traffic;
- the failing targets carry their reason, and the cycle stays well inside its time budget;
- no password, community or credential file path in `/metrics`, the API or the log.

At the end: over the whole window, each port's rate from the gNMI counters against the SNMP ones,
within 1 %. Writes target/gate-gnmi/result.json. Exit 0 when every check passed.
"""

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "gate-gnmi"
LAB = ROOT / "testenv" / "gnmi" / "generated"
ok = True


def check(cond, what):
    global ok
    ok &= bool(cond)
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    return cond


def get(url, timeout=15):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read().decode()


LINE = re.compile(r'^([a-z_]+)(?:\{(.*)\})? (\S+)$')
LABEL = re.compile(r'([a-z_]+)="((?:[^"\\]|\\.)*)"')


def parse(text):
    out = []
    for line in text.splitlines():
        m = LINE.match(line)
        if m:
            out.append((m[1], dict(LABEL.findall(m[2] or "")), float(m[3])))
    return out


def private(path, text):
    path.write_text(text)
    path.chmod(0o600)


def config(work, port, interval):
    community = (LAB / "snmp.community").read_text().strip()
    shutil.copy(LAB / "ca.pem", work / "ca.pem")
    private(work / "pass", (LAB / "linkscope.pass").read_text())
    private(work / "wrong.pass", "not-the-password\n")
    # A CA that signed nothing on the routers.
    subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
                    "-subj", "/CN=gate other CA", "-keyout", str(work / "other-ca.key"),
                    "-out", str(work / "other-ca.pem")], check=True, capture_output=True)
    auth = f"user=linkscope password-file={work}/pass ca={work}/ca.pem"
    targets = {
        "s1_e11_snmp": (f"32768:{community}@10.20.0.11:::::2", None),
        "s1_e11_oc": ("gnmi:ethernet-1/1@10.20.0.11:57400", auth),
        "s1_e11_srl": ("gnmi:ethernet-1/1@10.20.0.11:57400", auth + " model=srl"),
        "s2_e12_snmp": (f"32770:{community}@10.20.0.12:::::2", None),
        "s2_e12_srl": ("gnmi:ethernet-1/2@10.20.0.12:57400", auth + " model=srl"),
        "bad_password": ("gnmi:ethernet-1/1@10.20.0.12:57400",
                         f"user=linkscope password-file={work}/wrong.pass ca={work}/ca.pem model=srl"),
        "bad_ca": ("gnmi:ethernet-1/2@10.20.0.11:57400",
                   f"user=linkscope password-file={work}/pass ca={work}/other-ca.pem"),
        "not_there": ("gnmi:ethernet-1/1@10.20.0.99:57400", auth),
    }
    lines = [f"WorkDir: {work}", f"WebListen: 127.0.0.1:{port}", f"Interval: {interval}", ""]
    for name, (target, opts) in targets.items():
        lines.append(f"Target[{name}]: {target}")
        if opts:
            lines.append(f"GnmiOptions[{name}]: {opts}")
        lines += [f"MaxBytes[{name}]: 3125000000", f"Title[{name}]: {name}", ""]
    cfg = work / "gate.cfg"
    private(cfg, "\n".join(lines))
    secrets = {community, (LAB / "linkscope.pass").read_text().strip(), "not-the-password",
               str(work / "pass"), str(work / "wrong.pass")}
    return cfg, secrets


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--linkscope", default=str(ROOT / "target" / "debug" / "linkscope"))
    ap.add_argument("--minutes", type=float, default=60)
    ap.add_argument("--interval", type=int, default=5, help="MRTG Interval, in minutes (at least 5)")
    ap.add_argument("--port", type=int, default=18091)
    args = ap.parse_args()

    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    work = OUT / "work"
    work.mkdir()
    cfg, secrets = config(work, args.port, args.interval)
    base = f"http://127.0.0.1:{args.port}"
    log = OUT / "linkscope.log"
    proc = subprocess.Popen([args.linkscope, "--daemon", "--foreground", "--logging", str(log), str(cfg)],
                            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    results = {"started": time.time(), "polls": [], "binary": args.linkscope}
    try:
        for _ in range(60):
            if proc.poll() is not None:
                print(log.read_text() if log.exists() else "", file=sys.stderr)
                raise SystemExit(f"linkscope exited with {proc.returncode}; check: {args.linkscope} --check {cfg}")
            try:
                get(base + "/api/summary")
                break
            except OSError:
                time.sleep(1)
        started = time.time()
        seen = 0
        first, last = None, None
        while time.time() - started < args.minutes * 60 + 30:
            time.sleep(3)
            summary = json.loads(get(base + "/api/summary"))
            if summary["cycles"] <= seen:
                continue
            seen = summary["cycles"]
            text = get(base + "/metrics")
            s = parse(text)
            counter = {(l["target"], l["direction"]): v for n, l, v in s if n == "linkscope_target_counter_total"}
            polled = {l["target"]: v for n, l, v in s if n == "linkscope_target_last_poll_time_seconds"}
            duration = next((v for n, l, v in s if n == "linkscope_last_cycle_duration_seconds"), None)
            when = time.strftime("%H:%M:%S")
            print(f"poll {seen} at {when}, cycle {duration:.2f} s", flush=True)
            row = {"cycle": seen, "at": time.time(), "duration": duration, "counters": {}, "errors": {}}
            for (t, d), v in counter.items():
                row["counters"][f"{t} {d}"] = v
            if last is not None:
                # The port's rate over the last interval, from the SNMP counters.
                span = max(1.0, polled.get("s1_e11_snmp", 0) - last["polled"].get("s1_e11_snmp", 0))
                for snmp, others in (("s1_e11_snmp", ["s1_e11_oc", "s1_e11_srl"]), ("s2_e12_snmp", ["s2_e12_srl"])):
                    for d in ("in", "out"):
                        a = counter.get((snmp, d))
                        prev = last["counters"].get(f"{snmp} {d}")
                        rate = (a - prev) / span if a is not None and prev is not None else 0
                        for o in others:
                            b = counter.get((o, d))
                            allow = rate * 25 + 65536
                            diff = None if a is None or b is None else b - a
                            check(diff is not None and abs(diff) <= allow,
                                  f"  {o} {d}: gNMI {b}, SNMP {a}, {diff} apart ({diff / rate if rate and diff is not None else 0:.1f} s"
                                  f" of traffic), at most {allow:.0f}")
                for d in ("in", "out"):
                    a, b = counter.get(("s1_e11_oc", d)), counter.get(("s1_e11_srl", d))
                    prev = last["counters"].get(f"s1_e11_oc {d}")
                    rate = (a - prev) / span if a is not None and prev is not None else 0
                    allow = rate * 2 + 4096
                    check(a is not None and b is not None and abs(a - b) <= allow,
                          f"  srl1 ethernet-1/1 {d}: OpenConfig {a}, native {b}, at most {allow:.0f} apart")
            for t, want in (("bad_password", "login refused"), ("bad_ca", "TLS"), ("not_there", "")):
                e = json.loads(get(base + f"/api/targets/{t}")).get("error") or ""
                row["errors"][t] = e
                check(e and want in e, f"  {t}: {e[:140]}")
            check(duration is not None and duration < 30, f"  the cycle took {duration} s (budget well above)")
            pages = [text, get(base + "/api/targets")] + [get(base + f"/api/targets/{t}") for t in
                                                          ("s1_e11_oc", "s2_e12_srl", "bad_password", "bad_ca")]
            logtext = log.read_text() if log.exists() else ""
            leaks = sorted(x for x in secrets if any(x in p for p in pages) or x in logtext)
            check(not leaks, f"  no secret or credential path in /metrics, the API or the log ({len(leaks)} found)")
            row["polled"] = polled
            results["polls"].append(row)
            first = first or row
            last = row
        # The whole window: rates from gNMI against SNMP.
        results["window"] = {}
        if first is not None and last is not first:
            for snmp, others in (("s1_e11_snmp", ["s1_e11_oc", "s1_e11_srl"]), ("s2_e12_snmp", ["s2_e12_srl"])):
                for d in ("in", "out"):
                    def rate(t):
                        secs = last["polled"][t] - first["polled"][t]
                        return (last["counters"][f"{t} {d}"] - first["counters"][f"{t} {d}"]) / secs, secs
                    r0, secs = rate(snmp)
                    for o in others:
                        r1, _ = rate(o)
                        dev = (r1 - r0) / r0 * 100 if r0 else 0.0
                        results["window"][f"{o} {d}"] = {"gnmi": r1, "snmp": r0, "seconds": secs, "percent": dev}
                        check(abs(dev) <= 1.0, f"{o} {d}: {r1 * 8 / 1000:.1f} kbit/s by gNMI, {r0 * 8 / 1000:.1f} by SNMP "
                                               f"over {secs / 60:.0f} min ({dev:+.3f} %)")
        else:
            check(False, "at least two polls")
    finally:
        proc.terminate()
        try:
            proc.wait(15)
        except subprocess.TimeoutExpired:
            proc.kill()
        results["ok"] = ok
        (OUT / "result.json").write_text(json.dumps(results, indent=1))
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
