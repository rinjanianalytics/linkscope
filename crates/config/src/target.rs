// SPDX-License-Identifier: GPL-2.0-or-later
//! Port of MRTG's `targparser` and `newSnmpTarg`.
//!
//! MRTG rewrites every data source in a `Target[]` string into
//! ` $target->[N]{$mode} ` and later `eval`s the whole string as Perl.
//! This module runs the same three passes with the same regular
//! expressions (external commands, complex `OID&OID:` targets, then simple
//! `if:community@host` targets), producing the identical rewritten string
//! and source table. The rewritten string is then compiled by
//! [`crate::expr`] into a small arithmetic AST; nothing is ever evaluated
//! as code.

use std::collections::HashMap;

use regex::bytes::Regex;
use serde::Serialize;

use crate::perlhash::SnmpOptions;

/// How an SNMP source selects its interface (MRTG's `IfSel`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum IfSel {
    /// No interface suffix; the OID is used as written.
    None,
    /// Numeric ifIndex appended to the OID.
    If(String),
    /// Interface owning this IPv4 address (`/1.2.3.4`).
    Ip(String),
    /// Interface with this ifDescr (`\eth0`).
    Descr(String),
    /// Interface with this ifName (`#Gi0/1`).
    Name(String),
    /// Interface with this MAC address, normalised to `aa-bb-..` (`!0a-1b-..`).
    Eth(String),
    /// Interface with this ifType (`%6`).
    Type(String),
}

impl IfSel {
    /// MRTG's method name for the selector (`If`, `Ip`, `Descr`, ...).
    pub fn method(&self) -> &'static str {
        match self {
            IfSel::None => "None",
            IfSel::If(_) => "If",
            IfSel::Ip(_) => "Ip",
            IfSel::Descr(_) => "Descr",
            IfSel::Name(_) => "Name",
            IfSel::Eth(_) => "Eth",
            IfSel::Type(_) => "Type",
        }
    }
    /// The selector key (empty for `None`).
    pub fn key(&self) -> &str {
        match self {
            IfSel::None => "",
            IfSel::If(k) | IfSel::Ip(k) | IfSel::Descr(k) | IfSel::Name(k) | IfSel::Eth(k) | IfSel::Type(k) => k,
        }
    }
}

/// SNMP protocol version requested in the target string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SnmpVersion {
    V1,
    V2c,
    V3,
}

/// Connection parameters parsed from the `:port:timeout:retries:backoff:version`
/// suffix, following `SNMP_util::snmpopen`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnmpConn {
    pub port: Option<u16>,
    pub timeout: Option<f64>,
    pub retries: Option<u32>,
    pub backoff: Option<f64>,
    pub version: SnmpVersion,
    pub v4only: bool,
    pub local_host: Option<String>,
    pub local_port: Option<u16>,
}

impl SnmpConn {
    /// Parse MRTG's SnmpInfo string (everything after the host, starting with `:`).
    pub fn parse(info: &str) -> Result<SnmpConn, String> {
        let opts = info.strip_prefix(':').unwrap_or(info);
        let mut f: Vec<&str> = if info.is_empty() { vec![] } else { opts.splitn(6, ':').collect() };
        f.resize(6, "");
        let (mut port_s, timeout_s, retries_s, backoff_s, version_s, v4_s) = (f[0], f[1], f[2], f[3], f[4], f[5]);
        let (mut local_host, mut local_port) = (None, None);
        if let Some((p, l)) = port_s.split_once('!') {
            port_s = p;
            let (lh, lp) = match l.rsplit_once('!') {
                Some((h, p)) => (h, p),
                None => (l, ""),
            };
            if !lh.is_empty() {
                local_host = Some(lh.to_string());
            }
            if !lp.is_empty() {
                local_port = Some(lp.parse().map_err(|_| format!("bad local port '{lp}'"))?);
            }
        }
        let num = |s: &str, what: &str| -> Result<Option<f64>, String> {
            if s.is_empty() { Ok(None) } else { s.parse::<f64>().map(Some).map_err(|_| format!("bad {what} '{s}'")) }
        };
        let version = match version_s {
            "" | "1" => SnmpVersion::V1,
            v if v.eq_ignore_ascii_case("2") || v.eq_ignore_ascii_case("2c") => SnmpVersion::V2c,
            "3" => SnmpVersion::V3,
            v => return Err(format!("unknown SNMP version '{v}'")),
        };
        Ok(SnmpConn {
            port: if port_s.is_empty() { None } else { Some(port_s.parse().map_err(|_| format!("bad port '{port_s}'"))?) },
            timeout: num(timeout_s, "timeout")?,
            retries: num(retries_s, "retries")?.map(|r| r as u32),
            backoff: num(backoff_s, "backoff")?,
            version,
            v4only: v4_s == "v4only",
            local_host,
            local_port,
        })
    }
}

