// SPDX-License-Identifier: GPL-2.0-or-later
//! Threshold checks, a port of MRTG's `threshcheck` and `threshmail`.
//!
//! For each direction and bound (`ThreshMinI`, `ThreshMaxO`, ...) the day
//! graph's current rate (bytes per second, like MRTG) is compared with the
//! bound, which may be given in percent of `MaxBytes`. With `ThreshDir`, a
//! state file records a broken threshold so the program/mail runs once per
//! transition; without it they run on every poll (subject to MRTG's
//! `ThreshHyst` rule, reproduced as is).
//!
//! LinkScope's additions sit beside this, not inside it: [`evaluate`] is the
//! comparison both use, [`check_evals`] is MRTG's behavior (unchanged for
//! targets without `ThreshFor[]`, see `tests/thresh_mrtg.rs`), and
//! [`transition`] is what a `ThreshFor[]` target runs when
//! [`crate::alerts`] decides an alert fired or cleared.
//!
//! Hardening: programs are executed directly (no shell) in a process group of
//! their own, killed as a group on timeout; state files are created with
//! `O_NOFOLLOW`, and one that cannot be used is remembered in memory rather
//! than firing on every poll; mail headers are built from values with CR/LF
//! removed, and configured secrets are taken out of mail and program
//! arguments ([`redacted`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use linkscope_config::{Config, Target};

/// One action decided by a threshold check.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Run {
        argv: Vec<String>,
        env: Vec<(String, String)>,
    },
    Mail {
        to: String,
        subject: String,
        body: String,
    },
    /// LinkScope: a JSON body for `AlertWebhook`, signed when it is sent.
    Webhook { target: String, body: String },
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn perl_num(s: &str) -> f64 {
    linkscope_config::rules::perl_num(s)
}

fn no_crlf(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// PageTop reduced to text the way MRTG does for alert bodies.
fn pagetop_text(pt: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static H1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<h1>.*?</h1>").unwrap());
    static ROW: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)\s*<tr>\s*<td>(.*?)</td>\s*<td>(.*?)</td>\s*</tr>\s*").unwrap()
    });
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*<.+?>\s*\n?").unwrap());
    let s = H1.replace(pt, "").into_owned();
    let s = ROW.replace_all(&s, "$1 $2\n").into_owned();
    TAG.replace_all(&s, "").into_owned()
}

/// `action` with the configured secrets taken out of what leaves the process: a mail's subject
/// and body, a program's arguments (not the program itself) and its `THRESH_DESC`. A `Title[]`,
/// `ThreshDesc[]` or `PageTop[]` that quotes a community or a password must not mail it, as the
/// webhook body never carries it. `SetEnv[]` is left as configured: passing a value to a program
/// is what it is for. `redact` is the configuration's secret list (`View::redact_secrets`), which
/// leaves ordinary text, e-mail addresses included, as it is.
pub fn redacted(action: Action, redact: &dyn Fn(&str) -> String) -> Action {
    match action {
        Action::Run { mut argv, env } => {
            for a in argv.iter_mut().skip(1) {
                *a = redact(a);
            }
            let env = env
                .into_iter()
                .map(|(k, v)| {
                    if k == "THRESH_DESC" {
                        (k, redact(&v))
                    } else {
                        (k, v)
                    }
                })
                .collect();
            Action::Run { argv, env }
        }
        Action::Mail { to, subject, body } => Action::Mail {
            to,
            subject: redact(&subject),
            body: redact(&body),
        },
        w @ Action::Webhook { .. } => w,
    }
}

