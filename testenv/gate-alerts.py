#!/usr/bin/env python3
"""Build 2's gate (docs/PLAN.md): threshold alerts by webhook and mail, fired and resolved once.

    testenv/gate-alerts.py --cfg lab/mrtg.cfg           # a config polling the lab's CHR

It runs a LinkScope of its own (a copy of the config with its own WorkDir and web port), so the
lab's LinkScope is not touched, beside:

- a Mailpit (the image the lab's PathScope runs) that requires STARTTLS, with a certificate from a
  CA made for this run (LinkScope trusts it through SSL_CERT_FILE), and a login;
- a webhook receiver that checks every signature.

The alert target (default ether2, inbound about 42 % of 1 Mbit/s) gets ThreshMaxI 30 %,
ThreshFor 2 and a mail address. Checks: after one broken poll the API shows the bound waiting
and the target ok; after the second it fires: one signed webhook, one mail through STARTTLS
and login, `ThreshDir`'s state file, the target warn with its bound. One more poll repeats
nothing. Then the bound goes to 60 % in the config: at the next poll one resolved webhook, one
UN-BROKEN mail, the state file gone, the target ok.

The control target (default ether1, outbound about 42 %) has only MRTG's keywords: ThreshMaxO
30 %, ThreshProgO and ThreshProgOKO with ThreshDir. It must behave as MRTG: the program runs
once with MRTG's arguments at the first broken poll and the state file appears, then nothing.

Finally no secret of the alert settings (the webhook URL's token, its key, the mail password)
appears in the API, /metrics or LinkScope's log.

Writes target/gate-alerts/result.json. Exit 0 when every check passed.
"""

import argparse
import hashlib
import hmac
import http.server
import json
import os
import pathlib
import re
import secrets
import shutil
import signal
import smtplib
import subprocess
import sys
import threading
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "gate-alerts"
MAILPIT_IMAGE = "axllent/mailpit:v1.31.3"
MAILPIT = "gate-alerts-mailpit"
ok = True
log_lines = []


def check(cond, what):
    global ok
    ok &= bool(cond)
    line = ("ok   " if cond else "FAIL ") + what
    log_lines.append(line)
    print(line, flush=True)
    return cond


def note(what):
    print(f"{time.strftime('%H:%M:%S')} {what}", flush=True)


def get_json(url):
    with urllib.request.urlopen(url, timeout=15) as r:
        return json.loads(r.read())


def get_text(url):
    with urllib.request.urlopen(url, timeout=15) as r:
        return r.read().decode()


def make_tls(d):
    """A CA for this run, and a certificate it signs for localhost."""
    d.mkdir(parents=True)
    run = lambda *a: subprocess.run(a, check=True, capture_output=True, cwd=d)
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=LinkScope gate CA",
        "-keyout", "ca.key", "-out", "ca.pem", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign")
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", "server.key", "-out", "server.csr")
    (d / "ext").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n")
    run("openssl", "x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-days", "2",
        "-extfile", "ext", "-out", "server.pem")
    os.chmod(d / "server.key", 0o644)  # read by the container's user; a key for this run only


class Receiver(http.server.BaseHTTPRequestHandler):
    path_ok = ""
    key = b""
    events = []

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
        ts = self.headers.get("X-LinkScope-Timestamp", "")
        sig = self.headers.get("X-LinkScope-Signature", "")
        want = "sha256=" + hmac.new(self.key, f"{ts}.{body}".encode(), hashlib.sha256).hexdigest()
        valid = (self.path == self.path_ok and hmac.compare_digest(sig, want) and ts.isdigit()
                 and abs(time.time() - int(ts)) < 300)
        try:
            payload = json.loads(body)
        except ValueError:
            payload = None
        Receiver.events.append({"received": time.time(), "valid": valid, "payload": payload,
                                "event": self.headers.get("X-LinkScope-Event")})
        self.send_response(200 if valid else 401)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *a):
        pass


