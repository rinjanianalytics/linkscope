// SPDX-License-Identifier: GPL-2.0-or-later
//! Semantic checks and defaults, a port of MRTG's `cfgcheck`.
//!
//! Produces a typed [`Config`] from a [`RawConfig`]. Defaults and error
//! conditions follow MRTG. Additional checks performed by LinkScope:
//! * target names and `Directory[]` may not escape the work directory;
//! * `Target[]` expressions are compiled (not evaluated) and syntax errors
//!   are reported at load time;
//! * `AbsMax[]` smaller than `MaxBytes[]` is reported at load time (MRTG
//!   only noticed it while polling);
//! * features that need Perl (`|conversion` subroutines, `LibAdd`) or
//!   RRDtool are reported clearly instead of half-working.

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use crate::expr::Expr;
use crate::perlhash::{self, SnmpOptions};
use crate::raw::RawConfig;
use crate::rules::{perl_int, perl_num};
use crate::target::{Source, SourceTable};
use crate::ConfigError;

/// Options for [`check`].
#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    /// Do not create missing directories (MRTG's `--check`).
    pub no_create: bool,
}

/// Global settings.
#[derive(Debug, Clone, Serialize)]
pub struct Global {
    pub log_dir: PathBuf,
    pub html_dir: PathBuf,
    pub image_dir: PathBuf,
    pub work_dir: Option<PathBuf>,
    pub forks: Option<u32>,
    pub refresh: u32,
    /// Poll interval in seconds (`Interval:` is minutes, or `M:SS`).
    pub interval_secs: u64,
    pub enable_snmpv3: bool,
    pub enable_ipv6: bool,
    pub no_mib2: bool,
    pub single_request: bool,
    pub run_as_daemon: bool,
    pub no_detach: bool,
    pub max_age: Option<u64>,
    /// May hold SNMPv3 passwords: never serialized.
    #[serde(skip)]
    pub snmp_options: Option<SnmpOptions>,
    pub path_add: Option<String>,
    pub load_mibs: Vec<String>,
    pub thresh_dir: Option<PathBuf>,
    pub thresh_hyst: f64,
    pub thresh_mail_server: Option<String>,
    pub thresh_mail_sender: Option<String>,
    /// LinkScope: where threshold changes are posted (`AlertWebhook:`). The
    /// URL may carry a token, so it and the signing secret are never serialized.
    #[serde(skip)]
    pub alert_webhook: Option<String>,
    #[serde(skip)]
    pub alert_webhook_secret: Option<String>,
    /// LinkScope: log in to `ThreshMailServer` over STARTTLS with these.
    #[serde(skip)]
    pub alert_mail_user: Option<String>,
    #[serde(skip)]
    pub alert_mail_password: Option<String>,
    pub language: Option<String>,
    pub write_expires: bool,
    pub icon_dir: Option<String>,
    /// LinkScope: address for the built-in web UI.
    pub web_listen: Option<String>,
    /// LinkScope: `WebHosts:`, the host names the web UI answers to besides IP addresses and
    /// `localhost` (`*`: any). Against DNS rebinding: the UI has no login.
    pub web_hosts: Vec<String>,
    /// LinkScope: allow EXEC targets that need `/bin/sh` (pipes, redirects, ...).
    pub exec_shell: bool,
}

/// MRTG `Options[]` flags.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Options {
    pub growright: bool,
    pub bits: bool,
    pub noinfo: bool,
    pub absolute: bool,
    pub gauge: bool,
    pub nopercent: bool,
    pub avgpeak: bool,
    pub derive: bool,
    pub integer: bool,
    pub perhour: bool,
    pub perminute: bool,
    pub transparent: bool,
    pub dorelpercent: bool,
    pub unknaszero: bool,
    pub withzeroes: bool,
    pub noborder: bool,
    pub noarrow: bool,
    pub noi: bool,
    pub noo: bool,
    pub nobanner: bool,
    pub nolegend: bool,
    pub logscale: bool,
    pub secondmean: bool,
    pub pngdate: bool,
    pub printrouter: bool,
    pub expscale: bool,
}