/// Where alerts would travel in clear, for a warning at startup and on reload: threshold mail as
/// plain SMTP (no `AlertMailUser`) to a server that is not this host, and an `http://` webhook to
/// a host that is not this one. Both stay allowed (MRTG sends plain SMTP). Never quotes the
/// webhook URL, which may carry a token.
pub fn cleartext_warnings(cfg: &Config) -> Vec<String> {
    let local = |host: &str| {
        let h = host.trim_start_matches('[').trim_end_matches(']');
        h.eq_ignore_ascii_case("localhost")
            || h.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    };
    let mut out = Vec::new();
    let g = &cfg.global;
    let mails = cfg
        .targets
        .iter()
        .any(|t| t.thresholds.mail_address.is_some());
    if let (true, Some(server), None) = (mails, &g.thresh_mail_server, &g.alert_mail_user) {
        let host = match server.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => h,
            _ => server.as_str(),
        };
        if !local(host) {
            out.push(format!(
                "threshold mail to {host} goes as plain SMTP, without TLS, as MRTG sends it; set AlertMailUser and AlertMailPassword to send it over STARTTLS with a login"
            ));
        }
    }
    if let Some(url) = &g.alert_webhook
        && let Ok(u) = reqwest::Url::parse(url)
        && u.scheme() == "http"
        && !u.host_str().is_some_and(local)
    {
        out.push("AlertWebhook uses http://: alert bodies, and any token in its URL, travel unencrypted; use https://".into());
    }
    out
}

/// State-file base for a target: `ThreshDir/<cfg file name>.<target>`.
pub fn state_base(cfg: &Config, t: &Target) -> Option<PathBuf> {
    let dir = cfg.global.thresh_dir.as_ref()?;
    let name = cfg.path.file_name()?.to_string_lossy().into_owned();
    Some(dir.join(format!("{name}.{}", t.name)))
}

/// One configured bound of one direction, compared with this poll's rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Eval {
    /// `'i'` or `'o'`.
    pub dir: char,
    /// `"min"` or `"max"`.
    pub bound: &'static str,
    /// The bound as configured, e.g. `30%` or `37500`.
    pub orig: String,
    pub broken: bool,
    /// The day graph's current rate, in the log's units.
    pub rate: f64,
    /// The bound in the log's units (a percent bound taken of `MaxBytes`).
    pub bound_rate: f64,
    /// The rate in percent of `MaxBytes`, for a percent bound.
    pub percent: Option<f64>,
    // MRTG's figures and their text, exactly as threshcheck formats them.
    boundval: f64,
    threshval: f64,
    boundval_s: String,
    threshval_s: String,
    realthresh: String,
    realval: String,
}

/// Compare every configured bound with the current in/out rates.
pub fn evaluate(t: &Target, cur_in: Option<f64>, cur_out: Option<f64>) -> Vec<Eval> {
    let th = &t.thresholds;
    let mut out = Vec::new();
    for (dir, cur, maxbytes) in [('i', cur_in, t.max_bytes1), ('o', cur_out, t.max_bytes2)] {
        let Some(cur) = cur else { continue };
        for bound in ["min", "max"] {
            let configured = match (bound, dir) {
                ("min", 'i') => &th.min_i,
                ("min", _) => &th.min_o,
                ("max", 'i') => &th.max_i,
                _ => &th.max_o,
            };
            let Some(orig) = configured.clone() else {
                continue;
            };
            let mut boundval_s = orig.clone();
            let mut threshval = cur;
            let (mut realval, mut realthresh) = (String::new(), String::new());
            let mut threshval_s = fmt_num(cur);
            let mut percent = None;
            let mut bound_rate = None;
            if let Some(stripped) = boundval_s.strip_suffix('%') {
                boundval_s = stripped.to_string();
                let b = perl_num(&boundval_s);
                realval = format!("% ({})", fmt_num(cur));
                realthresh = format!("% ({:.1})", maxbytes as f64 * b / 100.0);
                threshval_s = format!(
                    "{:.1}",
                    if maxbytes > 0 {
                        cur / maxbytes as f64 * 100.0
                    } else {
                        0.0
                    }
                );
                threshval = perl_num(&threshval_s);
                percent = Some(threshval);
                bound_rate = Some(maxbytes as f64 * b / 100.0);
            }
            let boundval = perl_num(&boundval_s);
            let broken = (bound == "min" && boundval > threshval)
                || (bound == "max" && boundval < threshval);
            out.push(Eval {
                dir,
                bound,
                orig,
                broken,
                rate: cur,
                bound_rate: bound_rate.unwrap_or(boundval),
                percent,
                boundval,
                threshval,
                boundval_s,
                threshval_s,
                realthresh,
                realval,
            });
        }
    }
    out
}