/// One SNMP data source (an entry of MRTG's `@target` with `Methode => SNMP`).
#[derive(Debug, Clone, Serialize)]
pub struct SnmpSource {
    /// A credential: never serialized.
    #[serde(skip)]
    pub community: String,
    /// Host as written; numeric IPv6 keeps its brackets.
    pub host: String,
    /// Raw SnmpInfo string (`:port:timeout:...`), as MRTG keeps it.
    pub snmp_info: String,
    pub conn: SnmpConn,
    /// Name of an MRTG conversion subroutine (`|name`), if any.
    pub conversion: Option<String>,
    /// OIDs for input and output (names like `ifInOctets` or dotted numbers).
    pub oid: [String; 2],
    pub sel: [IfSel; 2],
    pub ipv4only: bool,
    /// May hold SNMPv3 passwords: never serialized.
    #[serde(skip)]
    pub snmp_options: SnmpOptions,
}

/// One external-program data source (MRTG's backtick target).
#[derive(Debug, Clone, Serialize)]
pub struct ExecSource {
    /// The command text with escaped backticks resolved.
    pub command: String,
}

/// A data source referenced by one or more targets.
#[derive(Debug, Clone, Serialize)]
#[allow(clippy::large_enum_variant)] // few sources per config; boxing buys nothing
pub enum Source {
    Snmp(SnmpSource),
    Exec(ExecSource),
}

/// Shared state of the parser across all targets of a config (MRTG's
/// `@target` array and `$targIndex` hash), so identical sources are polled once.
#[derive(Debug, Default)]
pub struct SourceTable {
    pub sources: Vec<Source>,
    index: HashMap<String, usize>,
}

/// Result of parsing one `Target[]` value.
#[derive(Debug, Clone)]
pub struct ParsedTarget {
    /// MRTG's rewritten expression, e.g. ` $target->[0]{$mode} * 8`.
    pub rewritten: String,
    /// Index of the single source this target uses, or `None` when it
    /// combines several sources (MRTG's `uniqueTarget` of -1).
    pub unique: Option<usize>,
}

/// Placeholder text MRTG substitutes for each source reference.
pub fn placeholder(idx: usize) -> String {
    format!(" $target->[{idx}]{{$mode}} ")
}

struct Patterns {
    external: Regex,
    complex: Regex,
    simple: Regex,
}

fn patterns() -> &'static Patterns {
    use std::sync::LazyLock;
    static P: LazyLock<Patterns> = LazyLock::new(|| {
        // Character classes are spelled out so they match Perl's ASCII
        // semantics exactly (no Unicode \w, \d, \s).
        let w = r"[A-Za-z0-9_]";
        let if_simple = String::from(
            r"([0-9]+)|/([0-9]+(?:\.[0-9]+)+)|\\((?:\\[ \t\n\r\x0b\x0c:&@]|[^ \t\n\r\x0b\x0c:&@])+)|\#((?:\\[ \t\n\r\x0b\x0c:&@]|[^ \t\n\r\x0b\x0c:&@])+)|!([a-fA-F0-9]+(?:-[a-fA-F0-9]+)+)|%([0-9]+)"
        );
        // Note the alternation precedence copied from MRTG: the leading '\.'
        // belongs to the numeric alternative only.
        let if_complex = format!(r"((?:\.[0-9]+)*?\.?[-a-zA-Z0-9]*(?:\.[0-9]+)*?)(\.{if_simple})?");
        let if_comhost = format!(
            r"((?:\\[@ ]|[^ \t\n\r\x0b\x0c@])+)@(?:(\[[a-fA-F0-9:]*\])|([-A-Za-z0-9_]+(?:\.[-A-Za-z0-9_]+)*))((?::[0-9.!]*)*)(?:\|([a-zA-Z_]{w}*))?"
        );
        let build = |p: String| {
            regex::bytes::RegexBuilder::new(&p)
                .unicode(false)
                .size_limit(1 << 26)
                .build()
                .expect("target regex")
        };
        Patterns {
            external: build(r"^(.*?)`((?:\\`|[^`])+)`(.*)$".to_string()),
            complex: build(format!(r"^(.*?)({if_complex}&{if_complex}:{if_comhost})(.*)$")),
            simple: build(format!(r"^(.*?)((-)?(?:{if_simple}):{if_comhost})(.*)$")),
        }
    });
    &P
}

