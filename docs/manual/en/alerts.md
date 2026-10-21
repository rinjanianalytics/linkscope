# Alerts

LinkScope checks thresholds the way MRTG does, with the same keywords, programs, mails and state
files. It adds four things, all off until you set them: a bound that must stay broken for several
polls before it fires, a signed webhook, mail over STARTTLS with a login, and the alert state in
the UI and the API.

## MRTG's thresholds

These keywords work exactly as in MRTG 2.17.10:

| Keyword | Meaning |
|---|---|
| `ThreshMinI[x]`, `ThreshMinO[x]` | Broken when the in (or out) rate falls below this. |
| `ThreshMaxI[x]`, `ThreshMaxO[x]` | Broken when the in (or out) rate rises above this. |
| `ThreshDesc[x]` | A description passed to programs (as `THRESH_DESC`) and mails. |
| `ThreshProgI[x]`, `ThreshProgO[x]` | A program to run when an in (or out) bound is broken. |
| `ThreshProgOKI[x]`, `ThreshProgOKO[x]` | A program to run when it is no longer broken. |
| `ThreshMailAddress[x]` | Mail the threshold notice to this address. |
| `ThreshMailServer` | The mail server, `host` or `host:port`. |
| `ThreshMailSender` | The sender address. Needed with `ThreshMailServer`. |
| `ThreshDir` | A directory for state files, so programs and mails run once per change. |
| `ThreshHyst` | MRTG's hysteresis rule, used without `ThreshDir` (only when set). |

A bound is a rate in bytes per second, or a percentage of `MaxBytes` when it ends in `%`
(`ThreshMaxI[x]: 80%`). LinkScope compares it with the day graph's current rate, as MRTG does.

A threshold program runs directly, without a shell, with the target's `SetEnv[]` and
`THRESH_DESC` in its environment. Its arguments are MRTG's: the target name, the bound as
configured, the current value, and, when broken, the description (`No Description` when none).
A program that runs longer than 60 seconds is stopped, together with everything it started (it
runs in a process group of its own).

The program is the file the configuration check found: a relative path is taken from the
directory LinkScope was started in, and run as that file. MRTG checked the path but ran a bare
name through `PATH`, which could run another program than the one checked. The file must be
executable.

A configured secret (an SNMP community, a password, the webhook URL) that a `Title[]`,
`ThreshDesc[]` or `PageTop[]` quotes is replaced with `•••` in the mail, in the program's arguments
and in `THRESH_DESC`, as in the webhook body. Other text, e-mail addresses included, is as MRTG
writes it.

Without `ThreshDir`, MRTG runs the program and sends the mail on every broken poll. With
`ThreshDir`, a state file per bound (`<target>.threshmaxI` and so on) records that it is broken,
so they run once when it breaks and once when it recovers. LinkScope keeps both behaviors.

A state file that cannot be used (`ThreshDir` gone or not writable, or something else in its place:
a directory, a link, a file of another account) is an error in the log. LinkScope then remembers
the change in memory, so the program and the mail still run once per change rather than on every
poll.

## What LinkScope adds

```text
ThreshMaxI[core-uplink]: 80%
ThreshFor[core-uplink]: 3

AlertWebhook: https://alerts.example.com/linkscope
AlertWebhookSecret: 3f7c9a1e5b2d4c6f8a0e

ThreshMailServer: smtp.example.com:587
ThreshMailSender: linkscope@example.com
AlertMailUser: linkscope@example.com
AlertMailPassword: app-password
```

| Keyword | Meaning |
|---|---|
| `ThreshFor[x]: N` | A bound fires after N broken polls in a row, and resolves at the first poll that is not broken. |
| `AlertWebhook` | Post every alert that fires or resolves to this URL. Needs `AlertWebhookSecret`. |
| `AlertWebhookSecret` | The key that signs each post. At least 16 characters. |
| `AlertMailUser`, `AlertMailPassword` | Log in to `ThreshMailServer` over STARTTLS. Both or neither. |

### ThreshFor

Without `ThreshFor[]`, a bound fires at its first broken poll, and the target's programs and mail
run exactly as MRTG runs them.

With `ThreshFor[x]: N`, a bound must stay broken for N polls in a row before it fires. A single
poll back within the bound starts the count again. The target's `ThreshProg*` and
`ThreshMailAddress[]` then run once when the alert fires and once when it resolves, with MRTG's
arguments and message format.

A poll with no rate for a direction (no answer from the device) changes nothing: missing data
neither fires nor clears an alert.

### Mail with a login

Without `AlertMailUser`, mail goes as MRTG sends it: plain SMTP, to port 25 unless
`ThreshMailServer` names a port. To a server other than this host, LinkScope warns at startup
that the mail travels without TLS.