const KNOWN_OPTIONS: &[&str] = &[
    "growright", "bits", "noinfo", "absolute", "gauge", "nopercent", "avgpeak", "derive", "integer",
    "perhour", "perminute", "transparent", "dorelpercent", "unknaszero", "withzeroes", "noborder",
    "noarrow", "noi", "noo", "nobanner", "nolegend", "logscale", "secondmean", "pngdate", "printrouter",
    "expscale",
];

impl Options {
    fn set(&mut self, name: &str) {
        match name {
            "growright" => self.growright = true,
            "bits" => self.bits = true,
            "noinfo" => self.noinfo = true,
            "absolute" => self.absolute = true,
            "gauge" => self.gauge = true,
            "nopercent" => self.nopercent = true,
            "avgpeak" => self.avgpeak = true,
            "derive" => self.derive = true,
            "integer" => self.integer = true,
            "perhour" => self.perhour = true,
            "perminute" => self.perminute = true,
            "transparent" => self.transparent = true,
            "dorelpercent" => self.dorelpercent = true,
            "unknaszero" => self.unknaszero = true,
            "withzeroes" => self.withzeroes = true,
            "noborder" => self.noborder = true,
            "noarrow" => self.noarrow = true,
            "noi" => self.noi = true,
            "noo" => self.noo = true,
            "nobanner" => self.nobanner = true,
            "nolegend" => self.nolegend = true,
            "logscale" => self.logscale = true,
            "secondmean" => self.secondmean = true,
            "pngdate" => self.pngdate = true,
            "printrouter" => self.printrouter = true,
            "expscale" => self.expscale = true,
            _ => {}
        }
    }

    /// rateup update letter, with MRTG's precedence (gauge wins).
    pub fn rate_letter(&self) -> char {
        if self.gauge {
            'g'
        } else if self.absolute {
            'a'
        } else if self.derive {
            'd'
        } else if self.perhour {
            'h'
        } else if self.perminute {
            'm'
        } else {
            'u'
        }
    }
}

/// Threshold settings for one target.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Thresholds {
    pub min_i: Option<String>,
    pub min_o: Option<String>,
    pub max_i: Option<String>,
    pub max_o: Option<String>,
    pub desc: Option<String>,
    pub prog_i: Option<String>,
    pub prog_o: Option<String>,
    pub prog_ok_i: Option<String>,
    pub prog_ok_o: Option<String>,
    pub mail_address: Option<String>,
    /// LinkScope's `ThreshFor[]`: polls a bound must stay broken before it
    /// fires. Set, the target's programs and mail run once when an alert
    /// fires and once when it clears; unset, they run as MRTG runs them.
    pub for_polls: Option<u32>,
}

impl Thresholds {
    pub fn any(&self) -> bool {
        self.min_i.is_some() || self.min_o.is_some() || self.max_i.is_some() || self.max_o.is_some()
    }
}

/// A `ThreshProg*[]` as it is run: the file the check found. MRTG checks the value as a path
/// (relative to the working directory) but runs it through `PATH`, so a bare name could check one
/// file and run another; LinkScope runs exactly the file it checked, made absolute here.
pub fn program_path(v: String) -> String {
    let p = Path::new(&v);
    if p.is_absolute() {
        return v;
    }
    std::env::current_dir().map(|d| d.join(p).to_string_lossy().into_owned()).unwrap_or(v)
}

