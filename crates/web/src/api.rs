// SPDX-License-Identifier: GPL-2.0-or-later
//! JSON API for the web UI.
//!
//! * `GET /api/summary`                      daemon status and totals
//! * `GET /api/targets`                      every target with its live status
//! * `GET /api/targets/{name}`               one target's details and display settings
//! * `GET /api/targets/{name}/series?period=day|week|month|year[&maxx=N]`
//!   graph columns and MRTG's Max/Average/Current figures for one period
//!
//! `GET /metrics` (Prometheus) is in [`crate::metrics`].
//!
//! Rates are in the log's unit (bytes per second, per minute with
//! `perminute`, per hour with `perhour`). `format` tells the UI how MRTG
//! would print them (`mul` is 8 for `bits`).

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use linkscope_config::{Config, Target};
use linkscope_rate::{Period, RateLog, SummaryOpts};
use serde::{Deserialize, Serialize};

use crate::state::SharedState;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// HTML from the config (PageTop, Title, Legends) is reduced to a safe
/// subset: structure and text formatting only, no scripts, styles, event
/// handlers or external resources.
pub fn sanitize(html: &str) -> String {
    use std::collections::HashSet;
    let tags: HashSet<&str> = [
        "h1", "h2", "h3", "h4", "div", "span", "p", "br", "b", "i", "em", "strong", "small", "code", "pre", "table", "thead", "tbody", "tr", "td", "th", "ul", "ol",
        "li", "a",
    ]
    .into_iter()
    .collect();
    ammonia::Builder::default()
        .tags(tags)
        .generic_attributes(HashSet::new())
        .tag_attributes(std::iter::once(("a", HashSet::from(["href"]))).collect())
        .url_schemes(HashSet::from(["http", "https"]))
        .link_rel(Some("noopener noreferrer nofollow"))
        .clean(html)
        .to_string()
}

/// MRTG's unit formatting rules for one target.
#[derive(Debug, Clone, Serialize)]
pub struct Format {
    /// 8 with `bits`, else 1.
    pub mul: f64,
    /// `Factor[]`.
    pub factor: f64,
    /// `Kilo[]` (1000 or 1024).
    pub kilo: f64,
    /// Unit labels by power of `kilo`, e.g. `["b/s","kb/s","Mb/s","Gb/s"]`.
    pub units: Vec<String>,
    /// `Options[]: integer`.
    pub integer: bool,
    /// `Options[]: nopercent`.
    pub nopercent: bool,
}

impl Format {
    pub fn for_target(t: &Target) -> Format {
        let o = &t.options;
        let per = if o.perminute {
            "/min"
        } else if o.perhour {
            "/h"
        } else {
            "/s"
        };
        let unit = if o.bits { "b" } else { "B" };
        let units: Vec<String> = match (&t.kmg, &t.short_legend) {
            (Some(kmg), Some(sl)) => kmg.iter().map(|k| format!("{k}{sl}")).collect(),
            (Some(kmg), None) => kmg.iter().map(|k| format!("{k}{unit}{per}")).collect(),
            (None, Some(sl)) if !sl.is_empty() => vec![sl.clone(), format!("k{sl}"), format!("M{sl}"), format!("G{sl}")],
            _ => ["", "k", "M", "G"].iter().map(|p| format!("{p}{unit}{per}")).collect(),
        };
        Format {
            mul: if o.bits { 8.0 } else { 1.0 },
            factor: t.factor,
            kilo: t.kilo as f64,
            units,
            integer: o.integer,
            nopercent: o.nopercent,
        }
    }

    /// MRTG's `fmi`: e.g. `"12.3 Mb/s (4.5%)"`.
    pub fn fmi(&self, number: f64, maxbytes: f64) -> String {
        let mut divnum = number * self.mul * self.factor;
        let mut divm = 0;
        while (divnum >= 10.0 * self.kilo || self.units.get(divm).is_some_and(|u| u.starts_with('-'))) && divm + 1 < self.units.len() {
            divm += 1;
            divnum /= self.kilo;
        }
        let unit = self.units.get(divm).map(String::as_str).unwrap_or("");
        let perc = if number == 0.0 || maxbytes == 0.0 { 0.0 } else { 100.0 / maxbytes * number };
        let num = if self.integer { format!("{divnum:.0}") } else { format!("{divnum:.1}") };
        if self.nopercent { format!("{num} {unit}") } else { format!("{num} {unit} ({perc:.1}%)") }
    }
}