/// The environment threshold programs run with.
fn prog_env(t: &Target) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = t.set_env.clone();
    if let Some(d) = &t.thresholds.desc {
        env.push(("THRESH_DESC".into(), d.clone()));
    }
    env
}

/// MRTG's program run and mail for one bound in its current state (broken or
/// not), before deciding whether to send them.
fn notices(t: &Target, e: &Eval, env: &[(String, String)]) -> Vec<Action> {
    let th = &t.thresholds;
    let (dir, bound) = (e.dir, e.bound);
    let mut actions = Vec::new();
    let prog = match (e.broken, dir) {
        (true, 'i') => &th.prog_i,
        (true, _) => &th.prog_o,
        (false, 'i') => &th.prog_ok_i,
        (false, _) => &th.prog_ok_o,
    };
    if let Some(p) = prog {
        let mut argv = vec![
            p.clone(),
            t.name.clone(),
            e.orig.clone(),
            e.threshval_s.clone(),
        ];
        if e.broken {
            argv.push(th.desc.clone().unwrap_or_else(|| "No Description".into()));
        }
        actions.push(Action::Run {
            argv,
            env: env.to_vec(),
        });
    }
    if let Some(to) = &th.mail_address {
        let mut body = format!(
            "\n{}\n\n   Target: {}\n     Type: thresh\nDirection: {dir}\n    Bound: {bound}\nThreshold: {}{}\n  Current: {}{}\n\n{}\n\n",
            t.title,
            t.name,
            e.boundval_s,
            e.realthresh,
            e.threshval_s,
            e.realval,
            pagetop_text(t.page_top.as_deref().unwrap_or(""))
        );
        if let Some(d) = &th.desc {
            body.push_str(&format!("\n{d}\n"));
        }
        let subject = if e.broken {
            format!(
                "[MRTG-TH] BROKEN {} thresh {bound} {dir} ( {}{} vs {}{} )",
                t.name, e.boundval_s, e.realthresh, e.threshval_s, e.realval
            )
        } else {
            format!(
                "[MRTG-TH] UN-BROKEN {} thresh {bound} {dir} ( {} vs {})",
                t.name, e.orig, e.threshval_s
            )
        };
        let body = format!(
            "Threshold {}\n{}\n{body}",
            if e.broken { "BROKEN" } else { "UN-BROKEN" },
            if e.broken {
                "----------------"
            } else {
                "-------------------"
            }
        );
        actions.push(Action::Mail {
            to: to.clone(),
            subject,
            body,
        });
    }
    actions
}

fn state_file(base: &Path, e: &Eval) -> PathBuf {
    PathBuf::from(format!(
        "{}.thresh{}{}",
        base.display(),
        e.bound,
        e.dir.to_ascii_uppercase()
    ))
}

/// Whether a `ThreshDir` state file says the bound was reported broken: `Ok(true)` for a regular
/// file of ours, `Ok(false)` when there is none, an error when something else stands there (a
/// directory, a FIFO, a link, a file of another account), which cannot be trusted either way.
fn state_present(s: &Path) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(s) {
        Ok(m) if !m.file_type().is_file() => Err(format!("{} is not a regular file", s.display())),
        // SAFETY: geteuid takes no arguments, has no preconditions and cannot fail.
        Ok(m) if m.uid() != unsafe { libc::geteuid() } => Err(format!(
            "{} belongs to uid {}, not to LinkScope's account",
            s.display(),
            m.uid()
        )),
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", s.display())),
    }
}

/// Bounds reported broken whose state file could not be used: remembered here until they clear
/// (or LinkScope restarts), so the notice goes once per change, not on every poll.
static UNSTORED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn unstored(f: impl FnOnce(&mut HashSet<PathBuf>) -> bool) -> bool {
    f(UNSTORED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashSet::new))
}