/// One graphed target (`Target[name]` and its companion keywords).
#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub name: String,
    /// Original `Target[]` text.
    pub target_text: String,
    /// MRTG's rewritten form (kept for diagnostics and parity tests).
    pub rewritten: String,
    pub expr: Expr,
    /// The single source this target reads, if it reads exactly one.
    pub unique_source: Option<usize>,
    pub title: String,
    pub page_top: Option<String>,
    pub page_foot: Option<String>,
    pub add_head: Option<String>,
    pub body_tag: Option<String>,
    /// Subdirectory (with trailing `/`) or empty.
    pub directory: String,
    pub max_bytes1: u64,
    pub max_bytes2: u64,
    pub abs_max: u64,
    pub options: Options,
    pub xsize: u32,
    pub ysize: u32,
    pub ytics: u32,
    pub yticsfactor: f64,
    pub factor: f64,
    pub kilo: u64,
    pub kmg: Option<Vec<String>>,
    /// (name, #rrggbb) for in, out, max in, max out (and percent with dorelpercent).
    pub colours: Vec<(String, String)>,
    pub background: String,
    pub text_color: String,
    pub legend: [Option<String>; 5],
    pub legend_i: Option<String>,
    pub legend_o: Option<String>,
    pub y_legend: Option<String>,
    pub short_legend: Option<String>,
    pub png_title: Option<String>,
    pub unscaled: String,
    pub with_peak: String,
    pub suppress: String,
    pub week_format: Option<char>,
    pub timezone: Option<String>,
    pub set_env: Vec<(String, String)>,
    pub extension: String,
    pub router_uptime: Option<String>,
    pub router_name: Option<String>,
    /// May hold SNMPv3 passwords: never serialized.
    #[serde(skip)]
    pub snmp_options: SnmpOptions,
    pub xzoom: f64,
    pub yzoom: f64,
    pub xscale: f64,
    pub yscale: f64,
    pub timestr_pos: String,
    pub timestr_fmt: String,
    pub thresholds: Thresholds,
    pub clone_directory: Option<String>,
}

/// A fully checked configuration.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
    pub global: Global,
    pub targets: Vec<Target>,
    /// Deduplicated data sources shared by all targets.
    pub sources: Vec<Source>,
    /// Non-fatal findings worth showing to the operator.
    pub warnings: Vec<String>,
    /// The raw keyword maps, for keywords not modelled above.
    #[serde(skip)]
    pub raw: RawConfig,
}

fn ensure_sl(p: &str) -> String {
    if p.ends_with('/') { p.to_string() } else { format!("{p}/") }
}

fn mkdirs(dir: &Path, opts: &CheckOptions, errors: &mut Vec<String>) {
    if opts.no_create || dir.is_dir() {
        return;
    }
    // LinkScope creates directories 0755 (MRTG used 0777 and relied on umask).
    if let Err(e) = fs::DirBuilder::new().recursive(true).mode(0o755).create(dir) {
        errors.push(format!("cannot create directory {}: {e}", dir.display()));
    }
}

fn yes(v: Option<&String>) -> bool {
    v.is_some_and(|v| v.eq_ignore_ascii_case("yes"))
}

/// Reject target names that could escape the log directory.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') || name.contains('\\') {
        return Err(format!("target name '{name}' may not contain '/', '\\' or be '.' / '..'"));
    }
    Ok(())
}

/// Normalize `Directory[]` the way MRTG uses it (`"$logdir$directory"`,
/// plain concatenation): a leading `/` does not make it absolute. The result
/// is a relative path of normal components only, so it cannot leave WorkDir.
fn normalize_directory(dir: &str) -> Result<String, String> {
    use std::path::Component;
    let rel = dir.trim_start_matches('/');
    if rel.contains('\0') {
        return Err(format!("Directory '{dir}' contains a NUL byte"));
    }
    let mut parts = Vec::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(format!("Directory '{dir}' may not contain '..'")),
        }
    }
    Ok(parts.join("/"))
}