fn cap(c: &regex::bytes::Captures, i: usize) -> Option<String> {
    c.get(i).map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
}

struct IfSpec {
    oid: [Option<String>; 2],
    alt: [Option<String>; 2],
    // per direction: num, ip, desc, name, eth, type
    sel: [[Option<String>; 6]; 2],
    com: String,
    host_v6: Option<String>,
    host_name: Option<String>,
    snmp_info: String,
    conv: Option<String>,
}

impl SourceTable {
    /// Parse one `Target[]` value, adding new sources to the table.
    pub fn parse(&mut self, target: &str, ipv4only: bool, snmp_options: &SnmpOptions, nohc: bool) -> Result<ParsedTarget, String> {
        let pats = patterns();
        let mut string = target.to_string();
        let mut parsed = String::new();
        let mut unique: i64 = -1;
        let mut others = 0;
        let mut note = |idx: usize, unique: &mut i64| {
            if *unique < 0 {
                *unique = idx as i64;
            } else if idx as i64 != *unique {
                others += 1;
            }
        };

        // Pass 1: external programs.
        while let Some(c) = pats.external.captures(string.as_bytes()) {
            let pre = cap(&c, 1).unwrap_or_default();
            let t = cap(&c, 2).unwrap_or_default();
            let post = cap(&c, 3).unwrap_or_default();
            let idx = match self.index.get(&t) {
                Some(&i) => i,
                None => {
                    let i = self.sources.len();
                    self.sources.push(Source::Exec(ExecSource { command: t.replace("\\`", "`") }));
                    self.index.insert(t.clone(), i);
                    i
                }
            };
            parsed.push_str(&pre);
            parsed.push_str(&placeholder(idx));
            string = post;
            note(idx, &mut unique);
        }
        string = std::mem::take(&mut parsed) + &string;

        // Pass 2: complex OID&OID:community@host targets.
        while let Some(c) = pats.complex.captures(string.as_bytes()) {
            let pre = cap(&c, 1).unwrap_or_default();
            let t = cap(&c, 2).unwrap_or_default();
            let post = cap(&c, 24).unwrap_or_default();
            let mut sel = [[None, None, None, None, None, None], [None, None, None, None, None, None]];
            for (d, base) in [(0usize, 3usize), (1, 11)] {
                for (k, slot) in sel[d].iter_mut().enumerate() {
                    *slot = cap(&c, base + 2 + k);
                }
            }
            let spec = IfSpec {
                oid: [cap(&c, 3), cap(&c, 11)],
                alt: [cap(&c, 4), cap(&c, 12)],
                sel,
                com: cap(&c, 19).unwrap_or_default(),
                host_v6: cap(&c, 20),
                host_name: cap(&c, 21),
                snmp_info: cap(&c, 22).unwrap_or_default(),
                conv: cap(&c, 23),
            };
            // MRTG: retry later as a simple target if a Descr/Name ends in a backslash.
            let ends_bs = |d: usize, k: usize| spec.sel[d][k].as_deref().is_some_and(|v| v.ends_with('\\'));
            if ends_bs(0, 2) || ends_bs(0, 3) || ends_bs(1, 2) || ends_bs(1, 3) {
                parsed.push_str(&pre);
                parsed.push_str(&t);
                string = post;
                continue;
            }
            let idx = match self.index.get(&t) {
                Some(&i) => i,
                None => {
                    let src = new_snmp_source(&t, spec, ipv4only, snmp_options)?;
                    let i = self.sources.len();
                    self.sources.push(Source::Snmp(src));
                    self.index.insert(t.clone(), i);
                    i
                }
            };
            parsed.push_str(&pre);
            parsed.push_str(&placeholder(idx));
            string = post;
            note(idx, &mut unique);
        }
        string = std::mem::take(&mut parsed) + &string;

        // Pass 3: simple if:community@host targets.
        while let Some(c) = pats.simple.captures(string.as_bytes()) {
            let pre = cap(&c, 1).unwrap_or_default();
            let t = cap(&c, 2).unwrap_or_default();
            let rev = cap(&c, 3);
            let post = cap(&c, 15).unwrap_or_default();
            let idx = match self.index.get(&t) {
                Some(&i) => i,
                None => {
                    // Groups: 3 reversal, 4..=9 selector, 10 community, 11 IPv6 host,
                    // 12 DNS host, 13 SnmpInfo, 14 conversion.
                    let d1 = if rev.as_deref() == Some("-") { 1 } else { 0 };
                    let one: [Option<String>; 6] = [cap(&c, 4), cap(&c, 5), cap(&c, 6), cap(&c, 7), cap(&c, 8), cap(&c, 9)];
                    let spec = IfSpec {
                        oid: [None, None],
                        alt: [Some("1".into()), Some("1".into())],
                        sel: [one.clone(), one],
                        com: cap(&c, 10).unwrap_or_default(),
                        host_v6: cap(&c, 11),
                        host_name: cap(&c, 12),
                        snmp_info: cap(&c, 13).unwrap_or_default(),
                        conv: cap(&c, 14),
                    };
                    let src = new_snmp_source(&t, fix_hc(spec, nohc, d1), ipv4only, snmp_options)?;
                    let i = self.sources.len();
                    self.sources.push(Source::Snmp(src));
                    self.index.insert(t.clone(), i);
                    i
                }
            };
            parsed.push_str(&pre);
            parsed.push_str(&placeholder(idx));
            string = post;
            note(idx, &mut unique);
        }
        let rewritten = parsed + &string;
        Ok(ParsedTarget {
            rewritten,
            unique: if others == 0 && unique >= 0 { Some(unique as usize) } else { None },
        })
    }
}

