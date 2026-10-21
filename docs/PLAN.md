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
   Done 2026-10-20 (`testenv/gate-metrics.py --minutes 60`, 11:50 to 12:50 WIB, the lab's CHR on
   SNMPv2c): 13 polls; after each of the 12 caught live, the router's REST counters read 0.6 to
   5.1 s later, LinkScope's counters behind them by at most 522 kB on the busy direction (10 s of
   its mean 52 kB/s; the lab's stream comes in bursts of up to 390 kB per 2.5 s) and 1 kB on the
   quiet one; every `/metrics` rate equal to the API's; no community in any scrape. A Prometheus
   (v3.15.0) scraped 240 times in 60 min, never down; the counters' growth over the hour against
   LinkScope's own rates: 52,144 and 52,126 B/s (0.04 %), 52,141 and 52,123 B/s, 2.8 and 2.8,
   80.1 and 80.3. 85 workspace tests and the new ones pass; clippy clean on the changed crates.
   Not run: `parity.sh` (MRTG 2.17.10 with its compiled `rateup`, snmpsim and faketime are not
   on this host); build 1 reads the logs and writes nothing to them.
   Found:
   - `cfgmaker` without a version writes SNMPv1 targets, as MRTG's does: 32-bit counters, which
     wrap every 34 s at 1 Gbit/s. The README now says to use `:::::2` for interfaces.
   - The gate's first allowance (twice the traffic since the poll) was tighter than the plan's
     gate and failed on bursts; it is now the traffic since the poll plus 30 s of it.
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
   Done 2026-10-20 (`testenv/gate-alerts.py`, 14:06 to 14:31 WIB, 6 polls, 22 checks, all
   passed). A LinkScope of the gate's own polled the CHR (the lab's stayed as it was). The mail
   went to a Mailpit of the gate's own, from the image the lab's runs, because the lab's takes
   plain SMTP only; it required STARTTLS (a CA made for the run, trusted through
   `SSL_CERT_FILE`) and a login, and refused a plain test mail. ether2 inbound at 41.7 % against
   `ThreshMaxI` 30 % with `ThreshFor` 2:
   - first broken poll: the bound waiting (1 of 2), the target ok, nothing sent;
   - second: one webhook (signature valid, `polls` 2, `since` the first broken poll), one
     BROKEN mail, the `ThreshDir` state file, the target warn with its bound, the overview
     counting it; one more poll repeated nothing;
   - the bound raised to 60 % in the config: at the next poll one resolved webhook (`cleared`),
     one UN-BROKEN mail, the state file removed, the target ok.

   The control, ether1 outbound at 41.8 % with MRTG's keywords only (`ThreshMaxO` 30 %,
   `ThreshProgO`, `ThreshProgOKO`, `ThreshDir`): the program ran once at its first broken poll
   with MRTG's arguments (`10.10.0.72_ether1 30% 41.8 No Description`), its state file
   appeared, nothing repeated; its webhook fired once at that poll. No webhook token, key or
   mail password in the API, `/metrics` or the log; no warning or error in the log.
   `crates/linkscope/tests/thresh_mrtg.rs` pins MRTG's programs, mails and state files over five
   polls (without `ThreshDir`, with `ThreshHyst`, with `ThreshDir`), recorded before the change
   and unchanged after it.
   Not run: `parity.sh` (no MRTG, `rateup`, snmpsim or faketime on this host); the log files are
   written by code build 2 did not touch.
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
   Changed in PathScope's plan (AK, 2026-10-20): flowd uses the sampling a router announces, and
   the CHR announces its own, so a wrong sampling in Settings changes nothing on the lab. The
   planted faults are the CHR exporting from no interface and 30 % of its IPFIX dropped; the
   wrong-sampling verdict is covered by tests.

- [x] Build 1: `/metrics`, rates and counters matching the router.
      2026-10-20: 13 polls in an hour, counters within 10 s of traffic of the router's, rates
      equal to the API's, Prometheus 240 scrapes never down, no community in any output.
