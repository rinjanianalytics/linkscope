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

## Free and paid (2026-10-20)

LinkScope is GPL-2.0-or-later because it is derived from MRTG; Rinjani Analytics cannot relicense
the derived parts, and anything built into the LinkScope program is GPL too. So:

- **Free, GPL, this repository:** builds 1 to 3 below, and gNMI in builds 4 to 6.
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
3. **Flow check, in PathScope (2026-10-20).** PathScope compares, per router interface, the
   bytes its flows saw (scaled by sampling) with LinkScope's octet counters for the same windows,
   and says when they disagree: a wrong sampling rate, an interface not exporting, a router
   dropping flow records. LinkScope's part is build 1's counters and labels; the rest is a
   PathScope change (a LinkScope connector with its `/metrics` URL, each target matched to a
   device and ifIndex, the result on the device's screen and in `pilot-check`), and a PathScope
   contract change, which needs the owner's OK in that repo's plan.

   Gate (on the PathScope lab): the CHR's two ports within 2 % over one hour (gate 3 of
   PathScope's Phase 6 measured -0.27 % over 5 minutes); a planted fault (the device's sampling
   set to 1 instead of 10) flagged within two windows and cleared after it is set back.
   Changed in PathScope's plan (2026-10-20): flowd uses the sampling a router announces, and
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

## The manual (2026-10-21)

LinkScope had no documentation beyond the README. The owner chose a manual served by LinkScope itself,
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

## Security assessment before going public (2026-10-21)

The repository is about to go public, and AI-driven tools now read source, fuzz parsers and probe
listeners at scale. The owner asked for hidden bugs to be found before they do.

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

  A change that weakens one of these needs the owner's OK.

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

The low and informational findings (2026-10-21: fix them before going public) are fixed in
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