/// MRTG's once-per-change rule with `ThreshDir`: whether to notify now. A state file that cannot
/// be created or read is an error in the log, and the change is remembered in memory instead.
fn state_fires(s: &Path, broken: bool) -> bool {
    match (broken, state_present(s)) {
        (true, Ok(true)) => false,
        (true, Ok(false)) => match crate::fsutil::create_new(s, 0o644) {
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(err) => {
                tracing::error!(
                    "creating threshold state {}: {err}; remembering it in memory instead",
                    s.display()
                );
                unstored(|m| m.insert(s.to_path_buf()))
            }
        },
        (true, Err(why)) => {
            tracing::error!("threshold state {why}; remembering it in memory instead");
            unstored(|m| m.insert(s.to_path_buf()))
        }
        (false, Ok(true)) => {
            let _ = std::fs::remove_file(s);
            unstored(|m| m.remove(s));
            true
        }
        (false, Ok(false)) => unstored(|m| m.remove(s)),
        (false, Err(why)) => {
            tracing::error!("threshold state {why}; left as it is");
            unstored(|m| m.remove(s))
        }
    }
}

/// Evaluate thresholds for one target given the current in/out rates, as
/// MRTG does. State files are updated here; the returned actions are
/// executed by the caller (see [`execute`]).
pub fn check(cfg: &Config, t: &Target, cur_in: Option<f64>, cur_out: Option<f64>) -> Vec<Action> {
    check_evals(cfg, t, &evaluate(t, cur_in, cur_out))
}

/// MRTG's decisions for already evaluated bounds: with `ThreshDir` once per
/// transition, without it on every broken poll (subject to `ThreshHyst`).
pub fn check_evals(cfg: &Config, t: &Target, evals: &[Eval]) -> Vec<Action> {
    let mut actions = Vec::new();
    let base = state_base(cfg, t);
    let env = prog_env(t);
    for e in evals {
        let state = base.as_ref().map(|b| state_file(b, e));
        let (bound, boundval, threshval) = (e.bound, e.boundval, e.threshval);
        let fire = match (&state, e.broken) {
            (Some(s), broken) => state_fires(s, broken),
            (None, true) => {
                // MRTG's rule without ThreshDir (hysteresis only if set explicitly).
                let hyst = cfg.raw.global.get("threshhyst").map(|h| perl_num(h));
                match hyst {
                    None => true,
                    Some(h) => {
                        (bound == "min" && boundval - h * boundval < threshval)
                            || (bound == "max" && boundval + h * boundval > threshval)
                    }
                }
            }
            (None, false) => false,
        };
        if fire {
            actions.extend(notices(t, e, &env));
        }
    }
    actions
}

/// What a `ThreshFor[]` target runs when one of its alerts fires (`e` is
/// broken) or clears (`e` is not): MRTG's program and mail for that state,
/// once. `ThreshDir`'s state file follows, for tools that read it.
pub fn transition(cfg: &Config, t: &Target, e: &Eval) -> Vec<Action> {
    if let Some(s) = state_base(cfg, t).map(|b| state_file(&b, e)) {
        // Kept for tools that read it; the alert book decides, so only errors are reported.
        match (e.broken, state_present(&s)) {
            (_, Err(why)) => tracing::error!("threshold state {why}; left as it is"),
            (true, Ok(false)) => match crate::fsutil::create_new(&s, 0o644) {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => tracing::error!("creating threshold state {}: {err}", s.display()),
            },
            (false, Ok(true)) => {
                let _ = std::fs::remove_file(&s);
            }
            _ => {}
        }
    }
    notices(t, e, &prog_env(t))
}

