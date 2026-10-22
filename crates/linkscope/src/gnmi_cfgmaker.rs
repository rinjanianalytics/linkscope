// SPDX-License-Identifier: GPL-2.0-or-later
//! `linkscope cfgmaker --gnmi`: write targets for every interface of gNMI devices, as cfgmaker
//! does for SNMP. It finds which model a device answers (OpenConfig first, then SR Linux's own),
//! lists its interfaces with their state, description and speed, and writes one target per
//! interface with `MaxBytes[]` from the speed. Interfaces that are down, or whose speed is not
//! known, are written commented out, so they can be turned on by hand.
//!
//! Everything a device sends is cleaned to one line before it goes into the config.

use std::process::ExitCode;
use std::time::Duration;

use linkscope_config::GnmiOptions;

use crate::cfgmaker::{fmi, html_escape, one_line};
use crate::gnmi::{DeviceKey, Discovered, discover};

pub const USAGE: &str = "Usage: cfgmaker --gnmi [options] host[:port] [...]

Options:
  --gnmi-options 'TEXT'   how to reach the devices, as GnmiOptions[] (user=, password-file=,
                          tls=, ca=, model=, ...); without model=, the model is detected
  --global 'Key: value'   add a global line (repeatable)
  --output FILE           write the config to FILE instead of stdout
  --show-op-down          also write targets for interfaces that are down
  --timeout SECONDS       per request (default 10)
  --help

The port defaults to 9339 (gNMI's registered port). Each device's targets get their own
GnmiOptions[] line, so devices with different options can share one config.";

#[derive(Debug, Default)]
struct Args {
    options_text: String,
    globals: Vec<String>,
    output: Option<String>,
    show_down: bool,
    timeout: Duration,
    hosts: Vec<(String, u16)>,
}

/// `host`, `host:port`, `[v6]` or `[v6]:port`.
fn host_port(s: &str) -> Result<(String, u16), String> {
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let end = rest.find(']').ok_or(format!("{s}: missing ]"))?;
        let host = format!("[{}]", &rest[..end]);
        match rest[end + 1..].strip_prefix(':') {
            Some(p) => (host, Some(p)),
            None if rest[end + 1..].is_empty() => (host, None),
            None => return Err(format!("{s}: unexpected text after ]")),
        }
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h.to_string(), Some(p)),
            Some(_) => return Err(format!("{s}: write an IPv6 address in brackets")),
            None => (s.to_string(), None),
        }
    };
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._:[]".contains(c))
    {
        return Err(format!("{s}: not a host name or address"));
    }
    let port = match port {
        None => linkscope_config::gnmi::DEFAULT_PORT,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 => n,
            _ => return Err(format!("{s}: the port must be 1 to 65535")),
        },
    };
    Ok((host, port))
}

fn parse(args: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args {
        timeout: Duration::from_secs(10),
        ..Default::default()
    };
    let mut it = args.iter();
    while let Some(x) = it.next() {
        let mut value = |name: &str| -> Result<String, String> {
            it.next().cloned().ok_or(format!("{name} needs a value"))
        };
        match x.as_str() {
            "--gnmi" => {}
            "--help" | "-h" => return Ok(None),
            "--gnmi-options" => a.options_text = value("--gnmi-options")?,
            "--global" => a.globals.push(value("--global")?),
            "--output" => a.output = Some(value("--output")?),
            "--show-op-down" => a.show_down = true,
            "--timeout" => {
                let v = value("--timeout")?;
                let secs: u64 = v
                    .parse()
                    .ok()
                    .filter(|s| (1..=300).contains(s))
                    .ok_or("--timeout must be 1 to 300 seconds")?;
                a.timeout = Duration::from_secs(secs);
            }
            o if o.starts_with('-') => return Err(format!("unknown option {o}")),
            h => a.hosts.push(host_port(h)?),
        }
    }
    Ok(Some(a))
}

