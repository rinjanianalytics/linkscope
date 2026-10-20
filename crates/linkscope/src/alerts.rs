// SPDX-License-Identifier: GPL-2.0-or-later
//! LinkScope's alert state, beside MRTG's threshold checks.
//!
//! For every configured threshold bound the [`Book`] counts how many polls in
//! a row it has been broken. A bound fires once it has been broken for
//! `ThreshFor[]` polls (one when unset) and resolves at the first poll it is
//! not. Each change is one [`Event`], which becomes a signed webhook
//! (`AlertWebhook:`) and, for `ThreshFor[]` targets, MRTG's program and mail
//! for that state ([`crate::thresh::transition`]). The book also gives the
//! API its warn state.
//!
//! The book is kept in `<cache>.alerts` (beside the `.ok` cache), so a
//! config reload or a restart neither repeats a notice nor loses one.
//!
//! A poll without a rate for a direction (no answer, no log update) leaves
//! that direction's bounds as they were: missing data neither fires nor
//! clears an alert.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use linkscope_config::{Config, Target};
use linkscope_web::AlertLive;
use serde::{Deserialize, Serialize};

use crate::thresh::{Action, Eval};

/// One bound's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// `in` or `out`, and `min` or `max`.
    pub direction: String,
    pub bound: String,
    /// Polls in a row the bound has been broken.
    pub polls: u32,
    /// The first broken poll, and the poll that fired it.
    pub since: i64,
    pub fired_at: Option<i64>,
    pub firing: bool,
    /// The latest poll's figures (log units; `per` seconds per unit).
    pub threshold: String,
    pub threshold_rate: f64,
    pub rate: f64,
    pub percent: Option<f64>,
    pub per: f64,
    pub time: i64,
}

impl Entry {
    fn update(&mut self, e: &Eval, per: f64, time: i64) {
        self.threshold = e.orig.clone();
        self.threshold_rate = e.bound_rate;
        self.rate = e.rate;
        self.percent = e.percent;
        self.per = per;
        self.time = time;
    }
}

/// What happened to a bound this poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Broken for `ThreshFor[]` polls.
    Fired,
    /// Not broken any more.
    Cleared,
    /// The bound (or its target) left the configuration while firing.
    Removed,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub target: String,
    pub change: Change,
    pub entry: Entry,
    /// This poll's evaluation (none for [`Change::Removed`]).
    pub eval: Option<Eval>,
}

fn key(e: &Eval) -> String {
    format!("{}_{}", e.bound, e.dir)
}

fn direction(dir: char) -> &'static str {
    if dir == 'i' { "in" } else { "out" }
}

/// Seconds per unit of the target's log: `perminute`, `perhour`, else 1.
pub fn unit_secs(t: &Target) -> f64 {
    if t.options.perhour {
        3600.0
    } else if t.options.perminute {
        60.0
    } else {
        1.0
    }
}

/// Every target's bounds that are broken now.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Book {
    #[serde(default)]
    targets: BTreeMap<String, BTreeMap<String, Entry>>,
    #[serde(skip)]
    dirty: bool,
}

impl Book {
    /// The state file for a `.ok` cache path.
    pub fn path_for(cache_path: &Path) -> PathBuf {
        cache_path.with_extension("alerts")
    }

