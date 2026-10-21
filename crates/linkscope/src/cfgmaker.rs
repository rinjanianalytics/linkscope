// SPDX-License-Identifier: GPL-2.0-or-later
//! `cfgmaker`: discover a device's interfaces and write an MRTG config.
//!
//! Output follows MRTG's cfgmaker (`GenConf`) so existing workflows keep
//! working: the same header, per-interface stanzas, problem comments for
//! skipped interfaces and the same option names.
//!
//! Security fixes:
//! * every string that comes from the device (sysName, sysDescr, ifDescr,
//!   ifName, ifAlias, ...) is stripped of control characters before it is
//!   written, so a device cannot inject extra config lines;
//! * interface references that contain characters the Target grammar cannot
//!   carry safely (such as backticks) are replaced by the ifIndex, so a
//!   device cannot smuggle an external command into `Target[]`;
//! * HTML-bound values (including sysName, which MRTG left raw) are escaped;
//! * `--if-filter` is a small safe expression language instead of Perl;
//!   `--if-template` and `--host-template` (Perl programs) are refused;
//! * passwords are masked in the "Created by" header and the output file is
//!   created with mode 0640.

use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::time::Duration;

use linkscope_config::{IfSel, SnmpConn, SnmpOptions, SnmpSource, SnmpVersion};

use crate::filter::{FVal, Filter};

/// A regex compiled once per call site.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new($p).expect("static regex"));
        &*R
    }};
}
use crate::iftype_table::IF_TYPES;
use crate::oids::OidResolver;
use crate::snmp::{Endpoint, Session, Val};

const USAGE: &str = "Usage: cfgmaker [options] [community@]router[:[port]:[timeout]:[retries]:[backoff]:[version]] [...]

Options (apply to the routers that follow them):
  --global 'Key: value'   add a global line (repeatable)
  --output FILE           write the config to FILE instead of stdout
  --ifref=TYPE[,TYPE..]   reference interfaces by nr, ip, eth, descr, name or type
  --ifdesc=TYPE[,..]      describe interfaces by nr, ip, eth, descr, name, alias, type
  --if-filter=EXPR        include interfaces where EXPR is true (safe subset of Perl)
  --community=STRING      SNMP community (default public)
  --snmp-options=:[port]:[timeout]:[retries]:[backoff]:[version]
  --dns-domain=DOMAIN     append DOMAIN to router names
  --no-down               do not skip down interfaces
  --show-op-down          include operationally down interfaces
  --noreversedns          do not look up interface IP names
  --subdirs=FORMAT        put each router in a subdirectory (HOSTNAME, SNMPNAME)
  --nointerfaces          only write the router header
  --zero-speed=BPS        speed for interfaces that report none
  --enable-ipv6           allow SNMP over IPv6
  --enablesnmpv3 --username=U --authprotocol=md5|sha|sha256 --authpassword=P
      --privprotocol=des|aes|aes256 --privpassword=P --contextname=C
  --help, --version";

#[derive(Debug, Clone, Default)]
struct Opts {
    ifref: Option<String>,
    ifdesc: Option<String>,
    if_filter: Option<String>,
    community: Option<String>,
    snmp_options: Option<String>,
    dns_domain: Option<String>,
    no_down: bool,
    show_op_down: bool,
    noreversedns: bool,
    subdirs: Option<String>,
    no_interfaces: bool,
    zero_speed: Option<u64>,
    v3: SnmpOptions,
    globals: Vec<String>,
    bits: bool,
}

#[derive(Debug, Clone)]
struct RouterSpec {
    community: String,
    name: String,
    snmpopt: String,
    opts: Opts,
    globals: Vec<String>,
}

struct Parsed {
    routers: Vec<RouterSpec>,
    trailing_globals: Vec<String>,
    output: Option<String>,
    enable_ipv6: bool,
    enablesnmpv3: bool,
    fullcmd: String,
}

/// Options that take no value (every other `--option` takes one, inline or as the next word).
fn flag_only(key: &str) -> bool {
    matches!(
        key,
        "no-down" | "show-op-down" | "noreversedns" | "nointerfaces" | "interfaces" | "enable-ipv6" | "enablesnmpv3" | "help" | "version" | "use-16bit" | "nodefaultglobal"
    )
}

/// Options whose value is a secret.
fn secret_option(key: &str) -> bool {
    matches!(key, "authpassword" | "privpassword" | "authkey" | "privkey" | "community")
}

/// The command line for the generated file's header, without its secrets: the values of the
/// password, key and community options, and the community in front of each `community@router`.
/// Arguments are read as `parse_args` reads them, so an option's value (a `--global` with an
/// address in it) is never taken for a router.
fn mask_secrets(args: &[String]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            let (key, inline) = match stripped.split_once('=') {
                Some((k, _)) => (k.to_lowercase(), true),
                None => (stripped.to_lowercase(), false),
            };
            let hide = secret_option(&key);
            match (inline, hide) {
                (true, true) => out.push(format!("{}=XXXXX", a.split_once('=').map_or(a.as_str(), |(k, _)| k))),
                _ => out.push(a.clone()),
            }
            if !inline && !flag_only(&key) {
                if let Some(v) = args.get(i + 1) {
                    out.push(if hide { "XXXXX".to_string() } else { v.clone() });
                    i += 1;
                }
            }
        } else if let Some((_, router)) = a.rsplit_once('@') {
            out.push(format!("XXXXX@{router}"));
        } else {
            out.push(a.clone());
        }
        i += 1;
    }
    out.join(" ")
}

/// Most interfaces cfgmaker writes for one device. Each takes about 2 KB of configuration, and
/// a device reporting a million would make one string of gigabytes.
const MAX_INTERFACES: usize = 50_000;