/// Hide SNMP communities (credentials) in a Target string shown in the UI:
/// `2:secret@router` becomes `2:•••@router`.
pub fn mask_communities(target: &str) -> String {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?:\\[@ ]|[^\s@:`&])+@").unwrap());
    RE.replace_all(target, "•••@").into_owned()
}

/// Per-config data computed once per configuration load: sanitized HTML
/// and the secrets that must never leave the process.
pub struct View {
    secrets: Vec<String>,
    title_html: Vec<String>,
    legend_i: Vec<String>,
    legend_o: Vec<String>,
}

impl View {
    fn build(cfg: &Config) -> View {
        let mut secrets: Vec<String> = Vec::new();
        for src in &cfg.sources {
            match src {
                linkscope_config::Source::Snmp(s) => {
                    secrets.push(s.community.clone());
                    for k in ["authpassword", "privpassword", "authkey", "privkey"] {
                        if let Some(v) = s.snmp_options.get(k) {
                            secrets.push(v.clone());
                        }
                    }
                }
                linkscope_config::Source::Exec(e) => secrets.push(e.command.clone()),
            }
        }
        // The webhook URL may carry a token; the secret and password are keys.
        let g = &cfg.global;
        secrets.extend([&g.alert_webhook, &g.alert_webhook_secret, &g.alert_mail_password].into_iter().flatten().cloned());
        for t in &cfg.targets {
            for k in ["authpassword", "privpassword", "authkey", "privkey"] {
                if let Some(v) = t.snmp_options.get(k) {
                    secrets.push(v.clone());
                }
            }
        }
        // Longest first, and ignore trivially short strings that would
        // mangle ordinary text.
        secrets.retain(|s| s.chars().count() >= 3);
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        View {
            secrets,
            title_html: cfg.targets.iter().map(|t| sanitize(&t.title)).collect(),
            legend_i: cfg.targets.iter().map(|t| t.legend_i.as_deref().map(sanitize).unwrap_or_else(|| "In".into())).collect(),
            legend_o: cfg.targets.iter().map(|t| t.legend_o.as_deref().map(sanitize).unwrap_or_else(|| "Out".into())).collect(),
        }
    }

    /// Remove every configured credential and external command line from a
    /// string that is about to leave the process.
    pub fn redact(&self, text: &str) -> String {
        let mut out = mask_communities(text);
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), "•••");
            }
        }
        out
    }
}

/// The [`View`] for `cfg`, cached per configuration.
pub fn view(cfg: &std::sync::Arc<Config>) -> std::sync::Arc<View> {
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Option<(usize, Arc<View>)>> = Mutex::new(None);
    let key = Arc::as_ptr(cfg) as usize;
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, v)) = c.as_ref() {
        if *k == key {
            return v.clone();
        }
    }
    let v = Arc::new(View::build(cfg));
    *c = Some((key, v.clone()));
    v
}

#[derive(Serialize)]
pub struct Summary {
    pub version: &'static str,
    pub started: i64,
    pub now: i64,
    pub cycles: u64,
    pub last_cycle: Option<crate::state::CycleInfo>,
    pub interval_secs: u64,
    pub config_path: String,
    pub warnings: Vec<String>,
    pub targets_total: usize,
    pub targets_ok: usize,
    /// Warn + error + stale (kept for compatibility).
    pub targets_error: usize,
    pub targets_warn: usize,
    pub targets_failed: usize,
    pub targets_stale: usize,
    pub targets_pending: usize,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Polled successfully in the last cycle.
    Ok,
    /// One of in/out was unknown, or a threshold alert is firing.
    Warn,
    /// No data this cycle.
    Error,
    /// Last sample is older than two intervals.
    Stale,
    /// Not polled yet since start.
    Pending,
}

#[derive(Serialize)]
pub struct TargetRow {
    pub name: String,
    pub title_html: String,
    pub directory: String,
    pub status: Status,
    pub error: Option<String>,
    pub last_poll: Option<i64>,
    pub sys_name: Option<String>,
    pub uptime: Option<String>,
    pub max_bytes1: u64,
    pub max_bytes2: u64,
    pub format: Format,
    pub legend_i: String,
    pub legend_o: String,
    pub colours: Vec<Colour>,
    pub no_i: bool,
    pub no_o: bool,
    pub growright: bool,
    pub logscale: bool,
    /// Newest rates (log units) from the last update, without a series request.
    pub cur_in: Option<f64>,
    pub cur_out: Option<f64>,
    /// MRTG-formatted `cur_in` / `cur_out`.
    pub cur_in_text: Option<String>,
    pub cur_out_text: Option<String>,
    /// Threshold bounds broken now (firing, or waiting out `ThreshFor[]`).
    pub alerts: Vec<crate::state::AlertLive>,
}

#[derive(Serialize, Clone)]
pub struct Colour {
    pub name: String,
    pub rgb: String,
}

#[derive(Serialize)]
pub struct TargetDetail {
    #[serde(flatten)]
    pub row: TargetRow,
    pub target_text: String,
    pub page_top_html: String,
    pub page_foot_html: String,
    pub y_legend: String,
    pub legends: [String; 5],
    pub periods: Vec<&'static str>,
    pub with_peak: Vec<&'static str>,
    pub unscaled: Vec<&'static str>,
    pub growright: bool,
    pub logscale: bool,
    pub expscale: bool,
    pub secondmean: bool,
    pub dorelpercent: bool,
    pub avgpeak: bool,
    pub noinfo: bool,
    pub nolegend: bool,
    pub xsize: u32,
    pub ysize: u32,
    pub ytics: u32,
    pub yticsfactor: f64,
    pub abs_max: u64,
    pub timezone: Option<String>,
    pub thresholds: linkscope_config::Thresholds,
}

#[derive(Serialize)]
pub struct Point {
    pub t: i64,
    pub r#in: f64,
    pub out: f64,
    pub in_max: f64,
    pub out_max: f64,
}

#[derive(Serialize)]
pub struct Series {
    pub period: &'static str,
    pub step: i64,
    pub now: i64,
    /// Oldest first.
    pub points: Vec<Point>,
    pub peak: bool,
    /// Fixed y-axis maximum (bytes) when `Unscaled[]` covers this period.
    pub fixed_max: Option<f64>,
    /// MRTG's Max/Average/Current rows, raw and formatted.
    pub stats: Stats,
}

#[derive(Serialize)]
pub struct Stats {
    pub max_in: i64,
    pub max_out: i64,
    pub avg_in: i64,
    pub avg_out: i64,
    pub cur_in: i64,
    pub cur_out: i64,
    pub avg_max_in: i64,
    pub avg_max_out: i64,
    pub max_pct: Option<i64>,
    pub avg_pct: Option<i64>,
    pub cur_pct: Option<i64>,
    pub text: StatsText,
}

#[derive(Serialize)]
pub struct StatsText {
    pub max_in: String,
    pub max_out: String,
    pub avg_in: String,
    pub avg_out: String,
    pub cur_in: String,
    pub cur_out: String,
    pub avg_max_in: String,
    pub avg_max_out: String,
}

fn colours(t: &Target) -> Vec<Colour> {
    t.colours.iter().map(|(n, r)| Colour { name: n.clone(), rgb: r.clone() }).collect()
}

fn letters(s: &str) -> Vec<&'static str> {
    let s = s.to_lowercase();
    [("d", "day"), ("w", "week"), ("m", "month"), ("y", "year")].into_iter().filter(|(l, _)| s.contains(l)).map(|(_, n)| n).collect()
}

pub(crate) fn row(cfg: &Config, view: &View, ti: usize, st: &crate::state::LiveState) -> TargetRow {
    let t = &cfg.targets[ti];
    let live = st.targets.get(&t.name);
    let interval = cfg.global.interval_secs as i64;
    let status = match live {
        None => Status::Pending,
        Some(l) if l.failed => Status::Error,
        Some(l) if l.in_raw.is_none() && l.out_raw.is_none() => Status::Error,
        Some(l) if now() - l.last_poll > 2 * interval + 60 => Status::Stale,
        Some(l) if l.in_raw.is_none() || l.out_raw.is_none() => Status::Warn,
        Some(l) if l.alerts.iter().any(|a| a.firing) => Status::Warn,
        Some(_) => Status::Ok,
    };
    let fmt = Format::for_target(t);
    TargetRow {
        name: t.name.clone(),
        title_html: view.title_html[ti].clone(),
        directory: t.directory.trim_end_matches('/').to_string(),
        status,
        error: live.and_then(|l| l.error.as_deref()).map(|e| view.redact(e)),
        last_poll: live.map(|l| l.last_poll),
        sys_name: live.and_then(|l| l.sys_name.clone()),
        uptime: live.and_then(|l| l.uptime.clone()),
        max_bytes1: t.max_bytes1,
        max_bytes2: t.max_bytes2,
        format: fmt.clone(),
        legend_i: view.legend_i[ti].clone(),
        legend_o: view.legend_o[ti].clone(),
        colours: colours(t),
        no_i: t.options.noi,
        no_o: t.options.noo,
        growright: t.options.growright,
        logscale: t.options.logscale,
        cur_in: live.and_then(|l| l.cur_in),
        cur_out: live.and_then(|l| l.cur_out),
        cur_in_text: live.and_then(|l| l.cur_in).map(|v| fmt.fmi(v, t.max_bytes1 as f64)),
        cur_out_text: live.and_then(|l| l.cur_out).map(|v| fmt.fmi(v, t.max_bytes2 as f64)),
        alerts: live.map(|l| l.alerts.clone()).unwrap_or_default(),
    }
}

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

fn config(state: &SharedState) -> Result<std::sync::Arc<Config>, (StatusCode, String)> {
    state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .config
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "configuration not loaded yet".into()))
}

pub async fn summary(State(state): State<SharedState>) -> ApiResult<Summary> {
    let cfg = config(&state)?;
    let st = state.read().unwrap_or_else(|e| e.into_inner());
    let v = view(&cfg);
    let rows: Vec<Status> = (0..cfg.targets.len()).map(|i| row(&cfg, &v, i, &st).status).collect();
    Ok(Json(Summary {
        version: env!("CARGO_PKG_VERSION"),
        started: st.started,
        now: now(),
        cycles: st.cycles,
        last_cycle: st.last_cycle.clone(),
        interval_secs: cfg.global.interval_secs,
        config_path: cfg.path.display().to_string(),
        warnings: st.warnings.iter().map(|w| v.redact(w)).collect(),
        targets_total: rows.len(),
        targets_ok: rows.iter().filter(|s| **s == Status::Ok).count(),
        targets_error: rows.iter().filter(|s| matches!(s, Status::Error | Status::Stale | Status::Warn)).count(),
        targets_warn: rows.iter().filter(|s| **s == Status::Warn).count(),
        targets_failed: rows.iter().filter(|s| **s == Status::Error).count(),
        targets_stale: rows.iter().filter(|s| **s == Status::Stale).count(),
        targets_pending: rows.iter().filter(|s| **s == Status::Pending).count(),
    }))
}

pub async fn targets(State(state): State<SharedState>) -> ApiResult<Vec<TargetRow>> {
    let cfg = config(&state)?;
    let st = state.read().unwrap_or_else(|e| e.into_inner());
    let v = view(&cfg);
    Ok(Json((0..cfg.targets.len()).map(|i| row(&cfg, &v, i, &st)).collect()))
}

fn find<'a>(cfg: &'a Config, name: &str) -> Result<&'a Target, (StatusCode, String)> {
    cfg.targets.iter().find(|t| t.name == name).ok_or((StatusCode::NOT_FOUND, format!("no target named {name}")))
}

pub async fn target(State(state): State<SharedState>, Path(name): Path<String>) -> ApiResult<TargetDetail> {
    let cfg = config(&state)?;
    let t = find(&cfg, &name)?;
    let ti = cfg.targets.iter().position(|x| x.name == t.name).unwrap_or(0);
    let view = view(&cfg);
    let st = state.read().unwrap_or_else(|e| e.into_inner());
    let suppressed = letters(&t.suppress);
    Ok(Json(TargetDetail {
        row: row(&cfg, &view, ti, &st),
        target_text: view.redact(&t.target_text),
        page_top_html: t.page_top.as_deref().map(sanitize).unwrap_or_default(),
        page_foot_html: t.page_foot.as_deref().map(sanitize).unwrap_or_default(),
        y_legend: t.y_legend.clone().unwrap_or_default(),
        legends: t.legend.clone().map(|l| l.map(|s| sanitize(&s)).unwrap_or_default()),
        periods: ["day", "week", "month", "year"].into_iter().filter(|p| !suppressed.contains(p)).collect(),
        with_peak: letters(&t.with_peak),
        unscaled: letters(&t.unscaled),
        growright: t.options.growright,
        logscale: t.options.logscale,
        expscale: t.options.expscale,
        secondmean: t.options.secondmean,
        dorelpercent: t.options.dorelpercent,
        avgpeak: t.options.avgpeak,
        noinfo: t.options.noinfo,
        nolegend: t.options.nolegend,
        xsize: t.xsize,
        ysize: t.ysize,
        ytics: t.ytics,
        yticsfactor: t.yticsfactor,
        abs_max: t.abs_max,
        timezone: t.timezone.clone(),
        thresholds: t.thresholds.clone(),
    }))
}

#[derive(Deserialize)]
pub struct SeriesQuery {
    pub period: Option<String>,
    pub maxx: Option<usize>,
}

pub async fn series(State(state): State<SharedState>, Path(name): Path<String>, Query(q): Query<SeriesQuery>) -> ApiResult<Series> {
    let cfg = config(&state)?;
    let t = find(&cfg, &name)?.clone();
    let (period, pname) = match q.period.as_deref().unwrap_or("day") {
        "day" => (Period::Day, "day"),
        "week" => (Period::Week, "week"),
        "month" => (Period::Month, "month"),
        "year" => (Period::Year, "year"),
        other => return Err((StatusCode::BAD_REQUEST, format!("unknown period {other}"))),
    };
    let maxx = q.maxx.unwrap_or(t.xsize as usize).clamp(8, 2000);
    let dir: PathBuf = cfg.global.log_dir.join(&t.directory);
    let peak = letters(&t.with_peak).contains(&pname);
    let unscaled = letters(&t.unscaled).contains(&pname);
    let fmt = Format::for_target(&t);
    let res = tokio::task::spawn_blocking(move || {
        let path = dir.join(format!("{}.log", t.name));
        if !path.exists() {
            return Err((StatusCode::NOT_FOUND, "no data collected yet".to_string()));
        }
        let log = RateLog::load(&dir, &t.name, now()).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        // Graph time is the last update, like rateup drawing right after it.
        let at = log.last_time();
        let opts = SummaryOpts {
            maxx,
            bits: false,
            peak,
            withzeroes: t.options.withzeroes,
            no_i: t.options.noi,
            no_o: t.options.noo,
            dorelpercent: t.options.dorelpercent,
        };
        let s = log.summarize(at, period, &opts);
        let mut points: Vec<Point> = s.points.iter().map(|p| Point { t: p.time, r#in: p.r#in, out: p.out, in_max: p.in_max, out_max: p.out_max }).collect();
        points.reverse();
        let (m1, m2) = (t.max_bytes1 as f64, t.max_bytes2 as f64);
        let text = StatsText {
            max_in: fmt.fmi(s.max_in as f64, m1),
            max_out: fmt.fmi(s.max_out as f64, m2),
            avg_in: fmt.fmi(s.avg_in as f64, m1),
            avg_out: fmt.fmi(s.avg_out as f64, m2),
            cur_in: fmt.fmi(s.cur_in as f64, m1),
            cur_out: fmt.fmi(s.cur_out as f64, m2),
            avg_max_in: fmt.fmi(s.avg_max_in as f64, m1),
            avg_max_out: fmt.fmi(s.avg_max_out as f64, m2),
        };
        Ok(Series {
            period: pname,
            step: period.step(),
            now: at,
            points,
            peak,
            fixed_max: unscaled.then_some(m1.max(m2)),
            stats: Stats {
                max_in: s.max_in,
                max_out: s.max_out,
                avg_in: s.avg_in,
                avg_out: s.avg_out,
                cur_in: s.cur_in,
                cur_out: s.cur_out,
                avg_max_in: s.avg_max_in,
                avg_max_out: s.avg_max_out,
                max_pct: s.max_pct,
                avg_pct: s.avg_pct,
                cur_pct: s.cur_pct,
                text,
            },
        })
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(res))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn communities_are_masked() {
        assert_eq!(mask_communities("2:s3cret@r1:161::::2 + #Gi0/1:p\\@ss@r2"), "2:•••@r1:161::::2 + #Gi0/1:•••@r2");
        assert_eq!(mask_communities("ifInErrors.1&ifOutErrors.1:pub@h"), "ifInErrors.1&ifOutErrors.1:•••@h");
    }

    #[test]
    fn sanitizer_keeps_structure_drops_script() {
        let s = sanitize("<h1 onclick=x>Hi</h1><script>alert(1)</script><table><tr><td style='x'>a</td></tr></table><a href='javascript:x'>l</a><img src=x onerror=y>");
        assert!(s.contains("<h1>Hi</h1>"));
        assert!(!s.contains("script") && !s.contains("onclick") && !s.contains("style") && !s.contains("onerror") && !s.contains("javascript"));
        assert!(s.contains("<td>a</td>"));
    }
}