With `AlertMailUser` and `AlertMailPassword`, LinkScope connects to port 587 unless
`ThreshMailServer` names one, requires STARTTLS, checks the server's certificate, and logs in. If
STARTTLS fails, the mail is not sent. `AlertMailUser` needs `ThreshMailServer`.

The subject and body follow MRTG: the subject starts with `[MRTG-TH] BROKEN` when a bound breaks
and `[MRTG-TH] UN-BROKEN` when it recovers, followed by the target, the bound and the values.

### Certificates

Mail and webhooks trust the system's certificate authorities. To use your own, set
`SSL_CERT_FILE` to a PEM file in LinkScope's environment. It replaces the system's authorities.

## Webhook

With `AlertWebhook` set, LinkScope posts one JSON body for every alert that fires or resolves, on
every target with a threshold. A target without `ThreshFor[]` fires at its first broken poll.

Each request has these headers:

| Header | Value |
|---|---|
| `Content-Type` | `application/json` |
| `User-Agent` | `LinkScope/<version>` |
| `X-LinkScope-Event` | `threshold` |
| `X-LinkScope-Delivery` | This notice's id, the same on every attempt |
| `X-LinkScope-Timestamp` | Unix time of this attempt |
| `X-LinkScope-Signature` | `sha256=` and the hex HMAC-SHA256 of the timestamp, a full stop and the body |

The body:

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

| Field | Type | Meaning |
|---|---|---|
| `event` | string | Always `threshold`. |
| `state` | string | `firing` or `resolved`. |
| `reason` | string or null | For `resolved`: `cleared` (back within the bound) or `removed` (the bound left the configuration). Null when firing. |
| `target` | string | The target's name. |
| `title` | string | `Title[]` as plain text. |
| `directory` | string | `Directory[]`, without the trailing slash. |
| `direction` | string | `in` or `out`. |
| `bound` | string | `min` or `max`. |
| `threshold` | string | The bound as configured, for example `80%` or `37500`. |
| `threshold_rate` | number | The bound as a rate per second. |
| `rate` | number | The current rate per second (bytes for interface counters). |
| `percent` | number or null | The rate in percent of `MaxBytes`, for a percent bound. |
| `polls` | number | Broken polls in a row: when it fired, or up to the poll that cleared it. |
| `since` | number | Unix time of the first broken poll. |
| `time` | number | Unix time of this poll. |
| `description` | string or null | `ThreshDesc[]`. |
| `linkscope` | string | LinkScope's version. |

### Retries

LinkScope waits up to 10 seconds for an answer (5 seconds to connect). It tries three times (now,
after 2 seconds, after 10 seconds) on a network error, a 5xx or a 429. Any other answer is final.
It never follows a redirect, so the signed body cannot go somewhere you did not configure, and it
ignores proxy settings in its environment (`HTTP_PROXY`, `ALL_PROXY`). A failure is logged without
the URL.

Delivery is at least once: an attempt that timed out may still have arrived, and the next one is
then a second copy. Every attempt of one notice carries the same `X-LinkScope-Delivery`, so a
receiver drops a delivery id it has already seen.

An `http://` URL to another host is allowed, but LinkScope warns at startup: the body, and any
token in the URL, travel unencrypted. Use `https://`.

### Check the signature

Compute the HMAC-SHA256 of the timestamp header, a full stop and the raw body, keyed with
`AlertWebhookSecret`, and compare it with the signature header. Reject old timestamps to stop
replays:

```python
import hashlib, hmac, time

def verify(secret: bytes, timestamp: str, body: bytes, signature: str) -> bool:
    """True for a signed, recent request; False for anything else, malformed headers included."""
    try:
        sent = int(timestamp)
        want = hmac.new(secret, timestamp.encode() + b"." + body, hashlib.sha256).hexdigest()
        ok = hmac.compare_digest(("sha256=" + want).encode(), signature.encode())
    except (TypeError, ValueError, AttributeError, UnicodeError):
        return False
    return ok and abs(time.time() - sent) < 300
```

The webhook URL, its secret and the mail password never appear in the UI, the API, `/metrics` or
the log.

## Alert state

LinkScope keeps the alerts in a state file beside the `.ok` cache: `mrtg.alerts` for
`mrtg.cfg`, or the `--confcache-file` path with its extension changed to `.alerts`. A reload or a restart neither repeats a
notice nor loses one. A bound removed from the configuration while firing sends one `resolved`
notice with the reason `removed`.

## In the UI and the API

- A target with a firing alert has the status `warn`, and its card shows the broken bound, for
  example `In above 80%, now 86.2%`.
- A bound broken for fewer polls than `ThreshFor[]` shows as waiting, for example
  `1 of 3 polls before it fires`, and the target stays `ok`.
- The [JSON API](api.md) lists each target's broken bounds in its `alerts` field.