/// Execute threshold actions: programs, mails and webhooks run concurrently
/// (at most eight at a time) with per-action timeouts, one SMTP transport
/// and one HTTP client are shared, and the whole batch is abandoned after
/// ten minutes.
pub async fn execute(cfg: &Config, actions: Vec<Action>) {
    let mailer = match mail_transport(cfg) {
        Ok(m) => Some(std::sync::Arc::new(m)),
        Err(e) => {
            if actions.iter().any(|a| matches!(a, Action::Mail { .. })) {
                tracing::warn!("threshold mail disabled: {e}");
            }
            None
        }
    };
    let hook = match (&cfg.global.alert_webhook, &cfg.global.alert_webhook_secret) {
        (Some(url), Some(secret))
            if actions.iter().any(|a| matches!(a, Action::Webhook { .. })) =>
        {
            match crate::alerts::Hook::new(url, secret) {
                Ok(h) => Some(std::sync::Arc::new(h)),
                Err(e) => {
                    tracing::warn!("alert webhook disabled: {e}");
                    None
                }
            }
        }
        _ => None,
    };
    let sender = cfg.global.thresh_mail_sender.clone();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = tokio::task::JoinSet::new();
    for a in actions {
        let (sem, mailer, sender, hook) =
            (sem.clone(), mailer.clone(), sender.clone(), hook.clone());
        set.spawn(async move {
            let _p = sem.acquire_owned().await;
            match a {
                Action::Run { argv, env } => run_program(&argv, env, PROGRAM_TIMEOUT).await,
                Action::Mail { to, subject, body } => {
                    if let (Some(m), Some(from)) = (mailer, sender)
                        && let Err(e) = send_mail(&m, &from, &to, &subject, &body).await
                    {
                        tracing::warn!("sending threshold mail to {to}: {e}");
                    }
                }
                Action::Webhook { target, body } => {
                    if let Some(h) = hook {
                        h.send(&target, body).await;
                    }
                }
            }
        });
    }
    let all = async { while set.join_next().await.is_some() {} };
    if tokio::time::timeout(Duration::from_secs(600), all)
        .await
        .is_err()
    {
        tracing::warn!("threshold actions still running after 10 minutes; abandoning them");
    }
}

/// How long a threshold program may run.
pub const PROGRAM_TIMEOUT: Duration = Duration::from_secs(60);

/// Kills a program's process group when dropped: on its timeout, and when the batch it belongs to
/// is abandoned (a stop, the ten-minute limit). Killing only the program left what it started
/// running.
struct Group(Option<i32>);

impl Drop for Group {
    fn drop(&mut self) {
        if let Some(pg) = self.0 {
            // SAFETY: kill(2) with a negative pid signals the process group the child leads.
            unsafe { libc::kill(-pg, libc::SIGKILL) };
        }
    }
}

/// Run one threshold program directly (no shell), in a process group of its own, for at most
/// `timeout`; its whole group is killed when it times out or is abandoned.
pub async fn run_program(argv: &[String], env: Vec<(String, String)>, timeout: Duration) {
    let Some(prog) = argv.first() else { return };
    let mut c = tokio::process::Command::new(prog);
    c.args(&argv[1..])
        .envs(env)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    match c.spawn() {
        Ok(mut child) => {
            let group = Group(child.id().and_then(|p| i32::try_from(p).ok()));
            match tokio::time::timeout(timeout, child.wait()).await {
                // Exited: what it left behind is its business, as in MRTG.
                Ok(_) => std::mem::forget(group),
                Err(_) => {
                    tracing::warn!(
                        "threshold program {prog} timed out after {} s; its process group is killed",
                        timeout.as_secs()
                    );
                    drop(group);
                    let _ = child.wait().await;
                }
            }
        }
        Err(e) => tracing::warn!("running threshold program {prog}: {e}"),
    }
}

type Mailer = lettre::AsyncSmtpTransport<lettre::Tokio1Executor>;

