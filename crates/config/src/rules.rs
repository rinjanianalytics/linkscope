// SPDX-License-Identifier: GPL-2.0-or-later
//! Port of MRTG's `%mrtgrules` keyword table and `quickcheck`.
//!
//! Every keyword MRTG accepts is listed here with the same validation
//! predicate and error text. Keys of the form `tool*key` belong to third
//! party tools and are accepted without checks, as in MRTG.
//!
//! Two predicates are changed for safety: `SnmpOptions` is parsed by a
//! data-only parser ([`crate::perlhash`]) instead of Perl `eval`, and
//! `ConversionCode` is accepted but reported as unsupported by `check`.

use std::net::ToSocketAddrs;
use std::path::Path;

use regex::Regex;

use crate::ConfigError;
use crate::perlhash;

/// Perl's numification of a string: the longest leading numeric prefix,
/// or 0. `"12abc"` is 12, `"1e3"` is 1000, `"abc"` is 0.
pub fn perl_num(s: &str) -> f64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start || (i == digits_start + 1 && b[digits_start] == b'.') {
        return 0.0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    s[..i].parse().unwrap_or(0.0)
}

/// Perl's `int($s)`: numify then truncate toward zero.
pub fn perl_int(s: &str) -> i64 {
    let n = perl_num(s).trunc();
    if n.is_finite() { n as i64 } else { 0 }
}

/// Compile-once cache for the static patterns used by the rules (large
/// configs run tens of thousands of checks).
fn re(p: &'static str) -> Regex {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    static CACHE: LazyLock<Mutex<HashMap<&'static str, Regex>>> = LazyLock::new(Default::default);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.entry(p).or_insert_with(|| Regex::new(p).expect("static regex")).clone()
}

fn yes_no(v: &str) -> bool {
    matches!(v.to_lowercase().as_str(), "yes" | "no")
}

fn is_dir(v: &str) -> bool {
    !v.is_empty() && Path::new(v).is_dir()
}

fn pos_int(v: &str) -> bool {
    re(r"^[0-9]+$").is_match(v) && perl_num(v) > 0.0
}

fn float(v: &str) -> bool {
    re(r"^[0-9]+(?:\.[0-9]+)?$").is_match(v)
}

fn email(v: &str) -> bool {
    !v.is_empty() && re(r"[^ \t\r\n\x0c]+@[^ \t\r\n\x0c]+").is_match(v)
}

fn ndwmy(v: &str) -> bool {
    re(r"(?i)[ndwmy]+").is_match(v)
}

fn interval_ok(v: &str) -> bool {
    let r = re(r"([0-9]+)(?::([0-9]+))?");
    let Some(c) = r.captures(v) else { return false };
    let mut secs = perl_num(&c[1]) * 60.0;
    if let Some(s) = c.get(2) {
        secs += perl_num(s.as_str());
    }
    (1.0..=3600.0).contains(&secs)
}

fn resolvable(host: &str) -> bool {
    !host.is_empty() && (host, 25).to_socket_addrs().map(|mut a| a.next().is_some()).unwrap_or(false)
}

/// `ThreshMailServer`: a host name, or LinkScope's `host:port`.
fn mail_server(v: &str) -> bool {
    match v.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok_and(|p| p > 0) => resolvable(h),
        _ => resolvable(v),
    }
}

fn http_url(v: &str) -> bool {
    re(r"(?i)^https?://[^\s/?#]+(?:[/?#]\S*)?$").is_match(v)
}

