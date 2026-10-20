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
//! Hardening: programs are executed directly (no shell) with a timeout,
//! state files are created with `O_NOFOLLOW`, and mail headers are built
//! from values with CR/LF removed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use linkscope_config::{Config, Target};

/// One action decided by a threshold check.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Run { argv: Vec<String>, env: Vec<(String, String)> },
    Mail { to: String, subject: String, body: String },
    /// LinkScope: a JSON body for `AlertWebhook`, signed when it is sent.
    Webhook { target: String, body: String },
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
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
    static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\s*<tr>\s*<td>(.*?)</td>\s*<td>(.*?)</td>\s*</tr>\s*").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*<.+?>\s*\n?").unwrap());
    let s = H1.replace(pt, "").into_owned();
    let s = ROW.replace_all(&s, "$1 $2\n").into_owned();
    TAG.replace_all(&s, "").into_owned()
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
            let Some(orig) = configured.clone() else { continue };
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
                threshval_s = format!("{:.1}", if maxbytes > 0 { cur / maxbytes as f64 * 100.0 } else { 0.0 });
                threshval = perl_num(&threshval_s);
                percent = Some(threshval);
                bound_rate = Some(maxbytes as f64 * b / 100.0);
            }
            let boundval = perl_num(&boundval_s);
            let broken = (bound == "min" && boundval > threshval) || (bound == "max" && boundval < threshval);
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
        let mut argv = vec![p.clone(), t.name.clone(), e.orig.clone(), e.threshval_s.clone()];
        if e.broken {
            argv.push(th.desc.clone().unwrap_or_else(|| "No Description".into()));
        }
        actions.push(Action::Run { argv, env: env.to_vec() });
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
            format!("[MRTG-TH] BROKEN {} thresh {bound} {dir} ( {}{} vs {}{} )", t.name, e.boundval_s, e.realthresh, e.threshval_s, e.realval)
        } else {
            format!("[MRTG-TH] UN-BROKEN {} thresh {bound} {dir} ( {} vs {})", t.name, e.orig, e.threshval_s)
        };
        let body = format!(
            "Threshold {}\n{}\n{body}",
            if e.broken { "BROKEN" } else { "UN-BROKEN" },
            if e.broken { "----------------" } else { "-------------------" }
        );
        actions.push(Action::Mail { to: to.clone(), subject, body });
    }
    actions
}

fn state_file(base: &Path, e: &Eval) -> PathBuf {
    PathBuf::from(format!("{}.thresh{}{}", base.display(), e.bound, e.dir.to_ascii_uppercase()))
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
            (Some(s), true) => match crate::fsutil::create_new(s, 0o644) {
                Ok(_) => true,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(err) => {
                    tracing::warn!("creating {}: {err}", s.display());
                    true
                }
            },
            (Some(s), false) => match std::fs::symlink_metadata(s) {
                Ok(_) => {
                    let _ = std::fs::remove_file(s);
                    true
                }
                Err(_) => false,
            },
            (None, true) => {
                // MRTG's rule without ThreshDir (hysteresis only if set explicitly).
                let hyst = cfg.raw.global.get("threshhyst").map(|h| perl_num(h));
                match hyst {
                    None => true,
                    Some(h) => (bound == "min" && boundval - h * boundval < threshval) || (bound == "max" && boundval + h * boundval > threshval),
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
        if e.broken {
            match crate::fsutil::create_new(&s, 0o644) {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => tracing::warn!("creating {}: {err}", s.display()),
            }
        } else if std::fs::symlink_metadata(&s).is_ok() {
            let _ = std::fs::remove_file(&s);
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
        (Some(url), Some(secret)) if actions.iter().any(|a| matches!(a, Action::Webhook { .. })) => match crate::alerts::Hook::new(url, secret) {
            Ok(h) => Some(std::sync::Arc::new(h)),
            Err(e) => {
                tracing::warn!("alert webhook disabled: {e}");
                None
            }
        },
        _ => None,
    };
    let sender = cfg.global.thresh_mail_sender.clone();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = tokio::task::JoinSet::new();
    for a in actions {
        let (sem, mailer, sender, hook) = (sem.clone(), mailer.clone(), sender.clone(), hook.clone());
        set.spawn(async move {
            let _p = sem.acquire_owned().await;
            match a {
                Action::Run { argv, env } => {
                    let mut c = tokio::process::Command::new(&argv[0]);
                    c.args(&argv[1..]).envs(env).stdin(std::process::Stdio::null()).kill_on_drop(true);
                    match c.spawn() {
                        Ok(mut child) => {
                            if tokio::time::timeout(Duration::from_secs(60), child.wait()).await.is_err() {
                                tracing::warn!("threshold program {} timed out after 60s", argv[0]);
                            }
                        }
                        Err(e) => tracing::warn!("running threshold program {}: {e}", argv[0]),
                    }
                }
                Action::Mail { to, subject, body } => {
                    if let (Some(m), Some(from)) = (mailer, sender) {
                        if let Err(e) = send_mail(&m, &from, &to, &subject, &body).await {
                            tracing::warn!("sending threshold mail to {to}: {e}");
                        }
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
    if tokio::time::timeout(Duration::from_secs(600), all).await.is_err() {
        tracing::warn!("threshold actions still running after 10 minutes; abandoning them");
    }
}

type Mailer = lettre::AsyncSmtpTransport<lettre::Tokio1Executor>;

/// `ThreshMailServer` may be `host` or `host:port`. Without `AlertMailUser`
/// the mail goes as MRTG sends it: plain SMTP, port 25 by default. With it,
/// LinkScope requires STARTTLS (a certificate the system trusts for that
/// host name; `SSL_CERT_FILE` replaces the system's CAs) and logs in, port
/// 587 by default.
fn mail_transport(cfg: &Config) -> anyhow::Result<Mailer> {
    let server = cfg.global.thresh_mail_server.as_deref().ok_or_else(|| anyhow::anyhow!("ThreshMailServer not set"))?;
    let login = cfg.global.alert_mail_user.as_ref().zip(cfg.global.alert_mail_password.as_ref());
    let default_port = if login.is_some() { 587 } else { 25 };
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => (h, p.parse::<u16>().unwrap_or(default_port)),
        _ => (server, default_port),
    };
    let timeout = Some(Duration::from_secs(30));
    Ok(match login {
        None => Mailer::builder_dangerous(host).port(port).timeout(timeout).build(),
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

async fn send_mail(mailer: &Mailer, from: &str, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
    use lettre::{AsyncTransport, Message};
    let mut builder = Message::builder().from(no_crlf(from).parse()?).subject(no_crlf(subject));
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
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}