/// `ThreshMailServer` may be `host` or `host:port`. Without `AlertMailUser`
/// the mail goes as MRTG sends it: plain SMTP, port 25 by default. With it,
/// LinkScope requires STARTTLS (a certificate the system trusts for that
/// host name; `SSL_CERT_FILE` replaces the system's CAs) and logs in, port
/// 587 by default.
fn mail_transport(cfg: &Config) -> anyhow::Result<Mailer> {
    let server = cfg
        .global
        .thresh_mail_server
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("ThreshMailServer not set"))?;
    let login = cfg
        .global
        .alert_mail_user
        .as_ref()
        .zip(cfg.global.alert_mail_password.as_ref());
    let default_port = if login.is_some() { 587 } else { 25 };
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => {
            (h, p.parse::<u16>().unwrap_or(default_port))
        }
        _ => (server, default_port),
    };
    let timeout = Some(Duration::from_secs(30));
    Ok(match login {
        None => Mailer::builder_dangerous(host)
            .port(port)
            .timeout(timeout)
            .build(),
        Some((user, password)) => {
            use lettre::transport::smtp::authentication::Credentials;
            Mailer::starttls_relay(host)?
                .port(port)
                .credentials(Credentials::new(user.clone(), password.clone()))
                .timeout(timeout)
                .build()
        }
    })
}

async fn send_mail(
    mailer: &Mailer,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    use lettre::{AsyncTransport, Message};
    let mut builder = Message::builder()
        .from(no_crlf(from).parse()?)
        .subject(no_crlf(subject));
    for addr in to.split([',', ' ']).filter(|a| !a.is_empty()) {
        builder = builder.to(no_crlf(addr).parse()?);
    }
    let msg = builder.body(body.to_string())?;
    mailer.send(msg).await?;
    Ok(())
}