def mails():
    return get_json("http://127.0.0.1:19026/api/v1/messages").get("messages", [])


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cfg", required=True)
    ap.add_argument("--linkscope", default=str(ROOT / "target" / "release" / "linkscope"))
    ap.add_argument("--alert-target", default="10.10.0.72_ether2")
    ap.add_argument("--control-target", default="10.10.0.72_ether1")
    ap.add_argument("--port", type=int, default=18090)
    ap.add_argument("--max-polls", type=int, default=9)
    args = ap.parse_args()
    if OUT.exists():
        shutil.rmtree(OUT)
    for d in ("work", "thresh", "progs"):
        (OUT / d).mkdir(parents=True)
    make_tls(OUT / "tls")
    alert_t, control_t = args.alert_target, args.control_target
    results = {"polls": [], "webhooks": [], "mails": [], "control_calls": []}

    # Secrets for this run.
    token, key, mail_pw = secrets.token_hex(12), secrets.token_hex(16), secrets.token_hex(12)
    secret_list = [token, key, mail_pw]
    (OUT / "tls" / "auth").write_text(f"gate:{mail_pw}\n")
    os.chmod(OUT / "tls" / "auth", 0o644)

    # The control target's program: MRTG passes target, bound, current value and description.
    calls = OUT / "progs" / "calls.log"
    prog = OUT / "progs" / "record.sh"
    prog.write_text(f'#!/bin/sh\necho "$(date +%s) $0 $*" >> {calls}\n')
    prog.chmod(0o755)

    # The config: the lab's, with its own WorkDir and port, and the alert settings.
    src = pathlib.Path(args.cfg).read_text()
    src = re.sub(r"(?mi)^WorkDir:.*$", f"WorkDir: {OUT / 'work'}", src)
    src = re.sub(r"(?mi)^WebListen:.*$", f"WebListen: 127.0.0.1:{args.port}", src)
    for t in (alert_t, control_t):
        if f"Target[{t}]" not in src:
            sys.exit(f"{args.cfg} has no Target[{t}]")
    cfg = OUT / "gate.cfg"
    alert_cfg = (
        f"\n### gate-alerts\nThreshDir: {OUT / 'thresh'}\nThreshMailServer: localhost:19025\n"
        f"ThreshMailSender: linkscope-gate@example.com\nAlertMailUser: gate\nAlertMailPassword: {mail_pw}\n"
        f"AlertWebhook: http://127.0.0.1:19080/hook/{token}\nAlertWebhookSecret: {key}\n"
        f"ThreshMaxI[{alert_t}]: {{bound}}\nThreshFor[{alert_t}]: 2\nThreshDesc[{alert_t}]: gate: {alert_t} inbound\n"
        f"ThreshMailAddress[{alert_t}]: noc@example.com\n"
        f"ThreshMaxO[{control_t}]: 30%\nThreshProgO[{control_t}]: {prog}\nThreshProgOKO[{control_t}]: {prog}\n")
    cfg.write_text(src + alert_cfg.format(bound="30%"))
    os.chmod(cfg, 0o600)
    state_alert = OUT / "thresh" / f"gate.cfg.{alert_t}.threshmaxI"
    state_control = OUT / "thresh" / f"gate.cfg.{control_t}.threshmaxO"

    # Mailpit: STARTTLS and a login required.
    subprocess.run(["docker", "rm", "-f", MAILPIT], capture_output=True)
    subprocess.run(["docker", "run", "-d", "--name", MAILPIT, "-p", "127.0.0.1:19025:1025", "-p", "127.0.0.1:19026:8025",
                    "-v", f"{OUT / 'tls'}:/tls:ro", MAILPIT_IMAGE, "--smtp-tls-cert", "/tls/server.pem",
                    "--smtp-tls-key", "/tls/server.key", "--smtp-require-starttls", "--smtp-auth-file", "/tls/auth"],
                   check=True, capture_output=True)
    Receiver.path_ok, Receiver.key = f"/hook/{token}", key.encode()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 19080), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    proc = None
    try:
        for _ in range(30):
            try:
                mails()
                break
            except OSError:
                time.sleep(1)
        # The Mailpit refuses mail without STARTTLS, so a mail that arrives came through it.
        refused = False
        try:
            with smtplib.SMTP("127.0.0.1", 19025, timeout=10) as s:
                s.sendmail("x@example.com", ["noc@example.com"], "Subject: plain\r\n\r\nplain")
        except smtplib.SMTPException:
            refused = True
        check(refused and not mails(), "the gate's Mailpit refuses mail without STARTTLS and a login")

        env = dict(os.environ, SSL_CERT_FILE=str(OUT / "tls" / "ca.pem"), LINKSCOPE_LOG="info", NO_COLOR="1")
        logf = open(OUT / "linkscope.log", "w")
        proc = subprocess.Popen([args.linkscope, "--daemon", "--foreground", str(cfg)], stdout=logf, stderr=subprocess.STDOUT, env=env)
        api = f"http://127.0.0.1:{args.port}"
        note(f"LinkScope {proc.pid} on {api}; polls every 5 min")

        def wait_poll(last):
            """The next poll of the alert target (its last_poll changes), and every row then."""
            deadline = time.time() + 420
            while time.time() < deadline:
                if proc.poll() is not None:
                    raise SystemExit("LinkScope exited; see target/gate-alerts/linkscope.log")
                try:
                    rows = {r["name"]: r for r in get_json(api + "/api/targets")}
                except OSError:
                    rows = {}
                p = rows.get(alert_t, {}).get("last_poll")
                if p and p != last:
                    time.sleep(8)  # the cycle's actions: mail and webhooks
                    rows = {r["name"]: r for r in get_json(api + "/api/targets")}
                    return p, rows
                time.sleep(3)
            raise SystemExit("no poll in 7 minutes")

        def hooks(target, state=None):
            return [e for e in Receiver.events if e["payload"] and e["payload"]["target"] == target
                    and (state is None or e["payload"]["state"] == state)]

        def record(p, rows):
            a, c = rows[alert_t], rows[control_t]
            row = {"poll": p, "alert": {"status": a["status"], "alerts": a["alerts"], "cur_in": a["cur_in"]},
                   "control": {"status": c["status"], "alerts": c["alerts"], "cur_out": c["cur_out"]},
                   "webhooks": len(Receiver.events), "mails": len(mails()),
                   "control_calls": len(calls.read_text().splitlines()) if calls.exists() else 0}
            results["polls"].append(row)
            pct = a["alerts"][0]["percent"] if a["alerts"] else None
            note(f"poll {time.strftime('%H:%M:%S', time.localtime(p))}: {alert_t} {a['status']} "
                 f"{[(x['bound'], x['direction'], x['polls'], x['firing']) for x in a['alerts']]} ({pct} %), "
                 f"{control_t} {c['status']}, webhooks {row['webhooks']}, mails {row['mails']}, program runs {row['control_calls']}")
            return row

        # Phase 1: until the alert fires.
        last, first_broken, fired = None, None, None
        control_first, control_pct = None, None
        for _ in range(args.max_polls):
            last, rows = wait_poll(last)
            row = record(last, rows)
            al = rows[alert_t]["alerts"]
            if rows[control_t]["alerts"] and control_first is None:
                control_first, control_pct = last, rows[control_t]["alerts"][0]["percent"]
            if al and first_broken is None:
                first_broken = last
                check(al[0]["polls"] == 1 and not al[0]["firing"] and rows[alert_t]["status"] == "ok",
                      f"{alert_t}: one broken poll shows the bound waiting (1 of 2 polls), the target still ok")
                check(not hooks(alert_t) and not any("BROKEN" in m["Subject"] for m in mails()),
                      f"{alert_t}: nothing sent after one broken poll")
            if al and al[0]["firing"]:
                fired = last
                break
        if not check(fired is not None, f"{alert_t}: the alert fired"):
            raise SystemExit(1)

        a = rows[alert_t]
        check(a["status"] == "warn" and a["alerts"][0]["bound"] == "max" and a["alerts"][0]["direction"] == "in"
              and a["alerts"][0]["threshold"] == "30%", f"{alert_t}: the API shows warn, with the bound (max in 30%)")
        summary = get_json(api + "/api/summary")
        check(summary["targets_warn"] >= 1, f"the overview counts {summary['targets_warn']} warn")
        fh = hooks(alert_t, "firing")
        check(len(fh) == 1 and fh[0]["valid"], f"{alert_t}: one firing webhook, signature valid ({len(fh)} received)")
        if fh:
            p = fh[0]["payload"]
            check(p["polls"] == 2 and p["time"] == fired and p["since"] == first_broken and p["bound"] == "max"
                  and p["direction"] == "in" and p["threshold"] == "30%" and p["rate"] > p["threshold_rate"] == 37500
                  and fh[0]["event"] == "threshold",
                  f"{alert_t}: the webhook body: fired at the second broken poll, rate {p['rate']} B/s over {p['threshold_rate']}")
        m = [x for x in mails() if x["Subject"].startswith(f"[MRTG-TH] BROKEN {alert_t} thresh max i ( 30% (37500.0) vs ")]
        check(len(m) == 1 and len(mails()) == 1, f"{alert_t}: one BROKEN mail through STARTTLS and a login ({len(mails())} in the Mailpit)")
        check(state_alert.exists(), f"{alert_t}: ThreshDir state file created when it fired")

        # The control target, MRTG's way.
        cl = calls.read_text().splitlines() if calls.exists() else []
        results["control_calls"] = cl
        check(len(cl) == 1 and cl[0].split()[1:] == [str(prog), control_t, "30%", f"{control_pct}", "No", "Description"],
              f"{control_t}: ThreshProgO ran once with MRTG's arguments ({cl[0].split(' ', 2)[2] if cl else 'none'})")
        check(state_control.exists(), f"{control_t}: ThreshDir state file created at the first broken poll")
        ch = hooks(control_t, "firing")
        check(len(ch) == 1 and ch[0]["valid"] and ch[0]["payload"]["polls"] == 1 and ch[0]["payload"]["time"] == control_first,
              f"{control_t}: one firing webhook at its first broken poll (no ThreshFor)")

        # One more poll: nothing repeats.
        last, rows = wait_poll(last)
        record(last, rows)
        cl = calls.read_text().splitlines()
        check(len(hooks(alert_t)) == 1 and len(mails()) == 1 and len(cl) == 1 and len(hooks(control_t)) == 1
              and rows[alert_t]["status"] == "warn",
              "one more broken poll: no second webhook, mail or program run; still warn")

        # Phase 2: the bound raised to 60 %.
        cfg.write_text(src + alert_cfg.format(bound="60%"))
        note(f"ThreshMaxI[{alert_t}] raised to 60%")
        last, rows = wait_poll(last)
        record(last, rows)
        rh = hooks(alert_t, "resolved")
        check(len(rh) == 1 and rh[0]["valid"] and rh[0]["payload"]["reason"] == "cleared" and rh[0]["payload"]["time"] == last
              and rh[0]["payload"]["threshold"] == "60%", f"{alert_t}: one resolved webhook at the next poll, signature valid")
        um = [x for x in mails() if x["Subject"].startswith(f"[MRTG-TH] UN-BROKEN {alert_t} thresh max i ( 60% vs ")]
        check(len(um) == 1 and len(mails()) == 2, f"{alert_t}: one UN-BROKEN mail ({len(mails())} in the Mailpit)")
        check(rows[alert_t]["status"] == "ok" and not rows[alert_t]["alerts"], f"{alert_t}: ok again, no alert")
        check(not state_alert.exists(), f"{alert_t}: ThreshDir state file removed")
        check(len(calls.read_text().splitlines()) == 1 and state_control.exists() and len(hooks(control_t)) == 1,
              f"{control_t}: still broken, nothing repeated")
        check(all(e["valid"] for e in Receiver.events), f"every webhook signature valid ({len(Receiver.events)} received)")

        # Secrets.
        texts = {u: get_text(api + u) for u in ["/api/summary", "/api/targets", f"/api/targets/{alert_t}", f"/api/targets/{control_t}", "/metrics"]}
        texts["linkscope.log"] = re.sub(r"\x1b\[[0-9;]*m", "", (OUT / "linkscope.log").read_text())
        leaks = sorted({w for w, t in texts.items() for s in secret_list if s in t})
        check(not leaks, "no webhook token, signing key or mail password in the API, /metrics or the log"
              + (f" (found in {leaks})" if leaks else ""))
        errors = [l for l in texts["linkscope.log"].splitlines() if " WARN " in l or " ERROR " in l]
        check(not errors, f"no warnings or errors in LinkScope's log ({len(errors)})" + (f": {errors[:3]}" if errors else ""))
        results["mails"] = [{"subject": x["Subject"], "created": x["Created"]} for x in mails()]
        results["webhooks"] = Receiver.events
    finally:
        if proc and proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(30)
            except subprocess.TimeoutExpired:
                proc.kill()
        subprocess.run(["docker", "rm", "-f", MAILPIT], capture_output=True)
        server.shutdown()
        results["ok"] = ok
        results["checks"] = log_lines
        (OUT / "result.json").write_text(json.dumps(results, indent=1))
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
