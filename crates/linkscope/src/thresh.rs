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

/// Evaluate thresholds for one target given the current in/out rates.
/// State files are updated here; the returned actions are executed by the
/// caller (see [`execute`]).
pub fn check(cfg: &Config, t: &Target, cur_in: Option<f64>, cur_out: Option<f64>) -> Vec<Action> {
    let mut actions = Vec::new();
    let th = &t.thresholds;
    let base = state_base(cfg, t);
    let mut env: Vec<(String, String)> = t.set_env.clone();
    if let Some(d) = &th.desc {
        env.push(("THRESH_DESC".into(), d.clone()));
    }
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
            if let Some(stripped) = boundval_s.strip_suffix('%') {
                boundval_s = stripped.to_string();
                let b = perl_num(&boundval_s);
                realval = format!("% ({})", fmt_num(cur));
                realthresh = format!("% ({:.1})", maxbytes as f64 * b / 100.0);
                threshval_s = format!("{:.1}", if maxbytes > 0 { cur / maxbytes as f64 * 100.0 } else { 0.0 });
                threshval = perl_num(&threshval_s);
            }
            let boundval = perl_num(&boundval_s);
            let dir_uc = dir.to_ascii_uppercase();
            let mut body = format!(
                "\n{}\n\n   Target: {}\n     Type: thresh\nDirection: {dir}\n    Bound: {bound}\nThreshold: {boundval_s}{realthresh}\n  Current: {threshval_s}{realval}\n\n{}\n\n",
                t.title,
                t.name,
                pagetop_text(t.page_top.as_deref().unwrap_or(""))
            );
            if let Some(d) = &th.desc {
                body.push_str(&format!("\n{d}\n"));
            }
            let broken = (bound == "min" && boundval > threshval) || (bound == "max" && boundval < threshval);
            let state = base.as_ref().map(|b| PathBuf::from(format!("{}.thresh{bound}{dir_uc}", b.display())));
            let prog = if broken {
                if dir == 'i' { &th.prog_i } else { &th.prog_o }
            } else if dir == 'i' {
                &th.prog_ok_i
            } else {
                &th.prog_ok_o
            };
            let mut argv: Vec<String> = Vec::new();
            if let Some(p) = prog {
                argv = vec![p.clone(), t.name.clone(), orig.clone(), threshval_s.clone()];
                if broken {
                    argv.push(th.desc.clone().unwrap_or_else(|| "No Description".into()));
                }
            }
            let subject = if broken {
                format!("[MRTG-TH] BROKEN {} thresh {bound} {dir} ( {boundval_s}{realthresh} vs {threshval_s}{realval} )", t.name)
            } else {
                format!("[MRTG-TH] UN-BROKEN {} thresh {bound} {dir} ( {orig} vs {threshval_s})", t.name)
            };
            let full_body = format!(
                "Threshold {}\n{}\n{body}",
                if broken { "BROKEN" } else { "UN-BROKEN" },
                if broken { "----------------" } else { "-------------------" }
            );
            let fire = match (&state, broken) {
                (Some(s), true) => match crate::fsutil::create_new(s, 0o644) {
                    Ok(_) => true,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
                    Err(e) => {
                        tracing::warn!("creating {}: {e}", s.display());
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
            if !fire {
                continue;
            }
            if !argv.is_empty() {
                actions.push(Action::Run { argv, env: env.clone() });
            }
            if let Some(to) = &th.mail_address {
                actions.push(Action::Mail { to: to.clone(), subject, body: full_body });
            }
        }
    }
    actions
}

/// Execute threshold actions: programs and mails run concurrently (at most
/// eight at a time) with per-action timeouts, one SMTP transport is shared,
/// and the whole batch is abandoned after ten minutes.
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
    let sender = cfg.global.thresh_mail_sender.clone();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = tokio::task::JoinSet::new();
    for a in actions {
        let (sem, mailer, sender) = (sem.clone(), mailer.clone(), sender.clone());
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
            }
        });
    }
    let all = async { while set.join_next().await.is_some() {} };
    if tokio::time::timeout(Duration::from_secs(600), all).await.is_err() {
        tracing::warn!("threshold actions still running after 10 minutes; abandoning them");
    }
}

type Mailer = lettre::AsyncSmtpTransport<lettre::Tokio1Executor>;

/// `ThreshMailServer` may be `host` or `host:port` (port 25 by default).
fn mail_transport(cfg: &Config) -> anyhow::Result<Mailer> {
    let server = cfg.global.thresh_mail_server.as_deref().ok_or_else(|| anyhow::anyhow!("ThreshMailServer not set"))?;
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => (h, p.parse::<u16>().unwrap_or(25)),
        _ => (server, 25),
    };
    Ok(Mailer::builder_dangerous(host).port(port).timeout(Some(Duration::from_secs(30))).build())
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