/// MRTG upgrades simple targets to 64-bit counters when the 5th SnmpInfo
/// field (version) is 2 or 3: `m/(?::[^:]*){4}:[32][Cc]?/`.
fn hc_requested(info: &str) -> bool {
    use std::sync::LazyLock;
    static R: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?::[^:]*){4}:[32][Cc]?").unwrap());
    R.is_match(info)
}

/// Recompute the OID choice once the real SnmpInfo capture is known.
fn fix_hc(mut spec: IfSpec, nohc: bool, d1: usize) -> IfSpec {
    let d2 = 1 - d1;
    let hc = !nohc && hc_requested(&spec.snmp_info);
    spec.oid[d1] = Some(if hc { "ifHCInOctets" } else { "ifInOctets" }.to_string());
    spec.oid[d2] = Some(if hc { "ifHCOutOctets" } else { "ifOutOctets" }.to_string());
    spec
}

fn new_snmp_source(t: &str, spec: IfSpec, ipv4only: bool, snmp_options: &SnmpOptions) -> Result<SnmpSource, String> {
    let mut sels = [IfSel::None, IfSel::None];
    let mut oids = [String::new(), String::new()];
    for i in 0..2 {
        if spec.oid[i].is_none() && spec.alt[i].is_none() {
            return Err(format!("Malformed {}ifSpec in '{t}'", if i == 1 { "output " } else { "input " }));
        }
        oids[i] = spec.oid[i].clone().unwrap_or_default();
        let s = &spec.sel[i];
        let mut sel = if spec.alt[i].is_some() {
            if let Some(v) = &s[0] {
                IfSel::If(v.clone())
            } else if let Some(v) = &s[1] {
                IfSel::Ip(v.clone())
            } else if let Some(v) = &s[2] {
                IfSel::Descr(v.clone())
            } else if let Some(v) = &s[3] {
                IfSel::Name(v.clone())
            } else if let Some(v) = &s[4] {
                let norm: Vec<String> = v
                    .split('-')
                    .map(|h| format!("{:02x}", u64::from_str_radix(h, 16).unwrap_or(0)))
                    .collect();
                IfSel::Eth(norm.join("-"))
            } else if let Some(v) = &s[5] {
                IfSel::Type(v.clone())
            } else {
                return Err(format!("Internal error parsing ifSpec in '{t}'"));
            }
        } else {
            IfSel::None
        };
        // Unescape and trim Descr/Name keys like MRTG.
        sel = match sel {
            IfSel::Descr(k) => IfSel::Descr(trim_ctl(&unescape(&k, " \t\n\r\x0b\x0c:&@"))),
            IfSel::Name(k) => IfSel::Name(trim_ctl(&unescape(&k, " \t\n\r\x0b\x0c:&@"))),
            IfSel::If(k) => IfSel::If(trim_ctl(&k)),
            IfSel::Ip(k) => IfSel::Ip(trim_ctl(&k)),
            IfSel::Eth(k) => IfSel::Eth(trim_ctl(&k)),
            IfSel::Type(k) => IfSel::Type(trim_ctl(&k)),
            IfSel::None => IfSel::None,
        };
        sels[i] = sel;
    }
    let host = spec.host_v6.or(spec.host_name).unwrap_or_default();
    let conn = SnmpConn::parse(&spec.snmp_info).map_err(|e| format!("{e} in '{t}'"))?;
    Ok(SnmpSource {
        community: unescape(&spec.com, " @"),
        host,
        snmp_info: spec.snmp_info,
        conn,
        conversion: spec.conv,
        oid: oids,
        sel: sels,
        ipv4only,
        snmp_options: snmp_options.clone(),
    })
}

