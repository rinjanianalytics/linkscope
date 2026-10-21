# Configuration

LinkScope reads MRTG's `mrtg.cfg` as it is. You do not need to convert or rewrite it. This page
describes how the file is read, the `Target[]` syntax, and every keyword LinkScope accepts. The
keywords LinkScope adds are marked **New**; MRTG itself rejects them, so keep them out of files
you still share with MRTG.

## How the file is read

A configuration file holds two kinds of lines:

- **Global keywords** apply to the whole run: `Keyword: value`.
- **Per-target keywords** apply to one target: `Keyword[name]: value`.

```ini
WorkDir: /var/lib/mrtg

Target[core-uplink]: 2:public@192.0.2.1:::::2
Title[core-uplink]: Core uplink
MaxBytes[core-uplink]: 125000000
```

The rules follow MRTG exactly:

- Keywords are not case sensitive. Target names are read in lower case.
- A line that starts with `#` is a comment. Blank lines are ignored.
- A line that starts with a space or a tab continues the previous per-target keyword. A
  continuation after a **global** keyword is ignored, as MRTG ignores it, and LinkScope warns
  about it. Put global values on one line.
- Defining the same `Keyword[name]` twice is an error that names both lines.
- An unknown keyword is an error. Keywords of other tools written as `tool*Keyword[name]` are
  accepted without checks.
- `Colors` is read as `Colours`, and the old spelling `Supress` as `Suppress`.

### Defaults, prefixes and suffixes

Three pseudo-targets set values for the targets that follow them:

| Form | Meaning |
|---|---|
| `Keyword[_]: value` | Default for every later target that does not set `Keyword` |
| `Keyword[^]: value` | Prefix put in front of every later target's value |
| `Keyword[$]: value` | Suffix put after every later target's value |

An empty value (`Options[_]:`) clears the default, prefix or suffix. A prefix is joined to the
value with a space, and a suffix too. If the global `NoSpaceChar` character ends the prefix or
suffix, it is removed and no space is added.

```ini
Options[_]: growright, bits
Directory[_]: routers
```

### Include

`Include: file` reads another file at that point. A relative path is relative to the directory
of the main configuration file (MRTG tried the working directory first, so where it was started
decided which file was read). A path with `*` is a glob; LinkScope reads the matching files in
sorted order (MRTG read them in reverse order). Includes may nest 16 levels deep, a file that
includes itself is an error, and one configuration reads at most 1,000 files and 64 MB in all.
An included file must be a regular file: a FIFO or a device is refused.

### Trusted files

A configuration file can run programs (external command targets and threshold programs), so
LinkScope only loads files that other accounts cannot change. Each file, including every
included file, must be owned by root or by the account that runs LinkScope, and neither the file
nor its directory may be writable by group or others (a sticky directory is fine).
`--allow-insecure-config` turns the refusal into a warning. See [Security](security.md).

## Required settings

| Keyword | Rule |
|---|---|
| `WorkDir` | Where the `.log` files go. Instead of it you may set all three of `LogDir`, `HtmlDir` and `ImageDir`. |
| `Target[name]` | The data source for the target. |
| `Title[name]` | The target's title. |
| `MaxBytes[name]` | The highest valid value. You may set `MaxBytes1` and `MaxBytes2` for input and output instead. |

LinkScope creates missing directories with mode 0755.

## The Target syntax

A `Target[]` value names one or more data sources and can combine them with arithmetic.

### Interface targets

The common form polls an interface's input and output octet counters:

```text
selector:community@host[:port:timeout:retries:backoff:version]
```

The selector chooses the interface:

| Selector | Example | Interface chosen by |
|---|---|---|
| number | `2:public@192.0.2.1` | ifIndex |
| `/address` | `/192.0.2.17:public@192.0.2.1` | the IPv4 address it carries |
| `\descr` | `\ether1:public@192.0.2.1` | ifDescr |
| `#name` | `#Gi0/1:public@192.0.2.1` | ifName |
| `!mac` | `!00-00-5e-00-53-01:public@192.0.2.1` | its MAC address |
| `%type` | `%6:public@192.0.2.1` | ifType |

