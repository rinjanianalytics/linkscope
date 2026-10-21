# cfgmaker

`cfgmaker` asks a device for its interfaces and writes a configuration for them, the way MRTG's
cfgmaker does. LinkScope has it built in, so there is no separate program to install.

## Run it

```sh
linkscope cfgmaker --global 'WorkDir: /var/lib/mrtg' --ifref=name \
    --output /etc/mrtg/mrtg.cfg public@192.0.2.1:::::2
```

A symlink named `cfgmaker` that points at `linkscope` works too:

```sh
ln -s /usr/local/bin/linkscope /usr/local/bin/cfgmaker
cfgmaker public@192.0.2.1:::::2
```

Without `--output`, the configuration goes to standard output. With it, LinkScope writes the
file with mode 0640, so the SNMP credentials in it are not readable by every account.

## Devices

Each device is one argument:

```text
[community@]router[:[port]:[timeout]:[retries]:[backoff]:[version]]
```

The fields after the name are the same as in a `Target[]` (see
[Configuration](configuration.md#connection-settings)). Without a community, cfgmaker uses
`--community`, or `public`. An IPv6 address goes in brackets: `public@[2001:db8::1]:::::2`.

### Use SNMP version 2 or 3

Without a version, cfgmaker writes SNMPv1 targets, as MRTG's does. Version 1 counters are
32 bits and wrap every 34 seconds at 1 Gbit/s, so the graphs and the [Prometheus
metrics](metrics.md) of a busy link go wrong. Give the version as the fifth field, with five
colons after the name:

```sh
linkscope cfgmaker public@192.0.2.1:::::2
```

### SNMPv3

Give the user and keys as options, and version 3 for the device:

```sh
linkscope cfgmaker --username linkscope --authprotocol sha256 \
    --authpassword 'example-auth-pass' --privprotocol aes \
    --privpassword 'example-priv-pass' 192.0.2.1:::::3
```

The options are written into each target's `SnmpOptions[]`. They take effect only for devices
with version 3; for other devices they are left out. The command line in the file's header shows
the passwords, keys and communities (options and `community@router`) as `XXXXX`.

cfgmaker writes at most 50,000 interfaces for one device; a device reporting more is an error.

## Options

Options apply to the devices that follow them on the command line, so different devices can
have different settings.

| Option | Meaning |
|---|---|
| `--global 'Key: value'` | Add a global line to the output. Repeat it for several lines. |
| `--output FILE` | Write the configuration to `FILE` instead of standard output |
| `--ifref=TYPE[,TYPE...]` | How targets refer to interfaces: `nr` (ifIndex), `ip`, `eth` (MAC), `descr`, `name` or `type`. The first that is unique for the interface wins. |
| `--ifdesc=TYPE[,TYPE...]` | How titles describe interfaces: `nr`, `ip`, `eth`, `descr`, `name`, `alias` or `type` |
| `--if-filter=EXPR` | Only include interfaces for which `EXPR` is true (see below) |
| `--community=STRING` | Community for devices written without one (default `public`) |
| `--snmp-options=:[port]:[timeout]:[retries]:[backoff]:[version]` | Connection fields for devices written without them |
| `--dns-domain=DOMAIN` | Append `DOMAIN` to the device names |
| `--no-down` | Do not skip interfaces that are down |
| `--show-op-down` | Include interfaces that are operationally down |
| `--noreversedns` | Do not look up the names of interface addresses |
| `--subdirs=FORMAT` | Put each device's logs in a subdirectory; `HOSTNAME` and `SNMPNAME` in `FORMAT` are replaced |
| `--nointerfaces` | Write only the device header, no interface targets |
| `--zero-speed=BPS` | Speed to assume for interfaces that report none |
| `--enable-ipv6` | Allow SNMP over IPv6 (also writes `EnableIPv6: yes`) |
| `--username`, `--authprotocol`, `--authpassword`, `--privprotocol`, `--privpassword`, `--contextname` | SNMPv3 settings (see above) |
| `--help`, `--version` | Show the usage or the version |

Without `--ifref`, interfaces are referred to by ifName where the device reports one, and by
ifIndex otherwise. `--ifref=name` makes the choice explicit. An interface reference that cannot
be written safely (for example a name with a backtick) falls back to the ifIndex.

This is what MRTG 2.17.10's cfgmaker does too, and what its `--help` says
(`--ifref=name ... (default)`). MRTG's longer manual page still says the default is `nr`;
that sentence is out of date. To keep ifIndex references, pass `--ifref=nr`.

Interfaces cfgmaker skips (down, a type it does not graph, a filter that rejected them) are still
written, commented out, with the reason above them. Remove the `#` to use one anyway.

## Interface filters

`--if-filter` takes a small expression language that covers the filters MRTG documents:

- variables such as `$default`, `$if_type`, `$if_is_ethernet`, `$if_admin`, `$if_oper`,
  `$if_speed`, `$if_snmp_descr`, `$if_snmp_name`, `$if_snmp_alias`, `$if_ip` and `$sysname`;
- numbers and quoted strings;
- `!`, `not`, `&&`, `and`, `||`, `or` and parentheses;
- numeric comparisons `== != < <= > >=`, string comparisons `eq ne lt gt le ge`;
- regular expression matches `=~ /re/i` and `!~ /re/`.

```sh
# Ethernet interfaces cfgmaker would include anyway, faster than 100 Mbit/s
linkscope cfgmaker --if-filter='$default && $if_is_ethernet && $if_speed > 12500000' \
    public@192.0.2.1:::::2

# Only interfaces whose alias mentions an uplink
linkscope cfgmaker --if-filter='$if_snmp_alias =~ /uplink/i' public@192.0.2.1:::::2
```

`$if_speed` is in bytes per second. MRTG ran the filter as Perl; LinkScope accepts only this
language. `--if-template` and `--host-template` run Perl programs and are refused: use
`--if-filter` and `--global`, or edit the generated file.

## Safe output

Device strings (sysName, sysDescr, ifDescr, ifName, ifAlias and the rest) come from the device,
so cfgmaker treats them as untrusted. It removes control characters, so a device cannot add lines
to your configuration, and it escapes values that end up in HTML. See [Security](security.md).

## Not supported

- `--if-template` and `--host-template` (Perl programs).
- Cisco VLAN columns: `$if_vlan_id` and `$if_cisco_trunk` are not available in filters, and the
  `catname` and `ppname` descriptions are not supported.
- Localized SNMPv3 keys (`--authkey`, `--privkey`): use the passwords.
