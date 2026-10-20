// SPDX-License-Identifier: GPL-2.0-or-later
//! `GET /metrics`: every target's rates, counters and status, and the last
//! polling cycle, in the Prometheus text format (version 0.0.4).
//!
//! Labels name the target, its `Directory[]`, the SNMP host and the
//! interface (the selector as written and the ifIndex it resolved to). No
//! community, SNMPv3 password or external command line is ever a label:
//! every label value also passes through the API's redaction.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use linkscope_config::{Config, IfSel, Source, Target};

use crate::api::{self, Status, View};
use crate::state::{LiveState, SharedState};

/// A label value escaped for the text format: backslash, quote and newline.
fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// A sample value: Prometheus spells the special floats its own way.
fn num(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0.0 {
            "+Inf".into()
        } else {
            "-Inf".into()
        }
    } else {
        format!("{v}")
    }
}

/// Seconds in the log's rate unit: rates are kept per minute with
/// `perminute` and per hour with `perhour`, per second otherwise.
fn unit_secs(t: &Target) -> f64 {
    if t.options.perhour {
        3600.0
    } else if t.options.perminute {
        60.0
    } else {
        1.0
    }
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Ok => "ok",
        Status::Warn => "warn",
        Status::Error => "error",
        Status::Stale => "stale",
        Status::Pending => "pending",
    }
}

/// The labels every per-target series carries, already escaped and redacted.
fn labels(cfg: &Config, view: &View, t: &Target, if_index: Option<&str>) -> String {
    let (kind, host, if_ref) = match t.unique_source.and_then(|u| cfg.sources.get(u)) {
        Some(Source::Snmp(s)) => {
            let sel = &s.sel[0];
            let r = match sel {
                IfSel::None => String::new(),
                sel => format!("{}:{}", sel.method(), sel.key()),
            };
            ("snmp", s.host.as_str(), r)
        }
        Some(Source::Exec(_)) => ("exec", "", String::new()),
        // Arithmetic over several sources: no single host or interface.
        None => ("expression", "", String::new()),
    };
    let r = |v: &str| esc(&view.redact(v));
    format!(
        "target=\"{}\",directory=\"{}\",source=\"{kind}\",host=\"{}\",interface=\"{}\",ifindex=\"{}\"",
        r(&t.name),
        r(t.directory.trim_end_matches('/')),
        r(host),
        r(&if_ref),
        r(if_index.unwrap_or("")),
    )
}