In a description or name, escape a space, `:`, `&` or `@` with a backslash (`\GigabitEthernet\ 0/1`).
A leading `-` (`-2:public@192.0.2.1`) swaps input and output. LinkScope looks the selector up on
the device and keeps the result in the interface cache (`<config>.ok`).

The host is a name, an IPv4 address, or an IPv6 address in brackets (`[2001:db8::1]`).

### Connection settings

The part after the host sets how LinkScope talks to the agent. Every field is optional; leave
one empty to keep its default.

| Field | Default | Notes |
|---|---|---|
| port | 161 | |
| timeout | 2 s | Between 0.05 and 300 s |
| retries | 5 | At most 50 |
| backoff | 1.0 | The timeout grows by this factor on each retry, between 1 and 10 |
| version | 1 | `1`, `2` (or `2c`), or `3` |

The version is the fifth field, so it needs five colons after the host:

```ini
Target[edge]: 2:public@192.0.2.1:::::2
Target[edge-v3]: 2:v3@192.0.2.1:::::3
```

With version 3 the user and keys come from `SnmpOptions` (below). The Target still needs a word
before the `@`; any word works.

Use version 2 or 3 for interface targets. Only they read the 64-bit counters (ifHCInOctets and
ifHCOutOctets). Version 1 counters are 32 bits and wrap every 34 seconds at 1 Gbit/s.
`NoHC[name]: yes` makes a target use the 32-bit counters anyway.

### Explicit OIDs

Two OIDs joined by `&` poll any pair of values:

```ini
Target[cpu]: 1.3.6.1.4.1.9.2.1.58.0&1.3.6.1.4.1.9.2.1.58.0:public@192.0.2.1:::::2
Target[pkts]: ifInUcastPkts.2&ifOutUcastPkts.2:public@192.0.2.1:::::2
```

An OID is dotted numbers or a name LinkScope knows (MIB-2 names, and names from the files in
`LoadMIBs`). An interface selector can replace the index: `ifInErrors#Gi0/1&ifOutErrors#Gi0/1:...`.

### Arithmetic

A Target can combine sources with numbers:

```ini
Target[both-uplinks]: 2:public@192.0.2.1:::::2 + 3:public@192.0.2.1:::::2
Target[in-bits]: 2:public@192.0.2.1:::::2 * 8
```

LinkScope compiles the expression when it loads the file; it never runs it as code. It accepts
data sources, numbers (decimal, hex, octal, with `_` separators), `+ - * / % **`, parentheses,
unary minus, `int()` and `abs()`, with Perl's precedence and arbitrary precision. Anything else
is an error at load time.

### External commands

A command in backticks supplies the values:

```ini
Target[mail-queue]: `/usr/local/bin/mailq-count`
```

The command prints up to four lines: the input value, the output value, the uptime and the name.
LinkScope runs it directly, without a shell, with a 120 s timeout. A command that needs a shell
(pipes, redirection, variables or globs) is an error unless you set `ExecShell: yes`. `PathAdd`
adds directories to the command's `PATH`.

### SNMPv3 and SnmpOptions

SNMPv3 settings go in `SnmpOptions`, globally or per target, as `key => value` pairs. Values are
numbers or quoted strings; LinkScope parses them as data, never as Perl.

```ini
SnmpOptions[edge-v3]: username => 'linkscope', authprotocol => 'sha256',
  authpassword => 'example-auth-pass', privprotocol => 'aes',
  privpassword => 'example-priv-pass'
```

| Key | Values |
|---|---|
| `username` | Required for version 3 |
| `authprotocol` | `md5` (default), `sha` (or `sha1`), `sha224`, `sha256`, `sha384`, `sha512` |
| `authpassword` | Without it, the target uses noAuthNoPriv |
| `privprotocol` | `des` (default), `aes` (or `aes128`), `aes192`, `aes256` |
| `privpassword` | With `authpassword`, the target uses authPriv; without it, authNoPriv |
| `contextname` | The SNMP context |