/// Create the log directories (WorkDir/LogDir and every `Directory[]`).
/// Call this after dropping privileges so the directories belong to the
/// account that writes the logs.
pub fn create_dirs(cfg: &Config) -> Result<(), ConfigError> {
    let mut errors = Vec::new();
    let opts = CheckOptions::default();
    mkdirs(&cfg.global.log_dir, &opts, &mut errors);
    for t in &cfg.targets {
        if !t.directory.is_empty() {
            mkdirs(&cfg.global.log_dir.join(&t.directory), &opts, &mut errors);
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(ConfigError::Invalid(errors)) }
}

/// Characters that only make sense to a shell.
pub fn needs_shell(cmd: &str) -> bool {
    cmd.contains(['|', '&', ';', '<', '>', '$', '`', '(', ')', '*', '?', '[', ']', '~', '{', '}'])
        || cmd.contains('\n')
}

/// Run MRTG's `cfgcheck` on `raw`.
pub fn check(raw: RawConfig, opts: &CheckOptions) -> Result<Config, ConfigError> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let g = &raw.global;

    // ---- directories
    let (mut log_dir, mut html_dir, mut image_dir, mut work_dir) = (None, None, None, None);
    if let Some(w) = g.get("workdir") {
        let w = ensure_sl(w);
        mkdirs(Path::new(&w), opts, &mut errors);
        work_dir = Some(PathBuf::from(&w));
        log_dir = Some(w.clone());
        html_dir = Some(w.clone());
        image_dir = Some(w);
    } else if !(g.contains_key("logdir") || g.contains_key("htmldir") || g.contains_key("imagedir")) {
        return Err(ConfigError::Invalid(vec!["\"WorkDir\" not specified in mrtg config file".into()]));
    } else {
        for (k, slot, label) in [
            ("logdir", &mut log_dir, "LogDir"),
            ("htmldir", &mut html_dir, "HtmlDir"),
            ("imagedir", &mut image_dir, "ImageDir"),
        ] {
            match g.get(k) {
                Some(d) => {
                    let d = ensure_sl(d);
                    mkdirs(Path::new(&d), opts, &mut errors);
                    *slot = Some(d);
                }
                None => errors.push(format!("\"{label}\" not specified")),
            }
        }
    }
    let log_dir = log_dir.unwrap_or_default();

    if g.contains_key("threshmailserver") && !g.contains_key("threshmailsender") {
        errors.push("If \"ThreshMailServer\" is defined, then \"ThreshMailSender\" must be defined too.".into());
    }
    if g.contains_key("threshmailsender") && !g.contains_key("threshmailserver") {
        errors.push("If \"ThreshMailSender\" is defined, then \"ThreshMailServer\" must be defined too.".into());
    }
    // LinkScope's alert keywords come in pairs (a webhook is always signed).
    for (a, b) in [("AlertWebhook", "AlertWebhookSecret"), ("AlertMailUser", "AlertMailPassword")] {
        for (x, y) in [(a, b), (b, a)] {
            if g.contains_key(&x.to_ascii_lowercase()) && !g.contains_key(&y.to_ascii_lowercase()) {
                errors.push(format!("If \"{x}\" is defined, then \"{y}\" must be defined too."));
            }
        }
    }
    if g.contains_key("alertmailuser") && !g.contains_key("threshmailserver") {
        errors.push("\"AlertMailUser\" needs \"ThreshMailServer:\" to log in to.".into());
    }
    let thresh_hyst = match g.get("threshhyst") {
        Some(h) if perl_num(h) != 0.0 => perl_num(h),
        _ => 0.1,
    };

    if g.contains_key("libadd") {
        warnings.push("LibAdd is a Perl library path and is ignored by LinkScope".into());
    }
    if g.contains_key("sendtographite") {
        warnings.push("SendToGraphite is not supported by LinkScope and is ignored".into());
    }
    if g.contains_key("rrdcached") {
        warnings.push("RRDCached is not supported by LinkScope and is ignored".into());
    }
    if g.get("logformat").is_some_and(|f| f == "rrdtool") {
        errors.push("LogFormat: rrdtool is not supported by LinkScope; it reads and writes MRTG's native .log files".into());
    }
    let snmp_options = match g.get("snmpoptions") {
        Some(s) => Some(perlhash::parse(s).map_err(|e| ConfigError::Invalid(vec![format!("SnmpOptions: {e}")]))?),
        None => None,
    };

    // ---- interval
    let interval_secs = {
        let r = Regex::new(r"([0-9]+)(?::([0-9]+))?").unwrap();
        match g.get("interval").and_then(|v| r.captures(v).map(|c| (c[1].to_string(), c.get(2).map(|m| m.as_str().to_string())))) {
            Some((m, s)) => {
                let mut minutes = perl_num(&m);
                if let Some(s) = s {
                    let s = perl_num(&s);
                    if s != 0.0 {
                        minutes += s / 60.0;
                    }
                }
                if minutes < 5.0 {
                    errors.push("CFG Error in \"Interval\": should be at least 5 Minutes (unless you use rrdtool)".into());
                }
                (minutes * 60.0).round() as u64
            }
            None => 300,
        }
    };

    let mut conversion_note = false;
    if g.contains_key("conversioncode") {
        conversion_note = true;
    }

    // Global continuation lines that MRTG silently ignores.
    for (k, per) in &raw.per_target {
        if per.contains_key("") {
            warnings.push(format!(
                "continuation line after global keyword '{k}' is ignored (MRTG ignores it too); put the value on one line"
            ));
        }
    }

    let enable_ipv6 = yes(g.get("enableipv6"));
    let exec_shell = yes(g.get("execshell"));

    let global = Global {
        log_dir: PathBuf::from(&log_dir),
        html_dir: PathBuf::from(html_dir.unwrap_or_default()),
        image_dir: PathBuf::from(image_dir.unwrap_or_default()),
        work_dir,
        forks: g.get("forks").map(|f| perl_int(f).max(1) as u32),
        refresh: g.get("refresh").map(|r| perl_int(r) as u32).unwrap_or(300),
        interval_secs,
        enable_snmpv3: yes(g.get("enablesnmpv3")),
        enable_ipv6,
        no_mib2: g.contains_key("nomib2") && !g.get("nomib2").is_some_and(|v| v.eq_ignore_ascii_case("no")),
        single_request: g.contains_key("singlerequest") && !g.get("singlerequest").is_some_and(|v| v.eq_ignore_ascii_case("no")),
        run_as_daemon: yes(g.get("runasdaemon")),
        no_detach: yes(g.get("nodetach")),
        max_age: g.get("maxage").map(|m| perl_int(m) as u64),
        snmp_options: snmp_options.clone(),
        path_add: g.get("pathadd").map(|p| ensure_sl(p)),
        load_mibs: g
            .get("loadmibs")
            .map(|m| m.split([',', ' ', '\t', '\n']).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default(),
        thresh_dir: g.get("threshdir").map(PathBuf::from),
        thresh_hyst,
        thresh_mail_server: g.get("threshmailserver").cloned(),
        thresh_mail_sender: g.get("threshmailsender").cloned(),
        alert_webhook: g.get("alertwebhook").cloned(),
        alert_webhook_secret: g.get("alertwebhooksecret").cloned(),
        alert_mail_user: g.get("alertmailuser").cloned(),
        alert_mail_password: g.get("alertmailpassword").cloned(),
        language: g.get("language").cloned(),
        write_expires: yes(g.get("writeexpires")),
        icon_dir: g.get("icondir").cloned(),
        web_listen: g.get("weblisten").cloned(),
        web_hosts: g.get("webhosts").and_then(|v| crate::rules::web_hosts(v)).unwrap_or_default(),
        exec_shell,
    };

    // ---- per target
    let mut table = SourceTable::default();
    let mut targets = Vec::with_capacity(raw.targets.len());
    let colour4 = Regex::new(r"(?ix)^([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})").unwrap();
    let colour5 = Regex::new(r"(?ix)^([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})\s*,\s*([^\#]+)(\#[0-9a-f]{6})").unwrap();
    let hexcol = Regex::new(r"(?i)^(\#[0-9a-f]{6})").unwrap();
    let setenv_re = Regex::new(r#"([^=]+)="([^"]*)"\s*"#).unwrap();
    let mut thresh_mail_error = false;

    for name in &raw.targets {
        let get = |k: &str| raw.get(k, name).map(str::to_string);
        let mut err = |m: String| errors.push(format!("[{name}] {m}"));

        if let Err(e) = check_name(name) {
            err(e);
            continue;
        }
        if get("threshmailaddress").is_some() && global.thresh_mail_server.is_none() && !thresh_mail_error {
            err("ThreshMailAddress specified without \"ThreshMailServer:\"".into());
            thresh_mail_error = true;
        }
        let t_snmp_options = match get("snmpoptions") {
            Some(s) => match perlhash::parse(&s) {
                Ok(o) => o,
                Err(e) => {
                    err(format!("SnmpOptions: {e}"));
                    continue;
                }
            },
            None => snmp_options.clone().unwrap_or_default(),
        };
        let title = match get("title") {
            Some(t) => t,
            None => {
                err(format!("\"Title[{name}]\" not specified"));
                String::new()
            }
        };
        let directory = match get("directory").filter(|d| !d.is_empty()) {
            Some(d) => match normalize_directory(&d) {
                Err(e) => {
                    err(e);
                    String::new()
                }
                Ok(n) if n.is_empty() => String::new(),
                Ok(n) => {
                    let d = ensure_sl(&n);
                    mkdirs(&Path::new(&log_dir).join(&d), opts, &mut errors);
                    d
                }
            },
            None => String::new(),
        };
        let mut err = |m: String| errors.push(format!("[{name}] {m}"));
        let page_top = get("pagetop").map(|s| s.replace("\\n", "\n"));
        let page_foot = get("pagefoot").map(|s| s.replace("\\n", "\n"));

        let mb = get("maxbytes");
        let mb1 = get("maxbytes1").or_else(|| mb.clone());
        let mb2 = get("maxbytes2").or_else(|| mb.clone());
        if mb.is_none() && mb1.is_none() && mb2.is_none() {
            err(format!("\"MaxBytes[{name}]\" not specified"));
        } else {
            if mb1.is_none() {
                err(format!("\"MaxBytes1[{name}]\" not specified"));
            }
            if mb2.is_none() {
                err(format!("\"MaxBytes2[{name}]\" not specified"));
            }
        }
        let max_bytes1 = mb1.as_deref().map(|v| perl_num(v) as u64).unwrap_or(0);
        let max_bytes2 = mb2.as_deref().map(|v| perl_num(v) as u64).unwrap_or(0);
        let maxv = max_bytes1.max(max_bytes2);
        let abs_max = get("absmax").map(|v| perl_num(&v) as u64).unwrap_or(maxv);
        if abs_max < maxv {
            err(format!("AbsMax: {abs_max} is smaller than MaxBytes: {maxv}"));
        }

        let mut options = Options::default();
        if let Some(o) = get("options") {
            for one in o.to_lowercase().split([',', ' ', '\t', '\n']).filter(|s| !s.is_empty()) {
                if KNOWN_OPTIONS.contains(&one) {
                    options.set(one);
                } else {
                    err(format!("Option[{name}]: \"{one}\" is unknown"));
                }
            }
            if options.derive {
                err("Option \"derive\" works only with rrdtool logformat".into());
            }
        }

        // ---- the Target expression
        let ipv4only = !enable_ipv6 || get("ipv4only").is_some_and(|v| v.eq_ignore_ascii_case("yes"));
        let nohc = get("nohc").is_some_and(|v| v.eq_ignore_ascii_case("yes"));
        let target_text = get("target");
        let (rewritten, expr, unique_source) = match &target_text {
            None => {
                err(format!("I can't find a \"target[{name}]\" definition"));
                (String::new(), Expr::Num(0.into()), None)
            }
            Some(t) => match table.parse(t, ipv4only, &t_snmp_options, nohc) {
                Err(e) => {
                    err(format!("Target: {e}"));
                    (String::new(), Expr::Num(0.into()), None)
                }
                Ok(p) => match Expr::compile(&p.rewritten) {
                    Ok(e) => (p.rewritten, e, p.unique),
                    Err(e) => {
                        err(format!("Target '{t}': {e}"));
                        (p.rewritten, Expr::Num(0.into()), None)
                    }
                },
            },
        };

        // ---- colours
        let mut colours: Vec<(String, String)> = Vec::new();
        match get("colours") {
            Some(c) => {
                let (re, n) = if options.dorelpercent { (&colour5, 5) } else { (&colour4, 4) };
                match re.captures(&c) {
                    Some(m) => {
                        for i in 0..n {
                            colours.push((m[2 * i + 1].to_string(), m[2 * i + 2].to_string()));
                        }
                    }
                    None => err(format!(
                        "\"colours[{name}]\" use the format: Name#hexcolour, Name#Hexcolour,...{}",
                        if options.dorelpercent { " (dorelpercent requires 5 colours)" } else { "" }
                    )),
                }
            }
            None => {
                colours = vec![
                    ("GREEN".into(), "#00cc00".into()),
                    ("BLUE".into(), "#0000ff".into()),
                    ("DARK GREEN".into(), "#006600".into()),
                    ("MAGENTA".into(), "#ff00ff".into()),
                ];
                if options.dorelpercent {
                    colours.push(("AMBER".into(), "#ef9f4f".into()));
                }
            }
        }
        let background_raw = get("background").unwrap_or_else(|| "#ffffff".into());
        let background = match hexcol.captures(&background_raw) {
            Some(m) => m[1].to_string(),
            None => {
                err(format!("\"background[{name}]: {background_raw}\" use the format: #rrggbb"));
                "#ffffff".into()
            }
        };
        let text_raw = get("textcolor").unwrap_or_else(|| {
            let h = |i: usize| u32::from_str_radix(&background[i..i + 2], 16).unwrap_or(255);
            if h(1) + h(3) + h(5) < 384 { "#ffffff".into() } else { "#000000".into() }
        });
        let text_color = match hexcol.captures(&text_raw) {
            Some(m) => m[1].to_string(),
            None => {
                err(format!("\"textcolor[{name}]: {text_raw}\" use the format: #rrggbb"));
                "#000000".into()
            }
        };

        let (mut timestr_pos, mut timestr_fmt) = (get("timestrpos"), get("timestrfmt"));
        if options.pngdate {
            timestr_pos = Some("RU".into());
            timestr_fmt = Some(if get("timezone").is_some() { "%Y-%m-%d %H:%M %Z" } else { "%Y-%m-%d %H:%M" }.into());
        }
        let set_env = get("setenv")
            .map(|s| setenv_re.captures_iter(&s).map(|c| (c[1].trim().to_string(), c[2].to_string())).collect())
            .unwrap_or_default();

        let thresholds = Thresholds {
            min_i: get("threshmini"),
            min_o: get("threshmino"),
            max_i: get("threshmaxi"),
            max_o: get("threshmaxo"),
            desc: get("threshdesc"),
            prog_i: get("threshprogi").map(program_path),
            prog_o: get("threshprogo").map(program_path),
            prog_ok_i: get("threshprogoki").map(program_path),
            prog_ok_o: get("threshprogoko").map(program_path),
            mail_address: get("threshmailaddress"),
            for_polls: get("threshfor").map(|v| perl_int(&v).clamp(1, u32::MAX as i64) as u32),
        };
        if ["hwthreshmini", "hwthreshmino", "hwthreshmaxi", "hwthreshmaxo"].iter().any(|k| get(k).is_some()) {
            warnings.push(format!("[{name}] HW thresholds need RRDtool's Holt-Winters data and are ignored"));
        }

        let f = |k: &str, d: f64| get(k).map(|v| perl_num(&v)).unwrap_or(d);
        targets.push(Target {
            name: name.clone(),
            target_text: target_text.unwrap_or_default(),
            rewritten,
            expr,
            unique_source,
            title,
            page_top,
            page_foot,
            add_head: get("addhead").map(|s| s.replace("\\n", "\n")),
            body_tag: get("bodytag"),
            directory,
            max_bytes1,
            max_bytes2,
            abs_max,
            xsize: get("xsize").map(|v| perl_int(&v) as u32).unwrap_or(400),
            ysize: get("ysize").map(|v| perl_int(&v) as u32).unwrap_or(100),
            ytics: get("ytics").map(|v| perl_int(&v) as u32).unwrap_or(4),
            yticsfactor: f("yticsfactor", 1.0),
            factor: f("factor", 1.0),
            kilo: get("kilo").map(|v| perl_num(&v) as u64).unwrap_or(1000),
            kmg: get("kmg").map(|k| k.chars().filter(|c| !c.is_whitespace()).collect::<String>().split(',').map(String::from).collect()),
            colours,
            background,
            text_color,
            legend: [get("legend1"), get("legend2"), get("legend3"), get("legend4"), get("legend5")],
            legend_i: get("legendi"),
            legend_o: get("legendo"),
            y_legend: get("ylegend"),
            short_legend: get("shortlegend"),
            png_title: get("pngtitle"),
            unscaled: get("unscaled").unwrap_or_default(),
            with_peak: get("withpeak").unwrap_or_default(),
            suppress: get("suppress").unwrap_or_default(),
            week_format: get("weekformat").and_then(|w| w.chars().find(|c| matches!(c, 'U' | 'V' | 'W'))),
            timezone: get("timezone"),
            set_env,
            extension: get("extension").unwrap_or_else(|| "html".into()),
            router_uptime: get("routeruptime"),
            router_name: get("routername"),
            snmp_options: t_snmp_options,
            xzoom: f("xzoom", 1.0),
            yzoom: f("yzoom", 1.0),
            xscale: f("xscale", 1.0),
            yscale: f("yscale", 1.0),
            timestr_pos: timestr_pos.unwrap_or_else(|| "NO".into()),
            timestr_fmt: timestr_fmt.unwrap_or_else(|| "%Y-%m-%d %H:%M".into()),
            thresholds,
            clone_directory: get("clonedirectory"),
            options,
        });
    }

    // ---- source-level checks
    for (i, s) in table.sources.iter().enumerate() {
        match s {
            Source::Snmp(s) => {
                if let Some(c) = &s.conversion {
                    errors.push(format!(
                        "data source {i} uses conversion subroutine '|{c}', which needs Perl (ConversionCode) and is not supported by LinkScope"
                    ));
                }
                if s.conn.version == crate::SnmpVersion::V3 && !s.snmp_options.contains_key("username") {
                    errors.push(format!(
                        "data source {i} ({}) uses SNMPv3 but its SnmpOptions has no username",
                        s.host
                    ));
                }
            }
            Source::Exec(e) => {
                if needs_shell(&e.command) && !exec_shell {
                    errors.push(format!(
                        "external command `{}` needs a shell (it uses pipes, redirection, variables or globs). \
                         LinkScope runs commands directly; wrap it in a script, or set \"ExecShell: yes\" to allow /bin/sh",
                        e.command
                    ));
                }
            }
        }
    }
    if conversion_note {
        warnings.push("ConversionCode is Perl and is ignored by LinkScope".into());
    }

    if !errors.is_empty() {
        return Err(ConfigError::Invalid(errors));
    }
    Ok(Config {
        path: raw.path.clone(),
        files: raw.files.clone(),
        global,
        targets,
        sources: table.sources,
        warnings,
        raw,
    })
}

/// Convenience for tests and the UI: keyword map of one target.
pub fn target_keywords(raw: &RawConfig, name: &str) -> IndexMap<String, String> {
    raw.per_target
        .iter()
        .filter_map(|(k, m)| m.get(name).map(|v| (k.clone(), v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{normalize_directory, program_path};

    #[test]
    fn a_threshold_program_is_run_as_the_file_that_was_checked() {
        assert_eq!(program_path("/usr/local/bin/notify".into()), "/usr/local/bin/notify");
        let rel = program_path("bin/notify".into());
        assert!(rel.starts_with('/') && rel.ends_with("/bin/notify"), "{rel}");
        assert_eq!(rel, std::env::current_dir().unwrap().join("bin/notify").to_string_lossy());
        // A bare name is the file in the working directory, as checked, not one found on PATH.
        assert!(program_path("notify".into()).ends_with("/notify"));
    }

    #[test]
    fn directory_stays_inside_workdir() {
        assert_eq!(normalize_directory("/routers/").unwrap(), "routers");
        assert_eq!(normalize_directory("a/./b").unwrap(), "a/b");
        assert_eq!(normalize_directory("//etc").unwrap(), "etc");
        assert!(normalize_directory("a/../../x").is_err());
        assert!(normalize_directory("../x").is_err());
    }
}