    /// Read the book; a missing file is an empty book, an unreadable one is
    /// reported and started afresh.
    pub fn load(path: &Path) -> Book {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}; starting with no alerts", path.display());
                Book::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Book::default(),
            Err(e) => {
                tracing::warn!("reading {}: {e}; starting with no alerts", path.display());
                Book::default()
            }
        }
    }

    /// Write the book if it changed since the last save.
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_atomic(path, &data, 0o644)?;
        self.dirty = false;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Forget bounds that are no longer configured. Those that were firing
    /// come back as [`Change::Removed`], so a receiver is told they ended.
    pub fn prune(&mut self, cfg: &Config) -> Vec<Event> {
        let mut events = Vec::new();
        let configured: BTreeMap<&str, &Target> = cfg.targets.iter().map(|t| (t.name.as_str(), t)).collect();
        self.targets.retain(|name, bounds| {
            let t = configured.get(name.as_str());
            bounds.retain(|k, entry| {
                let keep = t.is_some_and(|t| {
                    let th = &t.thresholds;
                    match k.as_str() {
                        "min_i" => th.min_i.is_some(),
                        "min_o" => th.min_o.is_some(),
                        "max_i" => th.max_i.is_some(),
                        "max_o" => th.max_o.is_some(),
                        _ => false,
                    }
                });
                if !keep {
                    self.dirty = true;
                    if entry.firing {
                        events.push(Event { target: name.clone(), change: Change::Removed, entry: entry.clone(), eval: None });
                    }
                }
                keep
            });
            !bounds.is_empty()
        });
        events
    }

    /// Take one poll's evaluation of a target's bounds; returns the alerts
    /// that fired or cleared.
    pub fn observe(&mut self, t: &Target, evals: &[Eval], time: i64) -> Vec<Event> {
        let need = t.thresholds.for_polls.unwrap_or(1).max(1);
        let per = unit_secs(t);
        let mut events = Vec::new();
        for e in evals {
            let k = key(e);
            if e.broken {
                let bounds = self.targets.entry(t.name.clone()).or_default();
                let entry = bounds.entry(k).or_insert_with(|| Entry {
                    direction: direction(e.dir).into(),
                    bound: e.bound.into(),
                    polls: 0,
                    since: time,
                    fired_at: None,
                    firing: false,
                    threshold: String::new(),
                    threshold_rate: 0.0,
                    rate: 0.0,
                    percent: None,
                    per,
                    time,
                });
                entry.polls = entry.polls.saturating_add(1);
                entry.update(e, per, time);
                if !entry.firing && entry.polls >= need {
                    entry.firing = true;
                    entry.fired_at = Some(time);
                    events.push(Event { target: t.name.clone(), change: Change::Fired, entry: entry.clone(), eval: Some(e.clone()) });
                }
                self.dirty = true;
            } else if let Some(bounds) = self.targets.get_mut(&t.name) {
                if let Some(mut entry) = bounds.remove(&k) {
                    self.dirty = true;
                    if entry.firing {
                        entry.update(e, per, time);
                        events.push(Event { target: t.name.clone(), change: Change::Cleared, entry, eval: Some(e.clone()) });
                    }
                }
                if bounds.is_empty() {
                    self.targets.remove(&t.name);
                }
            }
        }
        events
    }

    /// The target's broken bounds, for the API.
    pub fn live(&self, t: &Target) -> Vec<AlertLive> {
        let need = t.thresholds.for_polls.unwrap_or(1).max(1);
        self.targets
            .get(&t.name)
            .map(|bounds| {
                bounds
                    .values()
                    .map(|e| AlertLive {
                        direction: e.direction.clone(),
                        bound: e.bound.clone(),
                        threshold: e.threshold.clone(),
                        threshold_rate: e.threshold_rate,
                        rate: e.rate,
                        percent: e.percent,
                        firing: e.firing,
                        polls: e.polls,
                        polls_needed: need,
                        since: e.since,
                        fired_at: e.fired_at,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The webhook's JSON body for one change.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    /// Always `threshold`.
    pub event: String,
    /// `firing` or `resolved`.
    pub state: String,
    /// For `resolved`: `cleared` (the rate is back within the bound) or
    /// `removed` (the bound left the configuration).
    pub reason: Option<String>,
    pub target: String,
    /// `Title[]` as text.
    pub title: String,
    pub directory: String,
    /// `in` or `out`, `min` or `max`.
    pub direction: String,
    pub bound: String,
    /// The bound as configured (`30%`), and the bound and the rate per
    /// second (bytes per second for interface counters).
    pub threshold: String,
    pub threshold_rate: f64,
    pub rate: f64,
    /// The rate in percent of `MaxBytes`, for a percent bound.
    pub percent: Option<f64>,
    /// Polls in a row the bound has been broken: when it fired, or up to
    /// the poll that cleared it.
    pub polls: u32,
    /// First broken poll, and this poll (Unix time).
    pub since: i64,
    pub time: i64,
    /// `ThreshDesc[]`.
    pub description: Option<String>,
    pub linkscope: String,
}

/// `Title[]` without its HTML.
fn plain(html: &str) -> String {
    use std::sync::LazyLock;
    static TAG: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"<[^>]*>").unwrap());
    let s = TAG.replace_all(html, "");
    s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&").trim().to_string()
}

pub fn payload(cfg: &Config, view: &linkscope_web::api::View, ev: &Event) -> Payload {
    let t = cfg.targets.iter().find(|t| t.name == ev.target);
    let e = &ev.entry;
    let per = if e.per > 0.0 { e.per } else { 1.0 };
    Payload {
        event: "threshold".into(),
        state: if ev.change == Change::Fired { "firing" } else { "resolved" }.into(),
        reason: match ev.change {
            Change::Fired => None,
            Change::Cleared => Some("cleared".into()),
            Change::Removed => Some("removed".into()),
        },
        target: ev.target.clone(),
        title: t.map(|t| view.redact(&plain(&t.title))).unwrap_or_default(),
        directory: t.map(|t| t.directory.trim_end_matches('/').to_string()).unwrap_or_default(),
        direction: e.direction.clone(),
        bound: e.bound.clone(),
        threshold: e.threshold.clone(),
        threshold_rate: e.threshold_rate / per,
        rate: e.rate / per,
        percent: e.percent,
        polls: e.polls,
        since: e.since,
        time: e.time,
        description: t.and_then(|t| t.thresholds.desc.as_deref()).map(|d| view.redact(d)),
        linkscope: env!("CARGO_PKG_VERSION").into(),
    }
}

/// The webhook action for an event, when `AlertWebhook` is set.
pub fn webhook(cfg: &Config, view: &linkscope_web::api::View, ev: &Event) -> Option<Action> {
    cfg.global.alert_webhook.as_ref()?;
    let body = serde_json::to_string(&payload(cfg, view, ev)).ok()?;
    Some(Action::Webhook { target: ev.target.clone(), body })
}

/// `X-LinkScope-Signature`: `sha256=` and the hex HMAC-SHA256, keyed with
/// `AlertWebhookSecret`, of the timestamp header, a full stop and the body.
pub fn sign(secret: &[u8], timestamp: i64, body: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let mut out = String::from("sha256=");
    for b in mac.finalize().into_bytes() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// A webhook receiver: posts each body with its signature, three attempts.
pub struct Hook {
    url: String,
    secret: Vec<u8>,
    client: reqwest::Client,
}

impl Hook {
    pub fn new(url: &str, secret: &str) -> anyhow::Result<Hook> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            // A redirect would carry the signed body somewhere unconfigured.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("LinkScope/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| anyhow::anyhow!("{}", e.without_url()))?;
        Ok(Hook { url: url.to_string(), secret: secret.as_bytes().to_vec(), client })
    }

    /// Post one body. Retries after 2 s and 10 s on a network error, a 5xx
    /// or a 429; any other answer is final. Errors never show the URL (it
    /// may carry a token).
    pub async fn send(&self, target: &str, body: String) {
        let mut last = String::new();
        for wait in [0u64, 2, 10] {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            let res = self
                .client
                .post(&self.url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header("X-LinkScope-Event", "threshold")
                .header("X-LinkScope-Timestamp", ts.to_string())
                .header("X-LinkScope-Signature", sign(&self.secret, ts, &body))
                .body(body.clone())
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => {
                    tracing::debug!("alert webhook for {target}: HTTP {}", r.status().as_u16());
                    return;
                }
                Ok(r) if !(r.status().is_server_error() || r.status().as_u16() == 429) => {
                    tracing::warn!("alert webhook for {target} refused: HTTP {}", r.status().as_u16());
                    return;
                }
                Ok(r) => last = format!("HTTP {}", r.status().as_u16()),
                Err(e) => last = e.without_url().to_string(),
            }
        }
        tracing::warn!("alert webhook for {target} failed after 3 attempts: {last}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &Path, body: &str) -> Config {
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let path = dir.join("m.cfg");
        std::fs::write(&path, format!("WorkDir: {}\n{body}", work.display())).unwrap();
        linkscope_config::load(
            &path,
            &linkscope_config::ReadOptions { trust: linkscope_config::TrustPolicy::Off },
            &linkscope_config::CheckOptions::default(),
        )
        .unwrap()
    }

    const T: &str = "Target[r1]: 1:pub-lic@10.0.0.1\nMaxBytes[r1]: 125000\nTitle[r1]: R1 <b>ether2</b>\nThreshMaxI[r1]: 30%\nThreshMinO[r1]: 100\n";

    #[test]
    fn fires_after_threshfor_polls_and_clears_once() {
        let d = tempfile::tempdir().unwrap();
        let c = cfg(d.path(), &format!("{T}ThreshFor[r1]: 2\n"));
        let t = &c.targets[0];
        let mut b = Book::default();
        let poll = |b: &mut Book, r: f64, time| b.observe(t, &crate::thresh::evaluate(t, Some(r), Some(500.0)), time);
        assert!(poll(&mut b, 52_118.0, 100).is_empty(), "one broken poll is not enough");
        assert_eq!(b.live(t)[0].polls, 1);
        assert!(!b.live(t)[0].firing);
        let ev = poll(&mut b, 52_118.0, 400);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].change, Change::Fired);
        assert_eq!((ev[0].entry.since, ev[0].entry.fired_at, ev[0].entry.polls), (100, Some(400), 2));
        assert!(poll(&mut b, 60_000.0, 700).is_empty(), "fires once");
        assert!(b.live(t)[0].firing);
        let ev = poll(&mut b, 10_000.0, 1000);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].change, Change::Cleared);
        assert!(b.live(t).is_empty());
        assert!(poll(&mut b, 10_000.0, 1300).is_empty());
    }

    #[test]
    fn a_short_burst_does_not_fire() {
        let d = tempfile::tempdir().unwrap();
        let c = cfg(d.path(), &format!("{T}ThreshFor[r1]: 3\n"));
        let t = &c.targets[0];
        let mut b = Book::default();
        for (i, r) in [52_118.0, 52_118.0, 1_000.0, 52_118.0, 52_118.0].iter().enumerate() {
            assert!(b.observe(t, &crate::thresh::evaluate(t, Some(*r), Some(500.0)), i as i64).is_empty());
        }
        assert_eq!(b.live(t)[0].polls, 2);
    }

    #[test]
    fn without_threshfor_one_poll_fires_and_missing_data_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let c = cfg(d.path(), T);
        let t = &c.targets[0];
        let mut b = Book::default();
        let ev = b.observe(t, &crate::thresh::evaluate(t, Some(52_118.0), Some(64.0)), 1);
        assert_eq!(ev.iter().map(|e| (e.entry.bound.as_str(), e.entry.direction.as_str())).collect::<Vec<_>>(), [("max", "in"), ("min", "out")]);
        // No rate for either direction this poll.
        assert!(b.observe(t, &crate::thresh::evaluate(t, None, None), 2).is_empty());
        assert_eq!(b.live(t).len(), 2);
    }

    #[test]
    fn survives_a_reload_and_reports_removed_bounds() {
        let d = tempfile::tempdir().unwrap();
        let c = cfg(d.path(), T);
        let t = &c.targets[0];
        let mut b = Book::default();
        b.observe(t, &crate::thresh::evaluate(t, Some(52_118.0), Some(500.0)), 1);
        let p = d.path().join("m.alerts");
        b.save(&p).unwrap();
        let mut again = Book::load(&p);
        assert!(again.prune(&c).is_empty());
        assert!(again.observe(t, &crate::thresh::evaluate(t, Some(52_118.0), Some(500.0)), 2).is_empty(), "already firing");
        let c2 = cfg(&d.path().join("b"), "Target[r1]: 1:pub-lic@10.0.0.1\nMaxBytes[r1]: 125000\nTitle[r1]: R1\nThreshMinO[r1]: 100\n");
        let ev = again.prune(&c2);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].change, Change::Removed);
        assert!(again.live(&c2.targets[0]).is_empty());
    }

    /// A receiver that answers 500 once, then 200: the body comes again,
    /// signed over its own timestamp.
    #[tokio::test]
    async fn webhook_retries_after_a_server_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for status in ["500 Internal Server Error", "200 OK"] {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Headers, then Content-Length bytes of body.
                let (head, body) = loop {
                    let n = s.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if let Some((h, b)) = text.split_once("\r\n\r\n") {
                        let len: usize = h.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap();
                        if b.len() >= len {
                            break (h.to_string(), b[..len].to_string());
                        }
                    }
                };
                s.write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await.unwrap();
                seen.push((head, body));
            }
            seen
        });
        let hook = Hook::new(&format!("http://{addr}/hook/tok-SECRET"), "0123456789abcdef").unwrap();
        hook.send("r1", "{\"state\":\"firing\"}".into()).await;
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 2);
        for (head, body) in &seen {
            let h = |name: &str| head.lines().find_map(|l| l.to_ascii_lowercase().starts_with(name).then(|| l[name.len()..].trim().to_string())).unwrap();
            assert!(head.starts_with("POST /hook/tok-SECRET HTTP/1.1"));
            assert_eq!(body, "{\"state\":\"firing\"}");
            let ts: i64 = h("x-linkscope-timestamp:").parse().unwrap();
            assert_eq!(h("x-linkscope-signature:"), sign(b"0123456789abcdef", ts, body));
            assert_eq!(h("content-type:"), "application/json");
        }
    }

    #[test]
    fn payload_and_signature() {
        let d = tempfile::tempdir().unwrap();
        let c = std::sync::Arc::new(cfg(d.path(), &format!("{T}ThreshDesc[r1]: uplink, community pub-lic\nOptions[r1]: perminute\n")));
        let t = &c.targets[0];
        let mut b = Book::default();
        let ev = b.observe(t, &crate::thresh::evaluate(t, Some(60.0 * 52_118.0), Some(60.0 * 500.0)), 1_700_000_000);
        let view = linkscope_web::api::view(&c);
        let p = payload(&c, &view, &ev[0]);
        assert_eq!((p.state.as_str(), p.reason, p.title.as_str()), ("firing", None, "R1 ether2"));
        assert_eq!((p.direction.as_str(), p.bound.as_str(), p.threshold.as_str()), ("in", "max", "30%"));
        // perminute: the log counts per minute, the webhook per second
        // (MaxBytes is per minute too: 30% of 125000 per minute).
        assert_eq!((p.rate, p.threshold_rate), (52_118.0, 625.0));
        assert_eq!(p.description.as_deref(), Some("uplink, community •••"));
        // Known answer, from Python's hmac: HMAC-SHA256("secret", "1700000000.{}").
        assert_eq!(sign(b"secret", 1_700_000_000, "{}"), "sha256=b8569b78799ff9e3cbff0fc2d63a33a2b57f3282abd07c37ae5e8e7d79a5f163");
    }
}