Prefer authPriv with SHA-2 and AES. Localized keys (`authkey`, `privkey`) are not supported; use
the passwords.

`SnmpOptions` also takes `port`, `timeout`, `retries` and `backoff` (used when the Target leaves
them empty), `maxrepetitions` (rows per GETBULK request, default 25) and `use_getbulk` (`0` to
walk tables with GETNEXT).

Conversion subroutines (`|name` after the host) need Perl and are not supported; a Target that
uses one is reported when the file is loaded.

## Global keywords

| Keyword | Default | Meaning |
|---|---|---|
| `WorkDir` | none | Directory for the `.log` files (and MRTG's pages) |
| `LogDir`, `HtmlDir`, `ImageDir` | none | Separate directories, used together instead of `WorkDir` |
| `Interval` | 5 | Minutes between polls, or `M:SS`. Less than 5 minutes is an error. |
| `RunAsDaemon` | no | Keep running and poll every `Interval` (like `--daemon`) |
| `NoDetach` | no | In daemon mode, stay attached to the terminal (like `--foreground`) |
| `MaxAge` | none | Seconds; a log not written for longer is not updated, and the target fails |
| `EnableIPv6` | no | Allow SNMP over IPv6. Per target, `IPv4Only[name]: yes` turns it off again. |
| `EnableSnmpV3` | no | Accepted for MRTG configs (`yes` or `no`) |
| `SingleRequest` | no | Ask for one OID per request, for agents that cannot answer several |
| `NoMib2` | no | Do not read sysUptime and sysName from the agents |
| `LoadMIBs` | none | MIB files whose object names a Target may use |
| `PathAdd` | none | Directory added to the `PATH` of external commands |
| `SnmpOptions` | none | SNMP settings for every target (see above) |
| `NoSpaceChar` | none | One character that joins a prefix or suffix without a space |
| `ThreshDir` | none | Directory for threshold state files (see [Alerts](alerts.md)) |
| `ThreshHyst` | 0.1 | Hysteresis for thresholds, as a fraction |
| `ThreshMailServer` | none | Mail server for threshold mail. **New:** also `host:port`. Needs `ThreshMailSender`. |
| `ThreshMailSender` | none | Sender address for threshold mail. Needs `ThreshMailServer`. |
| `LogFormat` | rateup | Only `rateup` (MRTG's `.log` files); `rrdtool` is an error |
| `Forks`, `Refresh`, `Language`, `WriteExpires`, `IconDir` | | Accepted for MRTG configs; LinkScope does not use them |
| `ConversionCode`, `LibAdd`, `SendToGraphite`, `RRDCached` | | Accepted with a warning and ignored (they need Perl or RRDtool) |

### New global keywords

| Keyword | Default | Meaning |
|---|---|---|
| `WebListen` | none | **New.** Address and port of the web UI, the JSON API and `/metrics`, such as `127.0.0.1:8080` or `[::1]:8080`. `--listen` overrides it. Without either, there is no web listener. |
| `WebHosts` | none | **New.** Host names the web UI answers to, besides IP addresses and `localhost`, separated by commas or spaces, such as `noc.example.com`. A request for any other name gets `421`: this keeps a web page that rebinds its own name to the UI's address from reading it. `*` answers to any name. |
| `ExecShell` | no | **New.** `yes` lets external commands run through `/bin/sh`. |
| `AlertWebhook` | none | **New.** An `http://` or `https://` URL that gets a signed POST when an alert fires or clears. Needs `AlertWebhookSecret`. |
| `AlertWebhookSecret` | none | **New.** The signing key, at least 16 characters. Needs `AlertWebhook`. |
| `AlertMailUser` | none | **New.** Log in to `ThreshMailServer` over STARTTLS with this user. Needs `AlertMailPassword`. |
| `AlertMailPassword` | none | **New.** The password for `AlertMailUser`. |

[Alerts](alerts.md) explains how these work. The webhook URL, its secret and the mail password
never appear in the UI, the API, `/metrics` or error messages.

## Per-target keywords

### Data and polling

| Keyword | Meaning |
|---|---|
| `Target[name]` | The data source (required) |
| `MaxBytes[name]` | The highest valid value per second (required, or `MaxBytes1` and `MaxBytes2`) |
| `MaxBytes1[name]`, `MaxBytes2[name]` | Separate maximums for input and output |
| `AbsMax[name]` | Values above `MaxBytes` and up to this are kept; it may not be smaller than `MaxBytes` |
| `Options[name]` | Flags, see below |
| `SnmpOptions[name]` | SNMP settings for this target's sources |
| `NoHC[name]` | `yes`: use 32-bit counters |
| `IPv4Only[name]` | `yes`: no IPv6 for this target |
| `RouterUptime[name]` | Read the uptime and name from another agent, `community@host` |
| `RouterName[name]` | Read the name from another OID |

### Logging

| Keyword | Meaning |
|---|---|
| `Directory[name]` | Subdirectory of `WorkDir` for this target's log. It stays inside `WorkDir`: a leading `/` is dropped and `..` is an error. |

### Display

These shape the target in the [Web UI](web-ui.md):

| Keyword | Meaning |
|---|---|
| `Title[name]` | Title (required) |
| `PageTop[name]`, `PageFoot[name]` | Text shown above and below the graphs; HTML is sanitized |
| `YLegend[name]`, `ShortLegend[name]` | Axis legend and unit |
| `Legend1[name]` to `Legend5[name]`, `LegendI[name]`, `LegendO[name]` | Legend texts |
| `kMG[name]` | Unit prefixes, such as `k,M,G,T` |
| `Kilo[name]` | 1000 (default) or 1024 |
| `Factor[name]` | Multiply shown values by this |
| `Unscaled[name]`, `WithPeak[name]`, `Suppress[name]` | Letters `d w m y` (day, week, month, year): graphs without scaling, with peaks, or not shown |
| `Colours[name]` | `Name#rrggbb` for in, out, max in and max out |

MRTG's other page and graph keywords (`XSize`, `YSize`, `Background`, `PNGTitle`, `TimeStrPos`,
`AddHead`, `BodyTag` and the rest) are accepted and checked as MRTG checks them. LinkScope writes
no HTML or PNG files, so they do not change the UI.

### Options

`Options[name]` takes a comma-separated list. The ones that change the data:

| Option | Meaning |
|---|---|
| `gauge` | Values are levels, not counters |
| `absolute` | Values are counts since the last poll |
| `perminute`, `perhour` | Show rates per minute or per hour |
| `unknaszero` | Log an unknown value as zero |
| `bits` | Show bits instead of bytes |

The others (`growright`, `logscale`, `nopercent`, `noi`, `noo`, and the rest of MRTG's list) are
accepted. `derive` needs RRDtool and is an error. An unknown option is an error.

### Thresholds

| Keyword | Meaning |
|---|---|
| `ThreshMinI[name]`, `ThreshMaxI[name]`, `ThreshMinO[name]`, `ThreshMaxO[name]` | Bounds for input and output, a number or a percentage of `MaxBytes` (`80%`) |
| `ThreshDesc[name]` | Text passed to the threshold program |
| `ThreshProgI[name]`, `ThreshProgO[name]` | Program run when a bound is broken |
| `ThreshProgOKI[name]`, `ThreshProgOKO[name]` | Program run when it is back within the bound |
| `ThreshMailAddress[name]` | Where threshold mail goes (needs `ThreshMailServer`) |
| `SetEnv[name]` | Environment variables for the threshold programs, `NAME="value"` |
| `ThreshFor[name]` | **New.** Polls a bound must stay broken before the alert fires |

`HWThresh*` keywords need RRDtool's Holt-Winters data; they are accepted with a warning and
ignored. [Alerts](alerts.md) covers how thresholds fire and clear.

## Check a configuration

`linkscope --check /etc/mrtg/mrtg.cfg` reads and checks the file without polling. It does not
take the lock, so it works while the daemon runs. See [Command line](command-line.md).