- [x] Build 2: webhook and mail alerts, fired and resolved once each.
      2026-10-20: fired after `ThreshFor` 2 polls and resolved once each, by signed webhook
      and by mail over STARTTLS with a login; the control target behaved as MRTG.
- [x] Build 3: flows and counters compared in PathScope, a planted fault named.
      2026-10-20 (PathScope Phase 7 build 1): flows within -0.43 % and -0.42 % of the counters
      over an hour on the CHR's two busy directions; no interface exported named after 20 min,
      30 % of IPFIX dropped named as lost (-35.6 %) after 19 min, both cleared within 21 min.

## The manual (AK, 2026-10-21)

LinkScope had no documentation beyond the README. AK chose a manual served by LinkScope itself,
in English and Bahasa.

- **Where:** `/docs` on the web listener (a **Docs** link in the header). It matches the
  installed version and works without internet access.
- **Source:** Markdown in `docs/manual/en/` and `docs/manual/id/`, in mdBook's layout
  (`SUMMARY.md` lists the pages), so the pages read on GitHub and a public site later is an
  mdBook build of the same files.
- **Serving:** `crates/web/src/docs.rs` embeds the pages, renders them with `pulldown-cmark` and
  sanitizes them with `ammonia` like every other HTML the API gives (`GET /api/docs`,
  `GET /api/docs/{lang}/{page}`); the UI shows them through `HtmlContent`, with no JavaScript
  dependency added.
- **Pages (13 in each language):** introduction, install and run, moving from MRTG, command
  line, configuration, cfgmaker, web UI, Prometheus metrics, alerts, JSON API, security,
  compatibility with MRTG, troubleshooting. Each fact was taken from the code; what the code did
  not confirm was left out.
- **Keeping it whole:** a change to a keyword, an option, an endpoint or a metric updates the
  manual in both languages in the same PR. `crates/web/tests/docs.rs` fails when a page is
  missing in a language, the order differs, a page has an em dash, or a link between pages leads
  to no page or no heading.

Found while writing it:
- The README's cfgmaker examples set the backoff, not the SNMP version (`::::2`); fixed in #5
  (`:::::2`, `:::::3`).
- `GET /api/targets/{name}` returned the threshold programs' paths and the threshold mail
  address, on a UI without a login. Fixed in #6: the API says only whether they are set, and
  `no_secrets.rs` checks both (it fails on the old code).
- cfgmaker without `--ifref` refers to interfaces by ifName, else the ifIndex. That is MRTG
  2.17.10's behavior too (its code and `--help`); its longer manual page says `nr`, which is out
  of date. No change; the manual says so.