fn interface_count_ok(n: usize, who: &str) -> Result<(), String> {
    if n > MAX_INTERFACES {
        return Err(format!("{who} reports {n} interfaces, more than cfgmaker writes for one device ({MAX_INTERFACES}); check the device"));
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Option<Parsed>, String> {
    let mut opts = Opts::default();
    let mut p = Parsed {
        routers: Vec::new(),
        trailing_globals: Vec::new(),
        output: None,
        enable_ipv6: false,
        enablesnmpv3: false,
        fullcmd: format!("cfgmaker {}", mask_secrets(&args[1..])),
    };
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            let (key, inline) = match stripped.split_once('=') {
                Some((k, v)) => (k.to_lowercase(), Some(v.to_string())),
                None => (stripped.to_lowercase(), None),
            };
            let takes_value = !flag_only(&key);
            let val = if takes_value {
                match inline {
                    Some(v) => Some(v),
                    None => {
                        i += 1;
                        Some(args.get(i).cloned().ok_or(format!("--{key} needs a value"))?)
                    }
                }
            } else {
                None
            };
            let v = val.unwrap_or_default();
            match key.as_str() {
                "help" => return Ok(None),
                "version" => {
                    println!("cfgmaker (LinkScope {})", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "global" => opts.globals.push(v),
                "output" => p.output = Some(v),
                "ifref" => opts.ifref = Some(v),
                "ifdesc" => opts.ifdesc = Some(v),
                "if-filter" => {
                    Filter::compile(&v)?;
                    opts.if_filter = Some(v);
                }
                "if-template" | "host-template" => {
                    return Err(format!(
                        "--{key} runs a Perl program and is not supported by LinkScope; use --if-filter and --global, or edit the generated file"
                    ));
                }
                "community" => opts.community = Some(v),
                "snmp-options" => opts.snmp_options = Some(v),
                "dns-domain" => opts.dns_domain = Some(v),
                "no-down" => opts.no_down = true,
                "show-op-down" => opts.show_op_down = true,
                "noreversedns" => opts.noreversedns = true,
                "subdirs" => opts.subdirs = Some(v),
                "nointerfaces" => opts.no_interfaces = true,
                "interfaces" => opts.no_interfaces = false,
                "zero-speed" => opts.zero_speed = Some(v.parse().map_err(|_| format!("zero-speed specified with non-integer speed: {v}"))?),
                "enable-ipv6" => p.enable_ipv6 = true,
                "enablesnmpv3" => p.enablesnmpv3 = true,
                "use-16bit" | "nodefaultglobal" => {}
                "username" | "authpassword" | "authprotocol" | "privpassword" | "privprotocol" | "contextname" | "contextengineid" | "authkey" | "privkey" => {
                    if key == "username" {
                        p.enablesnmpv3 = true;
                    }
                    opts.v3.insert(key.clone(), v);
                }
                "debug" | "snmp-debug" => {}
                other => return Err(format!("unknown option --{other}")),
            }
            if key == "global" && opts.globals.last().is_some_and(|g| g.to_lowercase().contains("options[_]") && g.to_lowercase().contains("bits")) {
                opts.bits = true;
            }
        } else {
            let (community, rest) = match a.rsplit_once('@') {
                Some((c, r)) => (c.to_string(), r.to_string()),
                None => (opts.community.clone().unwrap_or_else(|| "public".into()), a.clone()),
            };
            let (name, mut snmpopt) = if rest.starts_with('[') {
                let end = rest.find(']').ok_or("unterminated IPv6 address")? + 1;
                (rest[..end].to_string(), rest[end..].to_string())
            } else {
                match rest.split_once(':') {
                    Some((h, o)) => (h.to_string(), format!(":{o}")),
                    None => (rest, String::new()),
                }
            };
            if snmpopt.is_empty() {
                if let Some(o) = &opts.snmp_options {
                    snmpopt = if o.starts_with(':') { o.clone() } else { format!(":{o}") };
                }
            }
            if name.is_empty() {
                return Err(format!("no router name in '{a}'"));
            }
            p.routers.push(RouterSpec { community, name, snmpopt, opts: opts.clone(), globals: std::mem::take(&mut opts.globals) });
        }
        i += 1;
    }
    p.trailing_globals = opts.globals;
    Ok(Some(p))
}

/// System information (DeviceInfo).
#[derive(Debug, Default, Clone)]
struct DevInfo {
    descr: String,
    object_id: String,
    contact: String,
    name: String,
    location: String,
    vendor: String,
}

#[derive(Debug, Default, Clone)]
struct IfInfo {
    if_type: i64,
    admin: i64,
    oper: i64,
    mtu: Option<i64>,
    speed: Option<u64>,
    alias: Option<String>,
    descr: Option<String>,
    name: Option<String>,
    ip: Option<String>,
    eth: Option<String>,
    type_key: Option<String>,
    dns_name: Option<String>,
    counter_problem: Option<String>,
    nohc: bool,
}

const VENDORS: &[(&str, &str)] = &[
    ("1.3.6.1.4.1.11863.", "tplink"),
    ("1.3.6.1.4.1.43.", "3com"),
    ("1.3.6.1.4.1.11.", "hp"),
    ("1.3.6.1.4.1.9.", "cisco"),
    ("1.3.6.1.4.1.171.", "dlink"),
    ("1.3.6.1.4.1.6141.", "wwp"),
    ("1.3.6.1.4.1.674.10895.", "dellLan"),
    ("1.3.6.1.4.1.1916.", "extremenetworks"),
    ("1.3.6.1.4.1.1991.", "foundry"),
    ("1.3.6.1.4.1.6027.", "force10"),
    ("1.3.6.1.4.1.2636.", "juniper"),
    ("1.3.6.1.4.1.94.", "nokiaipsofw"),
    ("1.3.6.1.4.1.307.", "portmaster"),
    ("1.3.6.1.4.1.890.", "zyxel"),
    ("1.3.6.1.4.1.2272.30", "nortel"),
    ("1.3.6.1.4.1.6339", "DCN"),
    ("1.3.6.1.4.1.30155.", "openBSD"),
    ("1.3.6.1.4.1.30065.", "arista"),
    ("1.3.6.1.4.1.5624.", "enterasys"),
    ("1.3.6.1.4.1.30803.", "Vyatta"),
    ("1.3.6.1.4.1.3955.", "LinkSys"),
    ("1.3.6.1.4.1.1588.", "brocade"),
    ("1.3.6.1.4.1.3709.", "datacom"),
    ("1.3.6.1.4.1.14988.", "mikrotik"),
    ("1.3.6.1.4.1.6486.", "alcatel"),
    ("1.3.6.1.4.1.2011.", "huawei"),
    ("1.3.6.1.4.1.35265.", "eltex"),
    ("1.3.6.1.4.1.4413", "ubiquiti"),
    ("1.3.6.1.4.1.41112", "ubiquiti"),
];

/// Vendors for which MRTG's cfgmaker walks ifAlias.
fn alias_vendor(v: &str) -> bool {
    v == "cisco"
        || re!(r"(?i)(?:hp|juniper|dlink|wwp|foundry|dellLan|force10|3com|extremenetworks|openBSD|arista|enterasys|zyxel|vyatta|dcn|brocade|datacom|alcatel|mikrotik|huawei|eltex|ubiquiti)")
            .is_match(v)
}

/// Replace control characters (including newlines) with spaces: a device
/// string must never start a new config line.
pub fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// HTML-escape for PageTop/Title (MRTG escaped `& < >`; quotes are escaped
/// too, and newlines can no longer occur).
pub fn html_escape(s: &str) -> String {
    one_line(s).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `fmi` from cfgmaker: human readable speed.
fn fmi(number: u64, bits: bool) -> String {
    let (n, short): (u64, [&str; 4]) = if bits {
        (number * 8, ["bits/s", "kbits/s", "Mbits/s", "Gbits/s"])
    } else {
        (number, ["Bytes/s", "kBytes/s", "MBytes/s", "GBytes/s"])
    };
    let digits = n.to_string().len();
    let mut divm = 0;
    while digits as i64 - divm as i64 * 3 > 4 && divm < 3 {
        divm += 1;
    }
    format!("{:.1} {}", n as f64 / 10f64.powi(divm as i32 * 3), short[divm])
}

fn if_type_name(t: i64) -> String {
    IF_TYPES.iter().find(|(n, _)| i64::from(*n) == t).map(|(_, s)| s.to_string()).unwrap_or_default()
}

async fn reverse_dns(ip: &str) -> Option<String> {
    let addr: std::net::IpAddr = ip.parse().ok()?;
    let fut = tokio::task::spawn_blocking(move || {
        let sa: std::net::SocketAddr = (addr, 0).into();
        let (ss, len) = match sa {
            std::net::SocketAddr::V4(v4) => {
                // SAFETY: sockaddr_in is plain data; zeroed is a valid value.
                let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
                s.sin_family = libc::AF_INET as libc::sa_family_t;
                s.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
                // SAFETY: sockaddr_in fits in sockaddr_storage.
                let mut st: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
                // SAFETY: copying a sockaddr_in into a zeroed sockaddr_storage of larger size.
                unsafe { std::ptr::copy_nonoverlapping(&s as *const _ as *const u8, &mut st as *mut _ as *mut u8, std::mem::size_of::<libc::sockaddr_in>()) };
                (st, std::mem::size_of::<libc::sockaddr_in>())
            }
            std::net::SocketAddr::V6(_) => return None,
        };
        let mut host = [0 as libc::c_char; 256];
        // SAFETY: valid sockaddr pointer/length and output buffer.
        let rc = unsafe {
            libc::getnameinfo(&ss as *const _ as *const libc::sockaddr, len as libc::socklen_t, host.as_mut_ptr(), host.len() as libc::socklen_t, std::ptr::null_mut(), 0, libc::NI_NAMEREQD)
        };
        if rc != 0 {
            return None;
        }
        // SAFETY: getnameinfo NUL-terminates on success.
        Some(unsafe { std::ffi::CStr::from_ptr(host.as_ptr()) }.to_string_lossy().into_owned())
    });
    tokio::time::timeout(Duration::from_secs(3), fut).await.ok()?.ok()?
}

fn endpoint_for(r: &RouterSpec, snmpopt: &str, enable_ipv6: bool) -> Result<Endpoint, String> {
    let conn = SnmpConn::parse(snmpopt)?;
    let mut o = r.opts.v3.clone();
    if conn.version != SnmpVersion::V3 {
        o.clear();
    }
    let src = SnmpSource {
        community: r.community.clone(),
        host: match &r.opts.dns_domain {
            Some(d) if !d.is_empty() => format!("{}.{d}", r.name),
            _ => r.name.clone(),
        },
        snmp_info: snmpopt.to_string(),
        conn,
        conversion: None,
        oid: [String::new(), String::new()],
        sel: [IfSel::None, IfSel::None],
        ipv4only: !enable_ipv6,
        snmp_options: o,
    };
    Endpoint::from_source(&src, false)
}

async fn discover(r: &RouterSpec, snmpopt: &str, enable_ipv6: bool, oids: &OidResolver) -> Result<(DevInfo, BTreeMap<u64, IfInfo>), String> {
    let ep = endpoint_for(r, snmpopt, enable_ipv6)?;
    let mut s = Session::open(&ep).await.map_err(|e| e.to_string())?;
    let sys = s.walk("1.3.6.1.2.1.1").await.map_err(|e| format!("{e}"))?;
    if sys.is_empty() {
        return Err(format!("no response from {}", ep.display()));
    }
    let mut dev = DevInfo::default();
    for (suffix, v) in &sys {
        let text = v.pretty();
        match suffix.as_str() {
            "1.0" => dev.descr = text,
            "2.0" => dev.object_id = text,
            "4.0" => dev.contact = text,
            "5.0" => dev.name = text,
            "6.0" => dev.location = text,
            _ => {}
        }
    }
    dev.vendor = VENDORS
        .iter()
        .filter(|(p, _)| dev.object_id.trim_start_matches('.').starts_with(p))
        .max_by_key(|(p, _)| p.len())
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| format!("Unknown Vendor - {}", dev.object_id));

    let mut ifs: BTreeMap<u64, IfInfo> = BTreeMap::new();
    let num = |v: &Val| v.number().and_then(|n| n.parse::<i64>().ok());
    let mut walk_col = |rows: Vec<(String, Val)>, f: &mut dyn FnMut(&mut IfInfo, &Val)| {
        for (suffix, v) in rows {
            if let Ok(idx) = suffix.parse::<u64>() {
                f(ifs.entry(idx).or_default(), &v);
            }
        }
    };
    let col = |name: &str| oids.resolve(name).expect("built-in OID");
    let w = |e: crate::snmp::SnmpError| e.to_string();
    walk_col(s.walk(&col("ifIndex")).await.map_err(w)?, &mut |_, _| {});
    walk_col(s.walk(&col("ifType")).await.map_err(w)?, &mut |i, v| i.if_type = num(v).unwrap_or(-1));
    walk_col(s.walk(&col("ifAdminStatus")).await.map_err(w)?, &mut |i, v| i.admin = num(v).unwrap_or(0));
    walk_col(s.walk(&col("ifOperStatus")).await.map_err(w)?, &mut |i, v| i.oper = num(v).unwrap_or(0));
    walk_col(s.walk(&col("ifMtu")).await.map_err(w)?, &mut |i, v| i.mtu = num(v));
    if alias_vendor(&dev.vendor) {
        walk_col(s.walk(&col("ifAlias")).await.unwrap_or_default(), &mut |i, v| i.alias = Some(crate::confcache::trim_ctl(&v.pretty()).to_string()));
    }
    let speeds = s.walk(&col("ifSpeed")).await.map_err(w)?;
    let high: HashMap<String, u64> = if ep.version != SnmpVersion::V1 {
        s.walk(&col("ifHighSpeed")).await.unwrap_or_default().into_iter().filter_map(|(k, v)| v.number().and_then(|n| n.parse().ok()).map(|n| (k, n))).collect()
    } else {
        HashMap::new()
    };
    for (suffix, v) in speeds {
        let Ok(idx) = suffix.parse::<u64>() else { continue };
        let mut sp = v.number().and_then(|n| n.parse::<u64>().ok());
        if ep.version != SnmpVersion::V1 && (sp.is_none() || sp == Some(u64::from(u32::MAX))) {
            sp = high.get(&suffix).and_then(|h| h.checked_mul(1_000_000));
        }
        ifs.entry(idx).or_default().speed = sp;
    }

    // Unique identifiers, MRTG confcache style: a value shared by two
    // interfaces cannot identify either of them.
    let mut uniq = |rows: Vec<(String, Val)>, key_of_row: &dyn Fn(&str, &Val) -> Option<(String, u64)>, set: &dyn Fn(&mut IfInfo, String)| {
        let mut seen: HashMap<String, Option<u64>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (suffix, v) in &rows {
            if let Some((k, idx)) = key_of_row(suffix, v) {
                match seen.get(&k) {
                    None => {
                        seen.insert(k.clone(), Some(idx));
                        order.push(k);
                    }
                    Some(Some(prev)) if *prev != idx => {
                        seen.insert(k, None);
                    }
                    _ => {}
                }
            }
        }
        for k in order {
            if let Some(Some(idx)) = seen.get(&k) {
                if let Some(i) = ifs.get_mut(idx) {
                    set(i, k);
                }
            }
        }
    };
    let text_key = |suffix: &str, v: &Val| -> Option<(String, u64)> {
        let t: String = crate::confcache::trim_ctl(&v.pretty()).chars().map(|c| if c <= ' ' { ' ' } else { c }).collect();
        Some((t, suffix.parse().ok()?))
    };
    uniq(s.walk(&col("ifName")).await.unwrap_or_default(), &text_key, &|i, k| i.name = Some(k));
    uniq(s.walk(&col("ifDescr")).await.unwrap_or_default(), &text_key, &|i, k| i.descr = Some(k));
    uniq(s.walk(&col("ifType")).await.unwrap_or_default(), &text_key, &|i, k| i.type_key = Some(k));
    uniq(
        s.walk(&col("ipAdEntIfIndex")).await.unwrap_or_default(),
        &|suffix, v| Some((suffix.to_string(), v.number()?.parse().ok()?)),
        &|i, k| {
            if i.ip.is_none() {
                i.ip = Some(k);
            }
        },
    );
    uniq(
        s.walk(&col("ifPhysAddress")).await.unwrap_or_default(),
        &|suffix, v| match v {
            Val::Str(b) if !b.is_empty() => Some((crate::confcache::mac_string(b), suffix.parse().ok()?)),
            _ => None,
        },
        &|i, k| i.eth = Some(k),
    );

    interface_count_ok(ifs.len(), &ep.display())?;

    // Counter check (IsCounterBroken), batched.
    let idxs: Vec<u64> = ifs.keys().copied().collect();
    if ep.version != SnmpVersion::V1 {
        let q: Vec<String> = idxs.iter().flat_map(|i| [format!("{}.{i}", col("ifHighSpeed")), format!("{}.{i}", col("ifHCInOctets"))]).collect();
        let v = s.get(&q).await.unwrap_or_default();
        for (k, idx) in idxs.iter().enumerate() {
            let ok = v.get(2 * k).is_some_and(|x| x.is_present()) && v.get(2 * k + 1).and_then(|x| x.number()).is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
            if !ok {
                ifs.get_mut(idx).expect("known").nohc = true;
            }
        }
    }
    let need_v1: Vec<u64> = idxs.iter().copied().filter(|i| ep.version == SnmpVersion::V1 || ifs[i].nohc).collect();
    if !need_v1.is_empty() {
        let q: Vec<String> = need_v1.iter().map(|i| format!("{}.{i}", col("ifInOctets"))).collect();
        match s.get(&q).await {
            Ok(v) => {
                for (k, idx) in need_v1.iter().enumerate() {
                    let ok = v.get(k).and_then(|x| x.number()).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
                    if !ok {
                        ifs.get_mut(idx).expect("known").counter_problem = Some(format!("No counter exists for {idx}"));
                    }
                }
            }
            Err(e) => {
                for idx in &need_v1 {
                    ifs.get_mut(idx).expect("known").counter_problem = Some(e.to_string());
                }
            }
        }
    }
    if !r.opts.noreversedns {
        // Look names up concurrently (each lookup has a 3 s limit).
        let ips: Vec<(u64, String)> = ifs.iter().filter_map(|(k, i)| i.ip.clone().map(|ip| (*k, ip))).collect();
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
        let mut set = tokio::task::JoinSet::new();
        for (k, ip) in ips {
            let sem = sem.clone();
            set.spawn(async move {
                let _p = sem.acquire_owned().await;
                (k, reverse_dns(&ip).await)
            });
        }
        while let Some(Ok((k, name))) = set.join_next().await {
            if let Some(i) = ifs.get_mut(&k) {
                i.dns_name = name;
            }
        }
    }
    Ok((dev, ifs))
}

/// Undo [`safe_ref`] escaping.
fn unescape_ref(r: &str) -> String {
    re!(r"\\([& :@])").replace_all(r, "$1").into_owned()
}

fn trim_if_ref(r: &str) -> String {
    let re = re!(r"[#!/\\:\s@%]+");
    let s = re.replace_all(r, "_").into_owned();
    let s = s.trim_matches('_');
    let s = if s.is_empty() { "_" } else { s };
    // Characters that would break `Key[name]:` lines or file names.
    s.chars().map(|c| if c.is_control() || matches!(c, '[' | ']' | '`' | '"' | '\'') { '_' } else { c }).collect()
}

/// Build an interface reference for `Target[]`, or `None` if the value
/// cannot be represented safely.
fn safe_ref(prefix: &str, value: &str) -> Option<String> {
    if value.is_empty() || value.chars().any(|c| c.is_control() || c == '`' || c == '\\') {
        return None;
    }
    let esc: String = value.chars().flat_map(|c| if matches!(c, '&' | ' ' | ':' | '@') { vec!['\\', c] } else { vec![c] }).collect();
    Some(format!("{prefix}{esc}"))
}

#[allow(clippy::too_many_arguments)]
fn gen_router(conf: &mut String, r: &RouterSpec, snmpopt: &str, dev: &DevInfo, ifs: &BTreeMap<u64, IfInfo>, enable_ipv6: bool, ipv4only: bool) -> Result<(), String> {
    let sysname = one_line(&dev.name);
    let comment_descr = one_line(&dev.descr);
    let community = r.community.replace('\\', "\\\\").replace('@', "\\@").replace(' ', "\\ ");
    let router_name = match &r.opts.dns_domain {
        Some(d) if !d.is_empty() => format!("{}.{d}", r.name),
        _ => r.name.clone(),
    };
    let router_connect = format!("{community}@{router_name}{snmpopt}");
    let v3 = SnmpConn::parse(snmpopt).map(|c| c.version == SnmpVersion::V3).unwrap_or(false);
    let v3options = r
        .opts
        .v3
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "contextengineid"))
        .map(|(k, v)| format!("{k}=>'{}'", v.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(",");
    let directory = r.opts.subdirs.as_ref().map(|f| {
        let snmp_name: String = sysname.chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
        f.replace("HOSTNAME", &router_name).replace("SNMPNAME", &snmp_name).replace("..", "_")
    });
    conf.push_str(&format!(
        "\n######################################################################\n# System: {sysname}\n# Description: {comment_descr}\n# Contact: {}\n# Location: {}\n######################################################################\n\n\n",
        one_line(&dev.contact),
        one_line(&dev.location)
    ));
    if r.opts.no_interfaces {
        return Ok(());
    }
    let filter = r.opts.if_filter.as_deref().map(Filter::compile).transpose()?;
    let cisco = dev.vendor == "cisco";
    for (&idx, i) in ifs {
        let mut prob: Vec<String> = Vec::new();
        let mut default_ifstate = true;
        let mut default_iftype = true;
        let mut if_ok = true;
        if !r.opts.no_down {
            match i.admin {
                2 => {
                    prob.push("it is administratively DOWN".into());
                    default_ifstate = false;
                }
                3 => {
                    prob.push("it is in administrative TEST mode".into());
                    default_ifstate = false;
                }
                _ => {}
            }
            if !r.opts.show_op_down {
                match i.oper {
                    2 => {
                        prob.push("it is operationally DOWN".into());
                        default_ifstate = false;
                    }
                    3 => {
                        prob.push("it is in operational TEST mode".into());
                        default_ifstate = false;
                    }
                    _ => {}
                }
            }
        }
        let descr_raw = i.descr.clone().unwrap_or_default();
        match i.if_type {
            18 if cisco => {
                prob.push("it is a DS1 controllers".into());
                default_iftype = false;
            }
            19 if cisco => {
                prob.push("it is a E1 controllers".into());
                default_iftype = false;
            }
            24 => {
                prob.push("it is a Software Loopback interface".into());
                default_iftype = false;
            }
            30 if cisco => {
                prob.push("it is a DS3 controller".into());
                default_iftype = false;
            }
            102 if cisco => {
                prob.push("it is a Voice controller".into());
                default_iftype = false;
            }
            103 if cisco => {
                prob.push("it is a Voice dial peer".into());
                default_iftype = false;
            }
            162 => prob.push("it is a CEF Sub-interface".into()),
            _ if cisco && descr_raw == "Null0" => {
                prob.push("it is a cisco Null0 interface".into());
                default_iftype = false;
            }
            _ => {}
        }
        let default = default_ifstate && default_iftype;
        let mut speed = i.speed.unwrap_or(0);
        if speed == 0 {
            if let Some(z) = r.opts.zero_speed {
                speed = z;
            }
        }
        if speed == 0 {
            speed = match (dev.vendor.as_str(), i.if_type) {
                ("foundry", 194) => 155_000_000,
                ("foundry", 135) => 1_000_000_000,
                ("cisco", _) if dev.descr.contains("FWSM-Firewall ") => 600_000_000,
                ("3com", _) if descr_raw.contains("RMON VLAN ") => 100_000_000,
                _ => {
                    prob.push("has no ifSpeed property".into());
                    if_ok = false;
                    0
                }
            };
        }
        if let Some(p) = &i.counter_problem {
            prob.push(format!("got '{}' from interface when trying to query", one_line(p)));
            if_ok = false;
        }

        // Interface reference.
        let mut if_ref: Option<String> = None;
        if let Some(spec) = &r.opts.ifref {
            for kind in spec.split(',') {
                let cand = match kind {
                    "ip" => i.ip.as_deref().and_then(|v| safe_ref("/", v)),
                    "eth" => i.eth.as_deref().and_then(|v| safe_ref("!", v)),
                    "descr" | "desc" => i.descr.as_deref().and_then(|v| safe_ref("\\", v)),
                    "name" => i.name.as_deref().and_then(|v| safe_ref("#", v)),
                    "type" => i.type_key.as_deref().and_then(|v| safe_ref("%", v)),
                    "nr" => Some(idx.to_string()),
                    other => return Err(format!("Invalid value for --ifref: {spec} ({other})")),
                };
                if cand.is_some() {
                    if_ref = cand;
                    break;
                }
            }
            if if_ref.is_none() {
                prob.push(format!("--ifref={spec} is not unique for this interface"));
                if_ok = false;
            }
        } else if let Some(n) = i.name.as_deref() {
            if_ref = safe_ref("#", n);
        }
        let if_ref = if_ref.unwrap_or_else(|| idx.to_string());

        // MRTG derives the name and title from the reference before escaping.
        let raw_ref = unescape_ref(&if_ref);
        let target_name = format!("{}_{}", router_name, trim_if_ref(&raw_ref));
        let mut if_title_desc = match raw_ref.chars().next() {
            Some(c) if !c.is_ascii_digit() => raw_ref[c.len_utf8()..].to_string(),
            _ => raw_ref.clone(),
        };
        let if_speed = speed / 8;
        let if_speed_str = fmi(if_speed, r.opts.bits);
        let if_type_desc = if_type_name(i.if_type);
        let if_snmp_descr = one_line(i.descr.as_deref().unwrap_or("No Description")).replace(' ', "-");
        let if_snmp_name = one_line(i.name.as_deref().unwrap_or("No Name"));
        let if_snmp_alias = one_line(i.alias.as_deref().unwrap_or(""));
        let if_ip = i.ip.clone().unwrap_or_else(|| "No Ip".into());
        let if_eth = i.eth.clone().unwrap_or_else(|| "No Ethernet Id".into());
        let if_dns_name = one_line(i.dns_name.as_deref().unwrap_or("No DNS name"));
        let mut desc_prefix = "Traffic Analysis for ".to_string();
        if let Some(spec) = &r.opts.ifdesc {
            desc_prefix.clear();
            for kind in spec.split(',') {
                let v = match kind {
                    "ip" => i.ip.clone(),
                    "eth" => i.eth.clone(),
                    "descr" | "desc" => i.descr.as_ref().map(|_| if_snmp_descr.clone()),
                    "alias" => i.alias.as_ref().map(|_| format!("{if_snmp_descr} {if_snmp_alias} ")),
                    "name" => i.name.as_ref().map(|n| format!("#{n}")),
                    "type" => i.type_key.as_ref().map(|t| format!("%{t}")),
                    "nr" => Some(format!("Interface {idx}")),
                    "" => Some(if if_type_desc.is_empty() { if_snmp_descr.clone() } else { if_type_desc.clone() }),
                    "catname" | "ppname" => None,
                    other => return Err(format!("Invalid value for --ifdesc: '{spec} ({other})'")),
                };
                if let Some(v) = v {
                    if_title_desc = v;
                    break;
                }
            }
        }
        let if_title_desc = {
            let t = re!(r"\\([:@\\/# ])").replace_all(&if_title_desc, "$1").into_owned();
            if t.is_empty() { if_snmp_name.clone() } else { one_line(&t) }
        };
        let if_description = format!("{if_snmp_descr} {if_snmp_alias} ");
        let html_title = format!("{}{} -- {}", html_escape(&desc_prefix), html_escape(&if_title_desc), html_escape(&sysname));

        let mut target = format!("Target[{target_name}]: {if_ref}:{router_connect}\n");
        if v3 {
            target.push_str(&format!("SnmpOptions[{target_name}]: {v3options}\n"));
        }
        if i.nohc && SnmpConn::parse(snmpopt).map(|c| c.version != SnmpVersion::V1).unwrap_or(false) {
            target.push_str(&format!("noHC[{target_name}]: yes\n"));
        }
        target.push_str(&format!(
            "SetEnv[{target_name}]: MRTG_INT_IP=\"{}\" MRTG_INT_DESCR=\"{}\"\n",
            if_ip.replace('"', "'"),
            if_snmp_descr.replace('"', "'")
        ));
        if let Some(d) = &directory {
            target.push_str(&format!("Directory[{target_name}]: {d}\n"));
        }
        target.push_str(&format!("MaxBytes[{target_name}]: {if_speed}\n"));
        if enable_ipv6 && ipv4only {
            target.push_str(&format!("IPv4Only[{target_name}]: yes\n"));
        }
        target.push_str(&format!("Title[{target_name}]: {html_title}\n"));
        let mut pt = format!(
            "PageTop[{target_name}]: <h1>{html_title}</h1>\n\t\t<div id=\"sysdetails\">\n\t\t\t<table>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>System:</td>\n\t\t\t\t\t<td>{} in {}</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Maintainer:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Description:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>ifType:</td>\n\t\t\t\t\t<td>{} ({})</td>\n\t\t\t\t</tr>\n\t\t\t\t<tr>\n\t\t\t\t\t<td>ifName:</td>\n\t\t\t\t\t<td>{}</td>\n\t\t\t\t</tr>",
            html_escape(&sysname),
            html_escape(&dev.location),
            html_escape(&dev.contact),
            html_escape(&if_description),
            html_escape(&if_type_desc),
            i.if_type,
            html_escape(&if_snmp_name),
        );
        pt.push_str(&format!("\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Max Speed:</td>\n\t\t\t\t\t<td>{if_speed_str}</td>\n\t\t\t\t</tr>"));
        if i.ip.is_some() {
            pt.push_str(&format!("\n\t\t\t\t<tr>\n\t\t\t\t\t<td>Ip:</td>\n\t\t\t\t\t<td>{if_ip} ({})</td>\n\t\t\t\t</tr>", html_escape(&if_dns_name)));
        }
        pt.push_str("\n\t\t\t</table>\n\t\t</div>\n");
        target.push_str(&pt);

        if let Some(f) = &filter {
            let b = |x: bool| FVal::Num(if x { 1.0 } else { 0.0 });
            let s = |x: &str| FVal::Str(x.to_string());
            let t = i.if_type;
            let vars: HashMap<&str, FVal> = HashMap::from([
                ("default", b(default)),
                ("default_ifstate", b(default_ifstate)),
                ("default_iftype", b(default_iftype)),
                ("if_admin", b(i.admin == 1)),
                ("if_oper", b(i.oper == 1)),
                ("if_type", FVal::Num(t as f64)),
                ("if_type_num", FVal::Num(t as f64)),
                ("if_is_ethernet", b([6, 7, 26, 62, 69, 117].contains(&t))),
                ("if_is_isdn", b([20, 21, 63, 75, 76, 77].contains(&t))),
                ("if_is_dialup", b([20, 21, 63, 75, 76, 77, 23, 81, 82, 108].contains(&t))),
                ("if_is_atm", b([37, 49, 107, 105, 106, 114, 134].contains(&t))),
                ("if_is_wan", b([22, 30, 32, 39, 44, 46].contains(&t))),
                ("if_is_lan", b([6, 7, 26, 62, 69, 117, 8, 9, 11, 15, 55, 59, 60, 115].contains(&t))),
                ("if_is_dsl", b([94, 95, 96, 97].contains(&t))),
                ("if_is_loopback", b(t == 24)),
                ("if_is_ciscovlan", b(cisco && re!(r"(?i)^(unrouted )?[- ]?VLAN[- ]?\d*$").is_match(&descr_raw))),
                ("if_index", FVal::Num(idx as f64)),
                ("if_MTU", i.mtu.map(|m| FVal::Num(m as f64)).unwrap_or_else(|| s("No MTU"))),
                ("if_eth", s(&if_eth)),
                ("if_ip", s(&if_ip)),
                ("if_snmp_descr", s(&if_snmp_descr)),
                ("if_snmp_name", s(&if_snmp_name)),
                ("if_snmp_alias", s(&if_snmp_alias)),
                ("if_dns_name", s(&if_dns_name)),
                ("if_speed", FVal::Num(if_speed as f64)),
                ("if_speed_str", s(&if_speed_str)),
                ("if_type_desc", s(&if_type_desc)),
                ("if_ref", s(&if_ref)),
                ("if_title_desc", s(&if_title_desc)),
                ("if_description", s(&if_description)),
                ("target_name", s(&target_name)),
                ("router_name", s(&router_name)),
                ("sysname", s(&sysname)),
                ("syscontact", s(&one_line(&dev.contact))),
                ("syslocation", s(&one_line(&dev.location))),
                ("sysdescr", s(&comment_descr)),
                ("directory_name", s(directory.as_deref().unwrap_or(""))),
                ("vendor", s(&dev.vendor)),
            ]);
            if f.eval(&vars)? {
                prob.clear();
            } else {
                prob.push("filter specified by --if-filter rejected the interface".into());
                if_ok = false;
            }
        }

        conf.push_str(&format!(
            "### Interface {idx} >> Descr: '{if_snmp_descr}' | Name: '{if_snmp_name}' | Ip: '{if_ip}' | Eth: '{if_eth}' ###\n"
        ));
        if !prob.is_empty() {
            conf.push_str("### The following interface is commented out because:\n");
            for p in &prob {
                conf.push_str(&format!("### * {p}\n"));
            }
            if_ok = false;
        }
        let mut lines = format!("\n{target}");
        if !if_ok {
            lines = lines.lines().map(|l| format!("# {l}")).collect::<Vec<_>>().join("\n") + "\n";
        }
        conf.push_str(&lines);
        conf.push_str("\n\n");
    }
    Ok(())
}

async fn run(p: Parsed) -> Result<String, String> {
    let oids = OidResolver::default();
    let mut conf = format!("# Created by \n# {}\n\n", one_line(&p.fullcmd));
    conf.push_str(
        "\n### Global Config Options\n\n#  for UNIX\n# WorkDir: /home/http/mrtg\n\n#  for NT\n# WorkDir: c:\\mrtgdata\n\n#  for several Linux, like Debian, RHEL and it's derivatives\n# WorkDir: /var/www/html/mrtg\n\n#  for others Linux\n# WorkDir: /srv/http/mrtg\n\n### Global Defaults\n\n#  to get bits instead of bytes and graphs growing to the right\n# Options[_]: growright, bits\n\n",
    );
    conf.push_str(if p.enable_ipv6 { "EnableIPv6: yes\n" } else { "EnableIPv6: no\n" });
    for r in &p.routers {
        for g in &r.globals {
            conf.push_str(&format!("{}\n", one_line(g)));
        }
        let snmpopt = r.snmpopt.clone();
        match discover(r, &snmpopt, p.enable_ipv6, &oids).await {
            Ok((dev, ifs)) => gen_router(&mut conf, r, &snmpopt, &dev, &ifs, p.enable_ipv6, !p.enable_ipv6)?,
            Err(e) => {
                eprintln!("WARNING: Skipping {}{} because: {e}", r.name, snmpopt);
                conf.push_str(&format!("\n### {}: skipped, no SNMP answer ({})\n", one_line(&r.name), one_line(&e)));
            }
        }
    }
    for g in &p.trailing_globals {
        conf.push_str(&format!("{}\n", one_line(g)));
    }
    Ok(conf)
}

/// Entry point; `args[0]` is the program name.
pub fn main(args: Vec<String>) -> ExitCode {
    let parsed = match parse_args(&args) {
        Ok(Some(p)) => p,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("ERROR: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if parsed.routers.is_empty() {
        eprintln!("ERROR: no routers given\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let output = parsed.output.clone();
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: {e}");
            return ExitCode::from(1);
        }
    };
    match rt.block_on(run(parsed)) {
        Ok(conf) => {
            match output {
                Some(path) => {
                    if let Err(e) = crate::fsutil::write_atomic(std::path::Path::new(&path), conf.as_bytes(), 0o640) {
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

    #[test]
    fn device_strings_cannot_inject() {
        assert_eq!(one_line("evil\nTarget[x]: `id`:p@h"), "evil Target[x]: `id`:p@h");
        assert_eq!(safe_ref("#", "Gi0/1 x:y@z&w").unwrap(), r"#Gi0/1\ x\:y\@z\&w");
        assert!(safe_ref("#", "`id`").is_none());
        assert!(safe_ref("#", "a\nb").is_none());
        assert_eq!(html_escape("<script>\"x\"&"), "&lt;script&gt;&quot;x&quot;&amp;");
        assert_eq!(trim_if_ref("#Gi0/1"), "Gi0_1");
        assert_eq!(trim_if_ref("#[a]`b`"), "_a__b_");
    }

    #[test]
    fn fmi_matches_cfgmaker() {
        assert_eq!(fmi(125_000_000, false), "125.0 MBytes/s");
        assert_eq!(fmi(1_250_000, true), "10.0 Mbits/s");
        assert_eq!(fmi(1500, false), "1500.0 Bytes/s");
    }

    #[test]
    fn args_and_secret_masking() {
        let a: Vec<String> = ["cfgmaker", "--global", "WorkDir: /x", "--ifref=name", "--username=u", "--authpassword", "s3cret", "pub@r1:161::::2", "r2"].iter().map(|s| s.to_string()).collect();
        let p = parse_args(&a).unwrap().unwrap();
        assert_eq!(p.routers.len(), 2);
        assert_eq!(p.routers[0].community, "pub");
        assert_eq!(p.routers[0].snmpopt, ":161::::2");
        assert_eq!(p.routers[0].globals, vec!["WorkDir: /x".to_string()]);
        assert!(p.routers[1].globals.is_empty());
        assert!(!p.fullcmd.contains("s3cret"));
        // Keys, communities (as options and in front of a router) and inline values too; an option's
        // value that holds an `@` is not a router.
        let a: Vec<String> = [
            "cfgmaker", "--authkey", "0xKEY1", "--privkey=0xKEY2", "--community", "comm1", "--community=comm2", "--global",
            "ThreshMailAddress[_]: noc@example.com", "comm3@r1:::::2", "secret\\@x@r2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let m = mask_secrets(&a[1..]);
        for secret in ["0xKEY1", "0xKEY2", "comm1", "comm2", "comm3", "secret"] {
            assert!(!m.contains(secret), "{secret} in {m}");
        }
        assert!(m.contains("noc@example.com") && m.contains("XXXXX@r1:::::2") && m.contains("XXXXX@r2"), "{m}");
        assert!(interface_count_ok(MAX_INTERFACES, "r1").is_ok());
        assert!(interface_count_ok(MAX_INTERFACES + 1, "r1").unwrap_err().contains("more than cfgmaker writes"));
        assert!(parse_args(&["cfgmaker".into(), "--if-template=x".into(), "r".into()]).is_err());
    }
}

#[cfg(test)]
mod injection_tests {
    use super::*;

    /// A device returning hostile strings must still produce a config whose
    /// only data sources are the expected SNMP interfaces.
    #[test]
    fn hostile_device_strings_produce_a_clean_config() {
        let dev = DevInfo {
            descr: "Router\nTarget[x]: `cmd`".into(),
            object_id: "1.3.6.1.4.1.9.1.1".into(),
            contact: "a\r\nWorkDir: /etc".into(),
            name: "<script>alert(1)</script>\nTitle[y]: z".into(),
            location: "rack".into(),
            vendor: "cisco".into(),
        };
        let mut ifs = BTreeMap::new();
        for (idx, name, descr, alias) in [
            (1u64, "`cmd`", "eth0\nTarget[evil]: `cmd`:a@b", "x"),
            (2, "Gi0/2 a:b@c", "GigabitEthernet0/2", "uplink\n PageTop[z]: <b>"),
            (3, "Gi0/3", "GigabitEthernet0/3", "ok"),
        ] {
            ifs.insert(idx, IfInfo {
                if_type: 6,
                admin: 1,
                oper: 1,
                speed: Some(1_000_000_000),
                name: Some(name.into()),
                descr: Some(descr.into()),
                alias: Some(alias.into()),
                ..Default::default()
            });
        }
        let spec = RouterSpec {
            community: "pub".into(),
            name: "r1".into(),
            snmpopt: "::::2".into(),
            opts: Opts { ifdesc: Some("alias".into()), ..Default::default() },
            globals: vec![],
        };
        let mut conf = String::from("WorkDir: @WORK@\n");
        gen_router(&mut conf, &spec, "::::2", &dev, &ifs, false, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gen.cfg");
        std::fs::write(&path, conf.replace("@WORK@", &dir.path().to_string_lossy())).unwrap();
        let cfg = linkscope_config::load(&path, &linkscope_config::ReadOptions { trust: linkscope_config::TrustPolicy::Off }, &linkscope_config::CheckOptions::default())
            .unwrap_or_else(|e| panic!("generated config must load: {e}\n{conf}"));
        assert_eq!(cfg.targets.len(), 3, "{conf}");
        assert!(cfg.sources.iter().all(|s| matches!(s, linkscope_config::Source::Snmp(_))), "no external commands allowed: {conf}");
        // The backtick name is not used as a reference; the index is.
        assert!(cfg.targets.iter().any(|t| t.target_text.starts_with("1:pub@r1")), "{conf}");
        // Comments may echo the raw value; nothing that is parsed or rendered may.
        assert!(conf.lines().filter(|l| !l.starts_with('#')).all(|l| !l.contains("<script>")), "{conf}");
        for t in &cfg.targets {
            assert!(!t.title.contains('\n'));
        }
        assert!(!cfg.raw.global.contains_key("workdir") || cfg.raw.global["workdir"] != "/etc");
    }
}