/// Validate one keyword value. `per_target` selects the `key[]` rule.
/// Returns `Ok(None)` when valid.
pub fn validate(key: &str, per_target: bool, v: &str) -> Result<(), String> {
    let ok: bool;
    let msg: String;
    macro_rules! rule {
        ($pred:expr, $m:expr) => {{
            ok = $pred;
            msg = $m;
        }};
    }
    match (key, per_target) {
        // ---- global keywords
        ("workdir", false) => rule!(is_dir(v), format!("Working directory {v} does not exist")),
        ("htmldir", false) => rule!(is_dir(v), format!("Html directory {v} does not exist")),
        ("imagedir", false) => rule!(is_dir(v), format!("Image directory {v} does not exist")),
        ("logdir", false) => rule!(is_dir(v), format!("Log directory {v} does not exist")),
        ("forks", false) => rule!(perl_int(v) > 0, "Less than 1 fork or not running on Unix/Linux".into()),
        ("refresh", false) => rule!(perl_int(v) >= 300, format!("{v} should be 300 seconds or more")),
        ("enablesnmpv3", false) | ("enableipv6", false) => rule!(yes_no(v), format!("{v} must be yes or no")),
        ("interval", false) => rule!(
            interval_ok(v),
            format!("{v} should be at least 1 Second (0:01) and no more than 60 Minutes (60)")
        ),
        ("writeexpires", false) | ("nomib2", false) | ("singlerequest", false) | ("runasdaemon", false)
        | ("nodetach", false) | ("rrdcached", false) | ("language", false) => rule!(true, String::new()),
        ("icondir", false) => rule!(!v.is_empty(), "Directory argument missing".into()),
        ("loadmibs", false) => rule!(!v.is_empty(), "No MIB Files specified".into()),
        ("userrdtool", false) => rule!(
            false,
            "UseRRDtool is not valid any more. Use LogFormat, PathAdd and LibAdd instead".into()
        ),
        ("userrdtool", true) => rule!(
            false,
            "UseRRDtool[] is not valid any more. Check the new xyz*bla[] syntax for passing parameters to tool xyz who reads the mrtg.cfg".into()
        ),
        ("logformat", false) => rule!(re(r"^(rateup|rrdtool)$").is_match(v), format!("Invalid Logformat '{v}'")),
        ("pathadd", false) | ("libadd", false) => rule!(Path::new(v).is_dir(), format!("{v} is not the name of a directory")),
        ("maxage", false) => rule!(pos_int(v), format!("{v} must be a Number bigger than 0")),
        ("nospacechar", false) => rule!(v.chars().count() == 1, format!("{v} must be one character long")),
        ("snmpoptions", false) | ("snmpoptions", true) => rule!(
            perlhash::parse(v).is_ok(),
            "Must have the format \"OptA => Number, OptB => 'String', ... \"".into()
        ),
        ("conversioncode", false) => rule!(
            std::fs::File::open(v).is_ok(),
            format!("Cannot read conversion code file {v}")
        ),
        ("sendtographite", false) => rule!(re(r"^.*,[0-9]+$").is_match(v), format!("Invalid Graphite Destination '{v}'")),
        ("threshdir", false) => rule!(is_dir(v), format!("Threshold directory {v} does not exist")),
        ("threshhyst", false) | ("hwthreshhyst", false) => rule!(float(v), format!("{v} must be a Number xxx.xxx")),
        ("threshmailserver", false) => rule!(mail_server(v), format!("Unknown mailserver hostname {v}")),
        ("threshmailsender", false) => rule!(
            email(v),
            format!("ThreshMailAddress {v} does not look like an email address at all")
        ),
        // LinkScope additions (MRTG itself would reject these keywords).
        ("weblisten", false) => rule!(
            v.parse::<std::net::SocketAddr>().is_ok(),
            format!("{v} must be an address:port such as 127.0.0.1:8080 or [::1]:8080")
        ),
        ("webhosts", false) => rule!(
            web_hosts(v).is_some(),
            format!("{v} must be host names separated by commas or spaces (or * for any)")
        ),
        ("execshell", false) | ("webreadonly", false) => rule!(yes_no(v), format!("{v} must be yes or no")),
        // These may hold credentials: their messages never repeat the value.
        ("alertwebhook", false) => rule!(http_url(v), "must be an http:// or https:// URL".into()),
        ("alertwebhooksecret", false) => rule!(v.chars().count() >= 16, "must be at least 16 characters long".into()),
        ("alertmailuser", false) | ("alertmailpassword", false) => rule!(!v.trim().is_empty(), "must not be empty".into()),

        // ---- per-target keywords
        ("target", true) | ("routeruptime", true) | ("routername", true) | ("title", true)
        | ("directory", true) | ("pagetop", true) | ("bodytag", true) | ("pagefoot", true)
        | ("addhead", true) | ("rrdrowcount", true) | ("rrdrowcount30m", true) | ("rrdrowcount2h", true)
        | ("rrdrowcount1d", true) | ("extension", true) | ("timezone", true) | ("options", true)
        | ("colours", true) | ("background", true) | ("textcolor", true) | ("kmg", true)
        | ("pngtitle", true) | ("ylegend", true) | ("shortlegend", true) | ("legend1", true)
        | ("legend2", true) | ("legend3", true) | ("legend4", true) | ("legend5", true)
        | ("legendi", true) | ("legendo", true) | ("timestrfmt", true) => rule!(true, String::new()),
        ("nohc", true) | ("ipv4only", true) => rule!(yes_no(v), format!("{v} must be yes or no")),
        ("maxbytes", true) => rule!(pos_int(v), format!("{v} must be a Number bigger than 0")),
        ("maxbytes1", true) => rule!(pos_int(v), format!("{v} must be numerical and larger than 0")),
        ("maxbytes2", true) => rule!(pos_int(v), format!("{v} must a number bigger than 0")),
        ("absmax", true) => rule!(re(r"^[0-9]+$").is_match(v), format!("{v} must be a Number")),
        ("clonedirectory", true) => rule!(
            re(r"[^,][ \t\n\r\x0c]*$").is_match(v),
            format!("{v} with comma must have the second parameter")
        ),
        ("rrdhwrras", true) => rule!(
            re(r"^RRA:(HWPREDICT|SEASONAL|DEVPREDICT|DEVSEASONAL|FAILURES):[^ \t\n\r\x0c]+([ \t\n\r\x0c]+RRA:(HWPREDICT|SEASONAL|DEVPREDICT|DEVSEASONAL|FAILURES):[^ \t\n\r\x0c]+)*$").is_match(v),
            format!("This does not look like rrdtool HW RRAs. Check the rrdcreate manual page for inspiration. ({v})")
        ),
        ("unscaled", true) | ("withpeak", true) | ("suppress", true) => rule!(
            ndwmy(v),
            "Must be a string of [n]one, [d]ay, [w]eek, [m]onth, [y]ear".into()
        ),
        ("weekformat", true) => rule!(re(r"[UVW]").is_match(v), "Must be either W, V, or U".into()),
        ("xsize", true) => rule!(
            (30..=600).contains(&perl_int(v)),
            format!("{v} must be between 30 and 600 pixels")
        ),
        ("ysize", true) => rule!(perl_int(v) >= 30, "Must be >= 30 pixels".into()),
        ("ytics", true) => rule!(perl_int(v) >= 1, "Must be >= 1".into()),
        ("yticsfactor", true) | ("factor", true) => rule!(
            re(r"[-+0-9.efg]+").is_match(v),
            "Should be a numerical value".into()
        ),
        ("step", true) => rule!(perl_int(v) >= 0, format!("{v} must be > 0")),
        ("kilo", true) => rule!(re(r"^[0-9]+$").is_match(v), format!("{v} must be a Integer Number")),
        ("setenv", true) => rule!(
            re(r#"^(?:[-A-Za-z0-9_]+="[^"]*"(?:[ \t\n\r\x0c]+|$))+$"#).is_match(v),
            format!("{v} must be XY=\"dddd\" AASD=\"kjlkj\" ... ")
        ),
        ("xzoom", true) | ("yzoom", true) | ("xscale", true) | ("yscale", true) => rule!(
            float(v),
            format!("{v} must be a Number xxx.xxx")
        ),
        ("threshmini", true) | ("threshmino", true) | ("threshmaxi", true) | ("threshmaxo", true)
        | ("threshdesc", true) | ("hwthreshmini", true) | ("hwthreshmino", true) | ("hwthreshmaxi", true)
        | ("hwthreshmaxo", true) | ("hwthreshdesc", true) => rule!(true, String::new()),
        ("threshprogi", true) | ("threshprogo", true) | ("threshprogoki", true) | ("threshprogoko", true)
        | ("hwthreshprogi", true) | ("hwthreshprogo", true) | ("hwthreshprogoki", true)
        | ("hwthreshprogoko", true) => rule!(
            !v.is_empty() && executable(Path::new(v)),
            format!("Threshold program {v} cannot be executed")
        ),
        ("threshmailaddress", true) | ("hwthreshmailaddress", true) => rule!(
            email(v),
            format!("ThreshMailAddress {v} does not look like an email address at all")
        ),
        ("threshfor", true) => rule!(pos_int(v), format!("{v} must be a Number bigger than 0")),
        ("timestrpos", true) => rule!(
            re(r"(?i)^(no|[lr][ul])$").is_match(v),
            "Must be a string of NO, LU, RU, LL, RL".into()
        ),
        _ => return Err(UNKNOWN.into()),
    }
    if ok { Ok(()) } else { Err(msg) }
}

const UNKNOWN: &str = "\u{0}unknown";

/// MRTG's `quickcheck`: validate `key` (global or per-target) and produce
/// MRTG's error text on failure.
pub fn quickcheck(key: &str, per_target: bool, value: &str, file: &Path, line: usize) -> Result<(), ConfigError> {
    // Third-party `tool*key` entries are not checked (MRTG passes no rules).
    if key.contains('*') && !key.starts_with('*') {
        return Ok(());
    }
    match validate(key, per_target, value) {
        Ok(()) => Ok(()),
        Err(m) if m == UNKNOWN => Err(ConfigError::at(
            file,
            line,
            format!("Unknown Option \"{key}\" on this line or above. Check mrtg-reference for help"),
        )),
        Err(m) => {
            let name = if per_target { format!("{key}[]") } else { key.to_string() };
            Err(ConfigError::at(file, line, format!("CFG Error in \"{name}\": {m}")))
        }
    }
}

/// True if `key` is a keyword this implementation understands (for docs/tests).
pub fn is_known(key: &str, per_target: bool) -> bool {
    !matches!(validate(key, per_target, ""), Err(ref m) if m == UNKNOWN)
}

/// `WebHosts:`: the host names the web UI answers to besides IP addresses and `localhost`,
/// separated by commas or spaces, lowercase; `*` answers to any. `None` when one is not a name.
pub fn web_hosts(v: &str) -> Option<Vec<String>> {
    let names: Vec<String> = v.split([',', ' ', '\t']).filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()).collect();
    let name_ok = |n: &str| n == "*" || (n.len() <= 253 && n.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')));
    (!names.is_empty() && names.iter().all(|n| name_ok(n))).then_some(names)
}

/// A regular file with an execute bit: what a threshold program must be to run.
fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
