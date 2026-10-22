# gNMI devices

LinkScope reads interface counters over SNMP, as MRTG does, and also over gNMI (gRPC Network
Management Interface). Many current routers and switches publish their counters over gNMI, and
some treat it as the main way to read them. A gNMI target works like any other: the same `.log`
files, graphs, thresholds, alerts, `/metrics` and API.

MRTG has no gNMI, so a config that uses it is a LinkScope config. Configs without gNMI targets are
read exactly as before.

## A target

```
Target[srl1_e1-1]: gnmi:ethernet-1/1@srl1.example.net:57400
GnmiOptions[srl1_e1-1]: user=linkscope password-file=/etc/linkscope/srl1.pass ca=/etc/linkscope/ca.pem
MaxBytes[srl1_e1-1]: 1250000000
Title[srl1_e1-1]: srl1 ethernet-1/1
```

`gnmi:<interface>@<host>[:port]` reads one interface: its in and out octet counters, which become
the target's in and out values, as `ifHCInOctets` and `ifHCOutOctets` do for SNMP. The interface
is named as the device names it (`ethernet-1/1`, `Ethernet1`, `xe-0/0/0`, `GigabitEthernet0/0/0/1`).
An IPv6 address goes in brackets (`gnmi:Ethernet1@[2001:db8::1]:6030`). The port defaults to 9339,
gNMI's registered port; many devices use another one:

| Platform | Usual gNMI port |
|---|---|
| Nokia SR Linux | 57400 |
| Arista EOS | 6030 |
| Cisco IOS XR | 57400 (set in its config) |
| Cisco IOS XE, NX-OS | 9339 or 50051 |
| Juniper Junos | 32767 |

A gNMI source can be combined with others in an expression, as MRTG combines SNMP sources:
`Target[both]: gnmi:ethernet-1/1@srl1:57400 + gnmi:ethernet-1/2@srl1:57400`. Leave a space
around each operator.

## GnmiOptions

`GnmiOptions[x]` says how to reach the device: `key=value` pairs separated by spaces. Set them
once for every target that follows with `GnmiOptions[_]:`, as for `SnmpOptions[]`.

| Key | Meaning |
|---|---|
| `user=` | The user name, sent with each request. |
| `password-file=` | A file whose first line is the password. Keep it readable by LinkScope's account only (`chmod 600`); LinkScope warns otherwise. |
| `password-env=` | An environment variable holding the password, instead of a file. |
| `tls=` | `verify` (the default): TLS, and the device's certificate must be trusted and carry its name. `insecure`: TLS without checking the certificate. `off`: no TLS. |
| `ca=` | CA certificates (PEM) to trust for this device, instead of the system's. |
| `tls-name=` | The name to check the certificate against, when the target uses an address. |
| `cert=`, `key=` | A client certificate and key (PEM), for devices that require mutual TLS. |
| `model=` | Which paths to read: `openconfig` (the default) or `srl`. See below. |
| `target=` | The gNMI target name, for devices behind a gNMI gateway. |

The password is never written in the config: `password=` is refused. LinkScope reads the file or
the variable at every cycle, so a changed password needs no restart. The password and the file
paths never appear in the UI, the API, `/metrics`, alerts or the log.

`tls=insecure` and `tls=off` work, but LinkScope warns at start and the target's page shows them:
without a checked certificate, anyone on the path can pretend to be the device, and with `tls=off`
the password travels in clear. Use them in a lab, not on a production network.

## Models

A model is the set of paths a device answers. LinkScope knows two:

| `model=` | In and out counters | Devices |
|---|---|---|
| `openconfig` | `/interfaces/interface[name=…]/state/counters/in-octets`, `out-octets` | Arista EOS, Cisco IOS XR, IOS XE and NX-OS, Juniper Junos, Nokia SR OS, SR Linux with OpenConfig on, and others that publish OpenConfig |
| `srl` | `/interface[name=…]/statistics/in-octets`, `out-octets` | Nokia SR Linux, with or without OpenConfig |

LinkScope also reads each device's name and boot time (shown as System and Uptime) and each
interface's ifIndex (in `/metrics`), when the device answers them; a device that does not still
gets its counters read.

Which platforms have been tested is in the project's plan (`docs/PLAN.md`, "Router models"). A
platform marked "by spec" publishes the OpenConfig counters in its documentation but has not been
run with LinkScope yet.

## How LinkScope reads a device

Once per cycle, LinkScope sends each device one gNMI `Get` for the counters of all its targets'
interfaces, and one more for the name, boot time and ifIndex. A device that does not implement
`Get` is read with a `Subscribe` in `ONCE` mode instead. The connection stays open between
cycles. gNMI devices share SNMP's limits: how many devices are read at once, and how long one
device may take per cycle.

When a device refuses the whole request (most do when one interface does not exist), LinkScope
asks again one interface at a time, so only the target with the wrong name fails.

## Writing targets with cfgmaker

`linkscope cfgmaker --gnmi` writes targets for every interface of gNMI devices, as cfgmaker does
for SNMP:

```
linkscope cfgmaker --gnmi \
  --gnmi-options 'user=linkscope password-file=/etc/linkscope/srl1.pass ca=/etc/linkscope/ca.pem' \
  --global 'WorkDir: /var/lib/linkscope' --output /etc/linkscope/mrtg.cfg \
  srl1.example.net:57400 srl2.example.net:57400
```

It tries OpenConfig first, then SR Linux's model (or only the one given with `model=`), and writes
the model it found into each target's `GnmiOptions[]`. `MaxBytes[]` comes from the port speed.
Interfaces that are down, or whose speed the device does not report, are written commented out;
`--show-op-down` writes the down ones too. Everything the device sends (names, descriptions) is
cleaned to one line before it goes into the config.

## When a target fails

The target's page and the API say why:

| Message | What to check |
|---|---|
| `login refused (wrong user or password)` | `user=` and the password file. |
| `TLS: invalid peer certificate: UnknownIssuer` | The device's certificate is not signed by a CA LinkScope trusts: set `ca=`. |
| `TLS: ... NotValidForName` | The certificate does not carry the name or address in the target: set `tls-name=`. |
| `unreachable: ...`, `connection refused` | The address and port, and whether gNMI is enabled on the device. |
| `the device does not answer these openconfig paths` | The device has no OpenConfig, or not for this interface: try `model=srl`, or check the interface name. |
| `no ... counter in the answer` | The device answered without the counters: check `model=` and the name. |

## Trying it without a router

`linkscope-mockagent gnmi` (built with the project) answers gNMI like a router, with counters that
grow, for trying a config:

```
linkscope-mockagent gnmi --listen 127.0.0.1:9339 --model srl --interfaces ethernet-1/1,ethernet-1/2
```

With no `--cert` and `--key` it speaks plain gRPC, so the target needs `GnmiOptions[x]: tls=off`.