/// Check that a program path given in the config exists and is executable.
pub fn program_ok(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(dir: &Path, body: &str) -> Config {
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let path = dir.join("m.cfg");
        std::fs::write(&path, format!("WorkDir: {}\n{body}", work.display())).unwrap();
        linkscope_config::load(
            &path,
            &linkscope_config::ReadOptions {
                trust: linkscope_config::TrustPolicy::Off,
            },
            &linkscope_config::CheckOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn secrets_leave_neither_by_mail_nor_as_program_arguments() {
        let redact = |t: &str| t.replace("comm-SECRET", "•••");
        let mail = Action::Mail {
            to: "noc@example.com".into(),
            subject: "[MRTG-TH] BROKEN r1 comm-SECRET".into(),
            body: "Title comm-SECRET\nmail noc@example.com".into(),
        };
        assert_eq!(
            redacted(mail, &redact),
            Action::Mail {
                to: "noc@example.com".into(),
                subject: "[MRTG-TH] BROKEN r1 •••".into(),
                body: "Title •••\nmail noc@example.com".into()
            }
        );
        let run = Action::Run {
            argv: vec![
                "/opt/comm-SECRET/notify".into(),
                "r1".into(),
                "30%".into(),
                "41.7".into(),
                "desc comm-SECRET".into(),
            ],
            env: vec![
                ("THRESH_DESC".into(), "desc comm-SECRET".into()),
                ("TOKEN".into(), "comm-SECRET".into()),
            ],
        };
        assert_eq!(
            redacted(run, &redact),
            Action::Run {
                // The program is run as configured; SetEnv passes what it was given.
                argv: vec![
                    "/opt/comm-SECRET/notify".into(),
                    "r1".into(),
                    "30%".into(),
                    "41.7".into(),
                    "desc •••".into()
                ],
                env: vec![
                    ("THRESH_DESC".into(), "desc •••".into()),
                    ("TOKEN".into(), "comm-SECRET".into())
                ],
            }
        );
        let hook = Action::Webhook {
            target: "r1".into(),
            body: "{}".into(),
        };
        assert_eq!(redacted(hook.clone(), &redact), hook);
    }

    #[test]
    fn alerts_in_clear_are_warned_about() {
        let d = tempfile::tempdir().unwrap();
        let target = "Target[r1]: 2:public@192.0.2.1\nTitle[r1]: R1\nMaxBytes[r1]: 100\nThreshMaxI[r1]: 30%\n";
        let cfg = |g: &str| {
            let mail = if g.contains("ThreshMailServer") {
                "ThreshMailAddress[r1]: noc@example.com\n"
            } else {
                ""
            };
            load(d.path(), &format!("{g}{target}{mail}"))
        };
        let plain = cleartext_warnings(&cfg(
            "ThreshMailServer: 192.0.2.25\nThreshMailSender: lab@example.com\n",
        ));
        assert!(
            plain.len() == 1 && plain[0].contains("plain SMTP") && plain[0].contains("192.0.2.25"),
            "{plain:?}"
        );
        assert!(
            cleartext_warnings(&cfg(
                "ThreshMailServer: 127.0.0.1:25\nThreshMailSender: lab@example.com\n"
            ))
            .is_empty()
        );
        assert!(cleartext_warnings(&cfg("ThreshMailServer: 192.0.2.25:587\nThreshMailSender: lab@example.com\nAlertMailUser: lab\nAlertMailPassword: pw-SECRET\n")).is_empty());
        let hook = cleartext_warnings(&cfg(
            "AlertWebhook: http://hooks.example.com/t/tok-SECRET\nAlertWebhookSecret: 0123456789abcdef-x\n",
        ));
        assert!(
            hook.len() == 1
                && hook[0].contains("http://")
                && !hook[0].contains("tok-SECRET")
                && !hook[0].contains("hooks.example.com"),
            "{hook:?}"
        );
        assert!(cleartext_warnings(&cfg("AlertWebhook: https://hooks.example.com/t\nAlertWebhookSecret: 0123456789abcdef-x\n")).is_empty());
        assert!(
            cleartext_warnings(&cfg(
                "AlertWebhook: http://127.0.0.1:9000/t\nAlertWebhookSecret: 0123456789abcdef-x\n"
            ))
            .is_empty()
        );
    }

    /// With ThreshDir, MRTG notifies once per change. A state file that cannot be used must not
    /// turn that into a notice on every poll, nor silence the change.
    #[test]
    fn an_unusable_state_file_notifies_once_per_change() {
        let d = tempfile::tempdir().unwrap();
        let tdir = d.path().join("thresh");
        std::fs::create_dir(&tdir).unwrap();
        let c = load(
            d.path(),
            &format!(
                "ThreshDir: {}\nTarget[r1]: 2:public@192.0.2.1\nTitle[r1]: R1\nMaxBytes[r1]: 1000\nThreshMaxI[r1]: 30%\nThreshProgI[r1]: /bin/true\nThreshProgOKI[r1]: /bin/true\n",
                tdir.display()
            ),
        );
        let t = &c.targets[0];
        let runs = |r: f64| {
            check(&c, t, Some(r), Some(0.0))
                .iter()
                .filter(|a| matches!(a, Action::Run { .. }))
                .count()
        };
        // A directory stands where the state file goes: neither usable nor removable as a file.
        let state = state_file(
            &state_base(&c, t).unwrap(),
            &evaluate(t, Some(900.0), Some(0.0))[0],
        );
        std::fs::create_dir(&state).unwrap();
        assert_eq!(
            [runs(900.0), runs(900.0), runs(900.0)],
            [1, 0, 0],
            "broken: once"
        );
        assert_eq!([runs(10.0), runs(10.0)], [1, 0], "cleared: once");
        // And a ThreshDir that has gone (creating the file fails): the same, from memory.
        std::fs::remove_dir(&state).unwrap();
        std::fs::remove_dir(&tdir).unwrap();
        assert_eq!(
            [runs(900.0), runs(900.0), runs(10.0), runs(10.0)],
            [1, 0, 1, 0]
        );
    }

    /// A program that times out is killed with everything it started.
    #[tokio::test]
    async fn a_program_that_times_out_takes_its_process_group_with_it() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("pid");
        let script = format!("sleep 30 & echo $! > {}; sleep 30", pidfile.display());
        let started = std::time::Instant::now();
        run_program(
            &["/bin/sh".into(), "-c".into(), script],
            vec![],
            Duration::from_millis(500),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Gone, or a zombie waiting for its new parent to reap it.
        let alive = |pid: i32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|s| !s.split(") ").nth(1).is_some_and(|r| r.starts_with('Z')))
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while alive(pid) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(pid), "the program's background child {pid} survived");
    }
}