/// Perl `s/\\([set])/$1/g`.
fn unescape(s: &str, set: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(&n) = it.peek() {
                if set.contains(n) {
                    out.push(n);
                    it.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Perl `s/[\0- ]+$//`.
fn trim_ctl(s: &str) -> String {
    s.trim_end_matches(|c: char| c <= ' ').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> (SourceTable, ParsedTarget) {
        let mut st = SourceTable::default();
        let p = st.parse(t, true, &SnmpOptions::new(), false).unwrap();
        (st, p)
    }

    #[test]
    fn simple_target() {
        let (st, p) = parse("2:public@router1");
        assert_eq!(p.rewritten, " $target->[0]{$mode} ");
        assert_eq!(p.unique, Some(0));
        let Source::Snmp(s) = &st.sources[0] else { panic!() };
        assert_eq!(s.oid, ["ifInOctets".to_string(), "ifOutOctets".to_string()]);
        assert_eq!(s.sel[0], IfSel::If("2".into()));
        assert_eq!(s.host, "router1");
    }

    #[test]
    fn hc_and_reverse() {
        let (st, _) = parse("-#Gi0/1:pub@r:::::2");
        let Source::Snmp(s) = &st.sources[0] else { panic!() };
        assert_eq!(s.oid, ["ifHCOutOctets".to_string(), "ifHCInOctets".to_string()]);
        assert_eq!(s.sel[1], IfSel::Name("Gi0/1".into()));
        assert_eq!(s.conn.version, SnmpVersion::V2c);
    }

    #[test]
    fn arithmetic_of_two_and_dedupe() {
        let (st, p) = parse("1:a@h + 1:a@h + 2:a@h");
        assert_eq!(st.sources.len(), 2);
        assert_eq!(p.unique, None);
        assert!(p.rewritten.contains("$target->[1]"));
    }

    #[test]
    fn complex_and_exec() {
        let (st, p) = parse("ifInErrors.3&ifOutErrors.3:pub@[2001:db8::1]:161 + `/bin/echo 1`");
        assert_eq!(st.sources.len(), 2);
        assert!(matches!(st.sources[0], Source::Exec(_)));
        let Source::Snmp(s) = &st.sources[1] else { panic!() };
        assert_eq!(s.host, "[2001:db8::1]");
        assert_eq!(s.oid[0], "ifInErrors");
        assert_eq!(s.conn.port, Some(161));
        assert!(p.rewritten.starts_with(" $target->[1]{$mode} "));
    }

    #[test]
    fn mac_normalised_and_escapes() {
        let (st, _) = parse(r"!0-1b-2c-3-4-5:pub@h");
        let Source::Snmp(s) = &st.sources[0] else { panic!() };
        assert_eq!(s.sel[0], IfSel::Eth("00-1b-2c-03-04-05".into()));
        let (st, _) = parse(r"\Port\ 1\:2:pub@h");
        let Source::Snmp(s) = &st.sources[0] else { panic!() };
        assert_eq!(s.sel[0], IfSel::Descr("Port 1:2".into()));
    }
}
