# LinkScope plan

What LinkScope adds after 0.1.0 (the MRTG-compatible release), build by build. Each build closes
when its gate passes with numbers recorded in its PR, and its box is ticked here in the same PR.

## Ground rules

- **MRTG compatibility holds.** An existing `mrtg.cfg` keeps working unchanged, and every `.log`
  stays byte-identical to MRTG 2.17.10 (`testenv/parity.sh`). New behavior comes only from new
  keywords or new endpoints, off until configured.
- **Secrets never leave.** Communities, SNMPv3 passwords and command lines stay out of every new
  output (metrics, alerts, webhooks), as they already are out of the web API
  (`crates/web/tests/no_secrets.rs` covers each new output too).
- **The config stays a trusted file.** Nothing here edits `mrtg.cfg` from the network.
- US English, no em dashes, in code, docs and commit messages.

## Free and paid (AK, 2026-10-20)

LinkScope is GPL-2.0-or-later because it is derived from MRTG; Rinjani Analytics cannot relicense
the derived parts, and anything built into the LinkScope program is GPL too. So:

- **Free, GPL, this repository:** builds 1 to 3 below.
- **Paid, a separate program** (closed, sold as a license or run as SaaS): it reads LinkScope only
  through its public interfaces (the JSON API, `/metrics`, the `.log` files) and is never linked
  into it. Candidates: 95th percentile billing on interface counters (what transit and IX
  invoices use, beside PathScope's flow-based percentiles), errors and discards analysis,
  capacity forecasts, multi-site views, sign-in for the UI, packaging and support.
- Counters the paid program needs that only a poll can get (errors, discards) are polled by
  LinkScope itself, in the free program; the paid program adds the analysis.

## Builds

1. **Metrics for Prometheus.** `GET /metrics` on the web listener (`WebListen`), in Prometheus
   text format:
   - per target: the current in and out rates (bytes per second, as the UI shows them), the
     octet counters as last read (`counter` type, so `rate()` works, and so PathScope can compare
     exact windows in build 3), `MaxBytes`, the status (ok, warn, failed, stale) and the time of
     the last good poll;
   - labels: the target name, its `Directory[]`, the SNMP host and the interface (name and
     ifIndex as resolved), never a community or a password;
   - per cycle: duration, targets polled, failed, log write failures.

   Gate: on the lab (the CHR, JKT-MT01), Prometheus scrapes LinkScope for an hour: every rate
   equals the API's within rounding, every counter equals the CHR's own `rx-byte`/`tx-byte`
   read over its REST API at the same moment (within one poll's traffic), no secret in the
   output (the secrets test extended to `/metrics`), `parity.sh` still byte-identical.
2. **Alerts that reach people.** MRTG's `ThreshProg*`, `ThreshMail*` and `ThreshDir` behave as
   they do now. New, off until set:
   - `AlertWebhook: <url>`: a JSON body per change (target, direction, bound, rate, state
     firing or resolved, time), signed with HMAC-SHA256 (`AlertWebhookSecret:`);
   - mail over STARTTLS with a login (`ThreshMailServer` gains a port, `AlertMailUser:`,
     `AlertMailPassword:`), since most mail servers no longer take plain SMTP;
   - `ThreshFor[x]: <polls>`: a bound must stay broken this many polls before it fires;
   - a resolved notice when it clears, and the state in the API and the UI (a target over its
     threshold shows as warn, with which bound).

   Gate: on the lab, a `ThreshMaxI` of 30 % on ether2 (it carries about 40 %): fires after
   `ThreshFor` polls, one webhook (signature verified by the receiver) and one mail in the lab's
   Mailpit; the overview shows it as warn; the bound raised to 60 %: one resolved webhook and
   mail; a target without the new keywords behaves exactly as MRTG (`parity.sh`, threshold state
   files unchanged).
3. **Flow check, in PathScope (AK, 2026-10-20).** PathScope compares, per router interface, the
   bytes its flows saw (scaled by sampling) with LinkScope's octet counters for the same windows,
   and says when they disagree: a wrong sampling rate, an interface not exporting, a router
   dropping flow records. LinkScope's part is build 1's counters and labels; the rest is a
   PathScope change (a LinkScope connector with its `/metrics` URL, each target matched to a
   device and ifIndex, the result on the device's screen and in `pilot-check`), and a PathScope
   contract change, which needs AK's OK in that repo's plan.

   Gate (on the PathScope lab): the CHR's two ports within 2 % over one hour (gate 3 of
   PathScope's Phase 6 measured -0.27 % over 5 minutes); a planted fault (the device's sampling
   set to 1 instead of 10) flagged within two windows and cleared after it is set back.

- [ ] Build 1: `/metrics`, rates and counters matching the router.
- [ ] Build 2: webhook and mail alerts, fired and resolved once each.
- [ ] Build 3: flows and counters compared in PathScope, a planted fault named.

## Open decisions

- The paid program's name, and whether it starts as a license, SaaS or both.
- A review of the free and paid split by someone who knows GPL licensing, before anything is sold.
- When this repository goes public (it is private now).
- `/metrics` on the UI's listener (simplest) or a listener of its own (`MetricsListen:`), for
  installs that expose the UI and the metrics to different networks.
- Which PathScope phase build 3 belongs to: PathScope's Phase 6 (pilot readiness) is closing,
  and its rules start no next phase early.