fn describe(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

/// The exposition for one configuration and its live state.
pub fn render(cfg: &Config, view: &View, st: &LiveState) -> String {
    let rows: Vec<_> = (0..cfg.targets.len())
        .map(|i| api::row(cfg, view, i, st))
        .collect();
    let lab: Vec<String> = cfg
        .targets
        .iter()
        .map(|t| {
            labels(
                cfg,
                view,
                t,
                st.targets.get(&t.name).and_then(|l| l.if_index.as_deref()),
            )
        })
        .collect();
    let mut out = String::new();

    describe(
        &mut out,
        "linkscope_info",
        "gauge",
        "LinkScope's version; always 1.",
    );
    let _ = writeln!(
        out,
        "linkscope_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    );
    describe(
        &mut out,
        "linkscope_start_time_seconds",
        "gauge",
        "When the daemon started, Unix time.",
    );
    let _ = writeln!(out, "linkscope_start_time_seconds {}", st.started);
    describe(
        &mut out,
        "linkscope_interval_seconds",
        "gauge",
        "Seconds between polling cycles (Interval).",
    );
    let _ = writeln!(
        out,
        "linkscope_interval_seconds {}",
        cfg.global.interval_secs
    );
    describe(
        &mut out,
        "linkscope_cycles_total",
        "counter",
        "Polling cycles completed since the daemon started.",
    );
    let _ = writeln!(out, "linkscope_cycles_total {}", st.cycles);

    if let Some(c) = &st.last_cycle {
        describe(
            &mut out,
            "linkscope_last_cycle_start_time_seconds",
            "gauge",
            "When the last cycle started, Unix time.",
        );
        let _ = writeln!(out, "linkscope_last_cycle_start_time_seconds {}", c.started);
        describe(
            &mut out,
            "linkscope_last_cycle_duration_seconds",
            "gauge",
            "How long the last cycle took.",
        );
        let _ = writeln!(
            out,
            "linkscope_last_cycle_duration_seconds {}",
            num(c.duration_ms as f64 / 1000.0)
        );
        describe(
            &mut out,
            "linkscope_last_cycle_targets",
            "gauge",
            "Targets in the last cycle, by outcome.",
        );
        let _ = writeln!(
            out,
            "linkscope_last_cycle_targets{{outcome=\"ok\"}} {}",
            c.ok
        );
        let _ = writeln!(
            out,
            "linkscope_last_cycle_targets{{outcome=\"failed\"}} {}",
            c.failed
        );
        describe(
            &mut out,
            "linkscope_last_cycle_log_write_failures",
            "gauge",
            "Logs the last cycle could not read or write.",
        );
        let _ = writeln!(
            out,
            "linkscope_last_cycle_log_write_failures {}",
            c.write_failures
        );
    }

    describe(
        &mut out,
        "linkscope_target_status",
        "gauge",
        "1 for the target's current status (ok, warn, error, stale, pending), 0 for the others.",
    );
    for (i, row) in rows.iter().enumerate() {
        for s in [
            Status::Ok,
            Status::Warn,
            Status::Error,
            Status::Stale,
            Status::Pending,
        ] {
            let _ = writeln!(
                out,
                "linkscope_target_status{{{},status=\"{}\"}} {}",
                lab[i],
                status_name(s),
                u8::from(row.status == s)
            );
        }
    }

    describe(
        &mut out,
        "linkscope_target_last_poll_time_seconds",
        "gauge",
        "When the target's last sample was taken, Unix time.",
    );
    for (i, row) in rows.iter().enumerate() {
        if let Some(t) = row.last_poll {
            let _ = writeln!(
                out,
                "linkscope_target_last_poll_time_seconds{{{}}} {t}",
                lab[i]
            );
        }
    }

    describe(
        &mut out,
        "linkscope_target_rate",
        "gauge",
        "The newest rate per second, as the UI shows it before formatting (bytes for interface counters; the bits option changes the display only).",
    );
    for (i, row) in rows.iter().enumerate() {
        let per = unit_secs(&cfg.targets[i]);
        for (dir, v) in [("in", row.cur_in), ("out", row.cur_out)] {
            if let Some(v) = v {
                let _ = writeln!(
                    out,
                    "linkscope_target_rate{{{},direction=\"{dir}\"}} {}",
                    lab[i],
                    num(v / per)
                );
            }
        }
    }

    describe(
        &mut out,
        "linkscope_target_max",
        "gauge",
        "MaxBytes for each direction, per second in the target's units.",
    );
    for (i, t) in cfg.targets.iter().enumerate() {
        let per = unit_secs(t);
        for (dir, v) in [("in", t.max_bytes1), ("out", t.max_bytes2)] {
            let _ = writeln!(
                out,
                "linkscope_target_max{{{},direction=\"{dir}\"}} {}",
                lab[i],
                num(v as f64 / per)
            );
        }
    }

    // What was read this poll: a counter (octets for an interface) unless the
    // target says the source is a gauge or an absolute value.
    let raw = |t: &Target| t.options.gauge || t.options.absolute;
    describe(
        &mut out,
        "linkscope_target_counter_total",
        "counter",
        "The counter as last read (octets for an interface target), so rate() and exact windows work.",
    );
    for (i, t) in cfg.targets.iter().enumerate() {
        if raw(t) {
            continue;
        }
        if let Some(l) = st.targets.get(&t.name) {
            for (dir, v) in [("in", &l.in_raw), ("out", &l.out_raw)] {
                if let Some(v) = v.as_deref().and_then(|v| v.parse::<f64>().ok()) {
                    let _ = writeln!(
                        out,
                        "linkscope_target_counter_total{{{},direction=\"{dir}\"}} {}",
                        lab[i],
                        num(v)
                    );
                }
            }
        }
    }
    describe(
        &mut out,
        "linkscope_target_value",
        "gauge",
        "The value as last read, for targets with the gauge or absolute option.",
    );
    for (i, t) in cfg.targets.iter().enumerate() {
        if !raw(t) {
            continue;
        }
        if let Some(l) = st.targets.get(&t.name) {
            for (dir, v) in [("in", &l.in_raw), ("out", &l.out_raw)] {
                if let Some(v) = v.as_deref().and_then(|v| v.parse::<f64>().ok()) {
                    let _ = writeln!(
                        out,
                        "linkscope_target_value{{{},direction=\"{dir}\"}} {}",
                        lab[i],
                        num(v)
                    );
                }
            }
        }
    }
    out
}

pub async fn metrics(State(state): State<SharedState>) -> Response {
    let st = state.read().unwrap_or_else(|e| e.into_inner());
    let Some(cfg) = st.config.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration not loaded yet",
        )
            .into_response();
    };
    let view = api::view(&cfg);
    let body = render(&cfg, &view, &st);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(esc(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(esc("x\ny"), "x\\ny");
    }

    #[test]
    fn special_floats() {
        assert_eq!(num(f64::NAN), "NaN");
        assert_eq!(num(f64::INFINITY), "+Inf");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(7453560794.0), "7453560794");
    }
}