The SNMP library (2026-10-21: replace snmp2): replaced by `async-snmp` 0.18.1, pinned, client
features only. It authenticates an SNMPv3 reply before changing session state, compares HMACs in
constant time and denies `unsafe` code in its client; `strict_source` keeps replies from other
addresses out. Checked against net-snmp 5.9.4: the v3 tests (SHA and AES, MD5, SHA-256 and AES, a
wrong password, a wrong community) pass, and cfgmaker over v1, v2c, v3 authPriv and v3 authNoPriv
writes byte-identical configs with the old and the new library (49 interfaces each); a cycle polls
the lab's CHR (v2c) with counters matching the lab's own. AES-192 and AES-256 now work (with
`aes192c`, `aes256c` for Cisco's key extension); v3 passwords must have 8 characters.

Still open, by choice or for later:
- The version stays in the UI and `linkscope_info`: useful for monitoring, little to hide in an
  open program.
- `create_dirs` is not fully race-free between levels (it would need `mkdirat`); without `/proc`
  the `Directory[]` protection falls back to the by-name check.

## Docker (2026-10-21)

Most ISPs run Docker, so the default install is an image and a compose file; building from source
is the alternative. `Dockerfile` (Rust 1.99 to build, 1.92 until #34; Debian bookworm-slim to run, CA certificates
only, uid 10001, a `/healthz` health check) and `compose.yaml` (the config mounted read-only at
`/etc/linkscope/mrtg.cfg`, everything LinkScope writes in the `linkscope-data` volume, the UI on the
host's loopback, a read-only filesystem, no capabilities, `no-new-privileges`, an init for EXEC
targets' children). Docker's own settings may go in an optional `.env` (`.env.example`); SNMP stays
in `mrtg.cfg`. Compose pulls `ghcr.io/rinjanianalytics/linkscope`, or builds it when none is
published.

- [x] The image and compose file, tested.
      2026-10-21: the image builds (104 MB). With compose: a config owned by the host user is
      refused with "Permission denied" (the manual says how to fix it); owned `root:10001`, mode
      640, the container is healthy. `docker compose run cfgmaker` against a net-snmp agent writes a
      config, and the daemon on it polls (status ok, sysName and uptime read, counters in the logs
      and on `/metrics`); a foreign Host gets 421, the manual is served, the root filesystem is
      read-only, the account is uid 10001, `.ok` is 0600; `docker compose down` takes 0.5 s, and
      after `up` again the volume keeps the logs and the polling goes on.
      arm64 (2026-10-21): the Dockerfile cross-compiles (Rust on the build machine's
      architecture, `gcc-aarch64-linux-gnu` for ring's C), and the final stage only copies files,
      so `docker buildx build --platform linux/amd64,linux/arm64` needs no emulation. The arm64
      binary is aarch64, needs only libc, libm and libgcc_s and glibc 2.34 (bookworm has 2.36); it
      could not be run on this amd64 host. A plain `docker build` on amd64 stays native.

## 0.2.0, the first public release (2026-10-21)

0.1.0 is the MRTG-compatible release this plan builds on; 0.2.0 adds builds 1 and 2, the manual,
Docker, `async-snmp` and the security assessment's fixes, some of which change behavior, so it is
a minor version (semantic versioning before 1.0). `CHANGELOG.md` lists what changed and, under
"Upgrading from 0.1.0", every behavior change to check.

- [x] 0.2.0 released and public.
      2026-10-21:
      - Release PR #15 (`c5b9c33`): the version is 0.2.0 everywhere, plus the changelog.
      - Tags: `v0.1.0` on the first commit (`3158c73`), `v0.2.0` on `c5b9c33`. The GitHub release
        v0.2.0 carries the changelog's notes.
      - Image: `ghcr.io/rinjanianalytics/linkscope:0.2.0` and `latest`, one index for linux/amd64
        and linux/arm64 (`sha256:7d9d05bc...`), built from a clean export of `v0.2.0`. The six
        versions pushed earlier as `0.1.0` (today's code under the old number) were deleted.
      - The lab runs 0.2.0 since 14:05 WIB: both CHR ports ok, `linkscope_info{version="0.2.0"}`.
      - Public: the repository first, then the release, then the package (14:23 WIB), so a GPL
        binary is never public without its source. Checked without a login: the repository
        (license GPL-2.0), the release, and `docker pull` of `0.2.0` and `latest` (amd64 and arm64)
        running as `linkscope 0.2.0`.
      - About on GitHub: the website https://rinjanianalytics.com and 14 topics (linkscope, rust,
        snmp, snmpv3, mrtg, flowscope, akvorado, network-monitoring, bandwidth-monitoring,
        prometheus, isp, netops, self-hosted, docker).

Releasing again (2026-10-21: a release pipeline in GitHub Actions):
bump the version and turn CHANGELOG's "Unreleased" into the version's section (with "Upgrading"
when behavior changes), merge, then push a tag `v<version>`. `.github/workflows/release.yml` runs CI, checks that the tag, the
version and the changelog agree, pushes the image for linux/amd64 and linux/arm64 (and `latest`,
except for a pre-release such as `0.3.0-rc.1`), and last creates the GitHub release from the
changelog, then runs the published image on amd64 and arm64 (`smoke.yml`).
`.github/workflows/ci.yml` checks every pull request and every push to main: rustfmt, the UI
rebuilt and compared with the committed `dist`, clippy and the tests on Rust 1.99.0 (the image's),
and a build on 1.89 (`rust-version`). A pull request that changes the Dockerfile, and a manual run
of Release, builds both images without pushing.

- [x] CI and the release pipeline.
      2026-10-21, PR #17 (`bf22df4`):
      - Actions runs on the public repository (the billing block of the private PathScope
        repositories does not apply), and CI passed on main after the merge.
      - The release dry run built both images in 4.5 minutes.
      - the owner gave the repository's Actions the Write role on the package.
      - Only GitHub's own actions (`checkout`, `cache`), pinned by commit; the Docker steps are
        plain commands. The token is read-only except in the publish job, which only a tag starts.
      - The first run found a bug in 0.2.0: `aes` 0.9.3, which `async-snmp` brings in, needs Rust
        1.89, so `rust-version` 1.88 had been wrong since #12. The minimum is now 1.89 in
        `Cargo.toml`, CI, the manual (EN, ID) and the changelog. The v0.2.0 release notes were
        corrected too. The image builds with 1.92 and was never affected.
- [x] rustfmt once, then checked (2026-10-21).
      PR #19: `cargo fmt --all` with Rust 1.92.0's rustfmt, default settings, in a commit of its
      own (`a54d2e8`, 47 files, formatting only: clippy clean, 114 tests pass), and a Rustfmt job
      in CI. `.git-blame-ignore-revs` lists the commit, so blame skips it (GitHub on its own; locally
      `git config blame.ignoreRevsFile .git-blame-ignore-revs`). PR #20 added it; in
      `crates/config/src/check.rs`, the lines blame gives to the formatting commit fell from 218 to 4.
- [x] CI required before merging to main (2026-10-21).
      Branch protection on `main`: the checks Rustfmt, UI build (since #22), Clippy and tests, and
      Build on the oldest supported Rust must pass, for admins too; the branch need not be up to date with main first;
      no review required (one maintainer); no force pushes or deletion. The Release dry run is not
      required, since it runs only when the Docker files change.
      - Each check must come from GitHub Actions (app 15368), so a status posted any other way
        cannot stand in for it.
      - Tested on PR #20 (`6abb285`): a merge while CI was queued was refused ("3 of 3 required
        status checks are queued"); after the three passed, it merged.
      - Tags are not affected: a release tag still starts the release workflow, which runs CI
        itself before publishing.
      - Renaming a CI job renames its check: change the protection in the same change, or every
        pull request waits for a check that never comes (Settings, Branches, main).
      - Nothing reaches main except through a pull request with green CI, admins included. To
        change that in an emergency, turn off "Do not allow bypassing" there, and turn it back on.
- [x] A public repository's basics (2026-10-21).
      Going public left no private way to report a vulnerability: GitHub showed this repository's
      `docs/SECURITY.md` as its policy, and it had no reporting section, so the only channel was a
      public issue.
      - Settings, on since then: private vulnerability reporting, Dependabot alerts and security
        updates, secret scanning and push protection. Their first scans found nothing.
      - PR #22 (`43e24e0`):
        - `docs/SECURITY.md` opens with "Reporting a vulnerability": GitHub's private report,
          what to include, an answer within seven days, credit.
        - `.github/dependabot.yml`: cargo, npm, GitHub Actions and Docker, weekly (Mondays,
          06:00 WIB), grouped.
        - CI's "UI build" rebuilds `crates/web/ui/dist` with Node 22 and fails when it differs
          from the committed one; a fresh build reproduced it byte for byte. An npm update
          (Dependabot's too) needs `npm ci && npm run build` and the new `dist` on its branch.
          It is a required check.
        - `smoke.yml` runs the published image as the manual's Docker quick start, on amd64 and
          on arm64 (GitHub's arm runner): cfgmaker from the image against snmpd on the runner,
          `docker compose up`, a poll that succeeded, `/metrics`, `/docs`, a healthy container.
          On 0.2.0, the first run of the arm64 image anywhere, both passed alike: 2 interfaces
          found, 2 polled, 0 failed, `/docs` 200, healthy.
        - `CONTRIBUTING.md`, issue forms (bug, idea; the private report and the manual as
          links), and an "Unreleased" section in the changelog.
      - GitHub's community profile went from 57 % to 71 %; a code of conduct and a pull request
        template are what it still lists.
- [x] The first release through the pipeline: 0.2.1 (2026-10-21: "cut the release").
      A patch release, with no change in behavior or configuration: the dependency updates of
      #33, #34 and #35 and the project work of #22. Dependabot's first nine pull requests (#23 to
      #31, all major versions) became those three PRs and then closed on their own:
      - #33: `hmac` 0.13, `sha2` 0.11, `nix` 0.31. Each failed alone; together they drop a second
        copy of each crate, since `async-snmp` already used them.
      - #34: Rust 1.99 for the image and CI together. Clippy and rustfmt needed no change.
      - #35: React 19 as a set (#31 had moved `react-dom` alone), recharts 3, TypeScript 7 and
        tailwind-merge 3. CI's new UI check found that Tailwind had scanned the committed
        `dist`, so each build's CSS depended on the previous build. `index.css` now names its
        sources, and a rebuild is byte-identical. CSS went from 57.6 to 45.4 kB. Checked in
        installed Chrome against a binary with the new UI, beside the lab's 0.2.0: the pages
        matched apart from live values.
      Release PR #36 (`536fc36`), tag `v0.2.1`, then `release.yml` from 10:51 to 10:58 UTC (6 min):
      - CI's four checks, then the image for amd64 and arm64 pushed with the repository's own token
        (`sha256:fb5066f5...`, tags `0.2.1` and `latest`; `0.2.0` kept);
      - the GitHub release from the changelog, now the latest;
      - the image run on both platforms: 2 polled, 0 failed, healthy.
      Checked without a login: the release, both tags on one index, its labels (version 0.2.1,
      revision `536fc36`), and `linkscope 0.2.1` from a fresh pull.
      The lab runs 0.2.1 since 18:01 WIB (built with Rust 1.99): both CHR ports ok,
      `linkscope_info{version="0.2.1"}`, the manual over the tailnet, a foreign Host refused
      (421). The rollback `target/release/linkscope.before-0.2.1` is 0.2.0, built from its tag.

## gNMI beside SNMP (2026-10-22)

SNMP reads every router LinkScope meets today, but the larger platforms now expose their counters
through gNMI (gRPC Network Management Interface) and treat it as the main way to read them. The owner asked
for gNMI beside SNMP, so LinkScope reads more router models. SNMP stays: MikroTik and most older
gear have no gNMI.

**Free (2026-10-22).** gNMI is part of the free, GPL program. gNMI clients are common open
source, and the paid program's value is the analysis on top of the counters.

**How it fits.** A gNMI interface is a third kind of data source beside SNMP and external
programs (`Source` in `crates/config/src/target.rs`). It yields the same in and out octet
counters, so the `.log` files, rates, alerts, `/metrics`, the API and the UI work on it unchanged.
A target may even combine an SNMP and a gNMI source in one expression, as MRTG combines sources
today.

**Config** (proposed; the exact syntax is settled in build 4's PR):

```
GnmiDevice[srl1]: address=10.10.0.80:57400 user=linkscope password-file=/etc/linkscope/srl1.pass
                  ca=/etc/linkscope/lab-ca.pem
Target[srl1_e1-1]: gnmi:ethernet-1/1@srl1
MaxBytes[srl1_e1-1]: 1250000000
```

- `gnmi:<interface>@<device>` is read only when `<device>` is a declared `GnmiDevice`, before
  MRTG's own passes. A config without `GnmiDevice` parses exactly as before (ground rule 1).
- TLS is on by default and the server's certificate is checked. `insecure` exists for labs but
  must be written out, and the UI shows it on the target.
- The password comes from a file or an environment variable, never inline. Client certificates
  (mutual TLS) are an option. The password, the key and the file paths stay out of every output
  (ground rule 2).
- `model=openconfig` (the default) reads OpenConfig paths:
  `/interfaces/interface[name=…]/state/counters/in-octets` and `out-octets`, plus `oper-status`,
  `description` and the speed. Devices whose OpenConfig is off or partial use a vendor model
  (`model=srl` first). Each model is a small table of paths in the code, so a new vendor is a
  table and its tests, not new logic.

**Crates.** `tonic` (MIT) and `prost` (Apache-2.0), with `gnmi.proto` and `gnmi_ext.proto` from
openconfig/gnmi (Apache-2.0) compiled in pure Rust (no `protoc` on the build host). LinkScope
already ships Apache-2.0-only crates (`ring`, `sync_wrapper`), so this adds nothing new to the GPL
review in the open decisions. Build 4 records the binary and image size before and after, and
keeps the minimum Rust at 1.89 or says why not.

**The lab.** Nokia SR Linux runs as a free container (`ghcr.io/nokia/srlinux`; a few GB of
memory, and the lab host has 31 GB) and answers both gNMI and SNMP. So one of its ports can be read both ways at once, which is
the best check that the two agree. It gets its own compose file in `testenv/`, with a sender and
a sink pushing traffic through it as the CHR's do. The CHR stays on SNMP; RouterOS has no gNMI.

**Router models.** A model is marked tested only after a gate or a recorded run on that platform:
real gear or the vendor's own lab image. "By spec" means its documentation lists gNMI with the
OpenConfig interface counters; it should work but has not been run.

| Platform | gNMI | LinkScope |
|---|---|---|
| Nokia SR Linux | yes, OpenConfig and native | tested in build 4's gate |
| Arista EOS | yes, OpenConfig | by spec; cEOS-lab could be tested (needs an Arista account) |
| Cisco IOS XR, IOS XE, NX-OS | yes, OpenConfig | by spec |
| Juniper Junos, Junos Evolved | yes, OpenConfig | by spec |
| Nokia SR OS | yes, OpenConfig and native | by spec |
| Huawei VRP | on some models | by spec, per model |
| MikroTik RouterOS | no | SNMP, as today |

4. **gNMI targets.** `GnmiDevice[]` and `gnmi:` targets, read once per cycle with a gNMI `Get`
   (or a `Subscribe` in `ONCE` mode where a device has no `Get` for state). One connection per
   device, reused across cycles, inside the same per-agent time budget as SNMP, so a slow
   device delays nobody else. Errors are named on the target: refused, TLS (with the reason),
   wrong login, path not supported by the model, interface not found. `/metrics` and the API
   gain the source kind (`snmp`, `gnmi`, `exec`); the UI shows it on the target page. The manual
   gets a gNMI page in English and Bahasa.

   Gate: on SR Linux in the lab, one busy port read by SNMP (64-bit counters) and by gNMI in the
   same cycles for an hour:
   - each poll's counters agree within the traffic between the two reads;
   - the two targets' rates within 1 % of each other over the hour;
   - a wrong password, an untrusted certificate and a stopped device are each named on their
     target, and the other targets' cycle time stays within its budget;
   - `insecure` is refused unless written out;
   - no password, key or file path in the API, `/metrics`, alerts or the log (`no_secrets.rs`
     extended);
   - every existing test passes, and a config without the new keywords writes the same `.log`
     files as before.
5. **Discovery.** `linkscope cfgmaker` learns gNMI: given a device, it reads its interfaces (name,
   description, state, speed), checks which model answers (OpenConfig, else the vendor's), and
   writes the `GnmiDevice[]` and one target per interface, with `MaxBytes` from the speed, as it
   does for SNMP. A mock gNMI server in `crates/mockagent` replays recorded answers, so every
   model's paths are tested in CI without a router.

   Gate: on SR Linux, the generated config polls every port that is up in its first cycle, with
   the right `MaxBytes`. Against the mock, the OpenConfig and SR Linux models both produce
   targets; a device answering neither is named, not skipped silently.
6. **Streaming (later, after 0.3.0).** A `Subscribe` in `SAMPLE` mode, every 10 s for example,
   keeps a live rate per gNMI target for the UI and `/metrics`, while the `.log` files still get
   one value per cycle as MRTG writes them. Planned only after builds 4 and 5 run on a real
   install, since it changes how often LinkScope talks to a router.

   Gate: written when the build starts.

Builds 4 and 5 ship together as 0.3.0.

- [ ] Build 4: gNMI targets, SR Linux read by SNMP and gNMI in agreement for an hour.
- [ ] Build 5: gNMI discovery, a generated config that polls every port that is up.
- [ ] Build 6: streaming, gate to be written.

## Open decisions

- The paid program's name, and whether it starts as a license, SaaS or both.
- A review of the free and paid split by someone who knows GPL licensing, before anything is sold.
- Whether the manual also gets a public website (an mdBook build of `docs/manual`, for example on
  rinjanianalytics.com). The repository is public, and GitHub already shows the pages.
- `/metrics` on the UI's listener (simplest) or a listener of its own (`MetricsListen:`), for
  installs that expose the UI and the metrics to different networks.
- The answer time `docs/SECURITY.md` promises to vulnerability reporters (seven days, written on
  the owner's behalf).
- A code of conduct for contributors (the Contributor Covenant, for example), which would also
  complete GitHub's community profile with a pull request template.
- A contact email in the UI's footer, once rinjanianalytics.com receives mail (on 2026-10-22 it had
  no MX record, so any address on it would bounce).