- [x] The manual at `/docs`, in English and Bahasa.
      2026-10-21 (#5, #6): 13 pages in each language; `cargo test --workspace` 87 pass, the
      manual's tests among them. In Google Chrome 154 against the built binary: the Docs link, a
      link between pages, `/docs/en/alerts#webhook` landing on its heading, the EN/ID switch on
      the same page, a 390 px phone with an Indonesian browser opening the Bahasa pages, the
      missing-page message; no sideways scroll and no errors, light and dark. On the lab since
      2026-10-21 11:13 WIB (`linkscope-lab` on `main`, a169b43), over the tailnet at port 8443.

## Security assessment before going public (AK, 2026-10-21)

The repository is about to go public, and AI-driven tools now read source, fuzz parsers and probe
listeners at scale. AK asked for hidden bugs to be found before they do.

- **Method:** five reviews, one per attacker position: anyone who reaches the web UI, a polled
  device or a spoofed reply, whoever influences the configuration, a local user, and the
  outbound alerts. Also `cargo audit`, `npm audit`, and live tests against a throwaway instance.
  The report stays local, not in this repository.
- **Found:** 3 high, 10 medium, and a list of low and informational items. The main defenses
  held:
  - no shell without `ExecShell`;
  - no path traversal or XSS from the web;
  - no mail downgrade with a login;
  - a fail-closed privilege drop;
  - BER decoding bounds-checked.
- **Fixed** in #8 (the highs and RUSTSEC-2026-0382) and #9 (the mediums). `docs/SECURITY.md`
  lists each one, who could do it and what LinkScope does now.
- **Behavior that changed:**
  - The UI answers only to IP addresses, `localhost` and the names in the new `WebHosts:`
    keyword.
  - A config is refused when any directory on the way to it lets another account rename it.
  - SNMPv3 settings that ask for less security than they seem to are an error.
  - `.ok` and `.alerts` are written owner only.
- **Pinned by tests:**
  - `tests/listener.rs`: header timeout.
  - `tests/host.rs`: Host check.
  - `tests/no_secrets.rs`: program paths and the mail address too.
  - The width, chain and SNMPv3 tests, and the trust rules per directory.

  A change that weakens one of these needs AK's OK.

- [x] The high and medium findings fixed, tested and on the lab.
      2026-10-21, before the fix:
      - About 1,100 half-sent HTTP requests used all 1,024 descriptors.
      - The UI stopped, and the next cycle opened no SNMP socket and wrote no log.
      - `--check` panicked on `Title[é]: x`.
      - The lab's `.ok` (with the community) was 0644.

      After the fix, under the same attack:
      - The daemon peaked at 268 descriptors, and `/healthz` answered in 5.1 s.
      - A cycle during a 330 s sustained attack had 0 log write failures.
      - `Title[é]` gives ordinary errors.

      Overall: `cargo test --workspace` 93 pass, clippy clean on the three crates. On the lab since
      12:04 WIB (`main`, f7d8923, `WebHosts:` set for the tailnet name):
      - first cycle 2 of 2 ok, no write failures;
      - a foreign Host gets 421, the tailnet `/docs` gets 200;
      - PathScope's network reads `/metrics` (200);
      - `.ok` and `.alerts` are 0600.

The low and informational findings (AK, 2026-10-21: fix them before going public) are fixed in
#11, one commit per area: SNMP and devices, alerts and threshold programs, logs and files and
configuration reading, the web UI and its build tools (Vite 8, Tailwind 4, `npm audit` clean).
`docs/SECURITY.md` has a row for each. Behavior that changed with them:
- a bare `ThreshProg*[]` name means the file in the working directory, not one found on `PATH`;
- an EXEC target's leftover processes are ended after it exits;
- a PID file of another live process no longer blocks startup (the lock decides);
- relative Include names resolve against the main config's directory only;
- the summary gives the configuration's file name, not its path.

- [x] The low findings fixed, tested and checked against the lab's data.
      2026-10-21 (#11): 114 workspace tests pass, clippy clean on the four crates. The release
      build's `--check` accepts the lab's config. Against a copy of the lab's config and logs,
      polling the real CHR: 2 of 2 targets ok, no write failures, `config_name` and no path, a
      foreign Host 421, an unknown API path 404, `no-store` on live data, `.ok` 0600, and the
      connection attack again at most 268 descriptors with `/healthz` in 5.2 s. The UI after
      Vite 8 and Tailwind 4 matches the old one in Chrome 154 (light, dark, desktop, phone).

Still open, by choice or for later:
- snmp2's own internals (a zero-length INTEGER shift, engine-time overflow, a non-constant-time
  HMAC): the open decision below.
- The version stays in the UI and `linkscope_info`: useful for monitoring, little to hide in an
  open program.
- `create_dirs` is not fully race-free between levels (it would need `mkdirat`); without `/proc`
  the `Directory[]` protection falls back to the by-name check.

## Open decisions

- The paid program's name, and whether it starts as a license, SaaS or both.
- A review of the free and paid split by someone who knows GPL licensing, before anything is sold.
- When this repository goes public (it is private now), and with it whether the manual also
  gets a public site (an mdBook build of `docs/manual`, for example on rinjanianalytics.com).
- `/metrics` on the UI's listener (simplest) or a listener of its own (`MetricsListen:`), for
  installs that expose the UI and the metrics to different networks.
- The snmp2 SNMPv3 issue (state changed before authentication): report it to its maintainer
  privately, patch a fork, or replace the library. LinkScope works around it by reopening the
  session.