/// A target name from the host and interface: anything but letters, digits, `.`, `-` and `_`
/// becomes `_` (`10.20.0.11_ethernet-1_1`).
fn target_name(host: &str, ifname: &str) -> String {
    format!("{host}_{ifname}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The config lines for one device.
pub fn render(
    host: &str,
    port: u16,
    options: &GnmiOptions,
    d: &Discovered,
    show_down: bool,
) -> String {
    let mut opts = options.clone();
    opts.model = d.model;
    let opts_text = opts.to_text();
    let sys = d
        .hostname
        .as_deref()
        .map(one_line)
        .unwrap_or_else(|| host.to_string());
    let mut out = String::new();
    out.push_str(&format!(
        "\n######################################################################\n\
         # Device: {sys} (gNMI {host}:{port}, model {})\n\
         ######################################################################\n",
        d.model.name()
    ));
    for f in &d.interfaces {
        let name = one_line(&f.name);
        let descr = f.description.as_deref().map(one_line).unwrap_or_default();
        let speed_text = f
            .speed_bps
            .map(|b| fmi(b / 8, true))
            .unwrap_or_else(|| "unknown".into());
        out.push_str(&format!(
            "\n### Interface {name} >> Descr: '{}' | Speed: {speed_text} | ifIndex: {} ###\n",
            descr.replace('\'', ""),
            f.if_index
                .as_deref()
                .map(one_line)
                .unwrap_or_else(|| "unknown".into())
        ));
        // A name LinkScope could not write in a Target[] is listed, never written.
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_whitespace() || c == '@' || c == '`')
        {
            out.push_str("# not written: its name cannot be used in a target\n");
            continue;
        }
        let mut why_off = Vec::new();
        if f.up == Some(false) && !show_down {
            why_off.push("it is down");
        }
        if f.speed_bps.unwrap_or(0) == 0 {
            why_off.push("its speed is unknown: set MaxBytes[]");
        }
        let t = target_name(host, &name);
        let title = html_escape(&format!("Traffic analysis for {name} -- {sys}"));
        let max_bytes = f.speed_bps.map(|b| b / 8).unwrap_or(0);
        let lines = [
            format!("Target[{t}]: gnmi:{name}@{host}:{port}"),
            format!("GnmiOptions[{t}]: {opts_text}"),
            format!("MaxBytes[{t}]: {max_bytes}"),
            format!("Title[{t}]: {title}"),
            format!(
                "PageTop[{t}]: <h1>{title}</h1>\n\t\t<div id=\"sysdetails\">\n\t\t\t<table>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>System:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Description:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Interface:</td>\n\t\t\t\t\t<td>{} (read by gNMI, {} model)</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Max Speed:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>\n\t\t\t</table>\n\t\t</div>",
                html_escape(&sys),
                html_escape(&descr),
                html_escape(&name),
                d.model.name(),
                html_escape(&speed_text),
            ),
        ];
        if why_off.is_empty() {
            for l in lines {
                out.push_str(&l);
                out.push('\n');
            }
        } else {
            out.push_str(&format!("# Not polled: {}.\n", why_off.join("; ")));
            for l in lines {
                for part in l.split('\n') {
                    out.push_str("# ");
                    out.push_str(part);
                    out.push('\n');
                }
            }
        }
    }
    out
}

async fn run(a: &Args) -> Result<String, String> {
    let options =
        GnmiOptions::parse(&a.options_text).map_err(|e| format!("--gnmi-options: {e}"))?;
    let model_given = a
        .options_text
        .split_whitespace()
        .any(|kv| kv.to_ascii_lowercase().starts_with("model="));
    let mut out = String::from(
        "# Created by linkscope cfgmaker --gnmi\n\
         # Targets read interface counters over gNMI; see the manual's gNMI page.\n\n",
    );
    for g in &a.globals {
        out.push_str(&one_line(g));
        out.push('\n');
    }
    for (host, port) in &a.hosts {
        let key = DeviceKey {
            host: host.clone(),
            port: *port,
            options: options.clone(),
        };
        let d = discover(&key, model_given, a.timeout).await?;
        out.push_str(&render(host, *port, &options, &d, a.show_down));
    }
    Ok(out)
}

/// `args[0]` is the program's name, as for MRTG's cfgmaker.
pub fn main(args: Vec<String>) -> ExitCode {
    let a = match parse(args.get(1..).unwrap_or_default()) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("ERROR: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if a.hosts.is_empty() {
        eprintln!("ERROR: no devices given\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: {e}");
            return ExitCode::from(1);
        }
    };
    match rt.block_on(run(&a)) {
        Ok(conf) => {
            match &a.output {
                Some(path) => {
                    if let Err(e) = crate::fsutil::write_atomic(
                        std::path::Path::new(path),
                        conf.as_bytes(),
                        0o640,
                    ) {
                        eprintln!("ERROR: creating {path}: {e}");
                        return ExitCode::from(1);
                    }
                }
                None => print!("{conf}"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gnmi::FoundInterface;

    #[test]
    fn arguments() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = parse(&v(&[
            "--gnmi",
            "--gnmi-options",
            "tls=off",
            "--show-op-down",
            "r1:57400",
            "r2",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(a.options_text, "tls=off");
        assert!(a.show_down);
        assert_eq!(a.hosts, vec![("r1".into(), 57400), ("r2".into(), 9339)]);
        assert!(parse(&v(&["--gnmi", "--help"])).unwrap().is_none());
        assert!(parse(&v(&["--gnmi", "--bogus"])).is_err());
    }

    #[test]
    fn hosts_and_ports() {
        assert_eq!(host_port("srl1").unwrap(), ("srl1".into(), 9339));
        assert_eq!(
            host_port("10.0.0.1:57400").unwrap(),
            ("10.0.0.1".into(), 57400)
        );
        assert_eq!(
            host_port("[2001:db8::1]:6030").unwrap(),
            ("[2001:db8::1]".into(), 6030)
        );
        assert!(host_port("2001:db8::1").is_err());
        assert!(host_port("r1:0").is_err());
        assert!(host_port("r1;rm -rf").is_err());
    }

    #[test]
    fn device_text_cannot_add_lines() {
        let d = Discovered {
            model: linkscope_config::GnmiModel::Srl,
            hostname: Some("srl1\nTarget[evil]: `rm -rf /`".into()),
            interfaces: vec![
                FoundInterface {
                    name: "ethernet-1/1".into(),
                    description: Some("uplink\nThreshProgI[x]: /bin/sh".into()),
                    up: Some(true),
                    speed_bps: Some(10_000_000_000),
                    if_index: Some("16".into()),
                },
                FoundInterface {
                    name: "ethernet-1/2".into(),
                    up: Some(false),
                    speed_bps: Some(1_000_000_000),
                    ..Default::default()
                },
                FoundInterface {
                    name: "bad name@x".into(),
                    up: Some(true),
                    speed_bps: Some(1_000_000_000),
                    ..Default::default()
                },
            ],
        };
        let text = render("10.20.0.11", 57400, &GnmiOptions::default(), &d, false);
        for line in text.lines() {
            assert!(
                line.is_empty()
                    || line.starts_with('#')
                    || line.starts_with('\t')
                    || line.contains('['),
                "{line}"
            );
            assert!(
                !line.starts_with("Target[evil]") && !line.starts_with("ThreshProgI"),
                "{line}"
            );
        }
        assert!(
            text.contains("Target[10.20.0.11_ethernet-1_1]: gnmi:ethernet-1/1@10.20.0.11:57400\n")
        );
        assert!(text.contains("GnmiOptions[10.20.0.11_ethernet-1_1]: model=srl\n"));
        assert!(text.contains("MaxBytes[10.20.0.11_ethernet-1_1]: 1250000000\n"));
        assert!(text.contains("# Not polled: it is down.\n# Target[10.20.0.11_ethernet-1_2]"));
        assert!(!text.contains("Target[10.20.0.11_bad"));
    }
}
