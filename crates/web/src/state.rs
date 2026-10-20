// SPDX-License-Identifier: GPL-2.0-or-later
//! Live state shared between the poller and the web UI.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use linkscope_config::Config;
use serde::Serialize;

/// What the last poll produced for one target.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TargetLive {
    /// Unix time of the sample.
    pub last_poll: i64,
    /// Raw values handed to the rate engine (`None` = unknown).
    pub in_raw: Option<String>,
    pub out_raw: Option<String>,
    /// The ifIndex the target's interface resolved to (a single SNMP source
    /// with an interface selector), for matching with other tools.
    pub if_index: Option<String>,
    /// Device uptime and name (MRTG's "system has been up" line).
    pub uptime: Option<String>,
    pub sys_name: Option<String>,
    /// Why data was missing, if it was.
    pub error: Option<String>,
    /// Newest rates in the log after this update (log units).
    pub cur_in: Option<f64>,
    pub cur_out: Option<f64>,
    /// The log could not be read or written (data is not being recorded).
    pub failed: bool,
    /// Threshold bounds broken now: firing, or waiting out `ThreshFor[]`.
    pub alerts: Vec<AlertLive>,
}

/// One broken threshold bound of a target.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AlertLive {
    /// `in` or `out`.
    pub direction: String,
    /// `min` or `max`.
    pub bound: String,
    /// The bound as configured, e.g. `30%`.
    pub threshold: String,
    /// The bound and the day graph's current rate, in the log's units (like
    /// `cur_in`); `percent` is the rate in percent of `MaxBytes` for a
    /// percent bound.
    pub threshold_rate: f64,
    pub rate: f64,
    pub percent: Option<f64>,
    /// Broken for `polls` polls in a row; it fires at `polls_needed`.
    pub firing: bool,
    pub polls: u32,
    pub polls_needed: u32,
    /// Unix time of the first broken poll, and of the poll that fired it.
    pub since: i64,
    pub fired_at: Option<i64>,
}

/// Summary of one polling cycle.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CycleInfo {
    pub started: i64,
    pub duration_ms: u64,
    pub targets: usize,
    pub ok: usize,
    pub failed: usize,
    /// Logs that could not be read or written.
    pub write_failures: usize,
}

/// Everything the UI shows that is not on disk.
#[derive(Debug, Default)]
pub struct LiveState {
    pub started: i64,
    pub cycles: u64,
    pub last_cycle: Option<CycleInfo>,
    pub targets: HashMap<String, TargetLive>,
    pub config: Option<Arc<Config>>,
    pub warnings: Vec<String>,
}

pub type SharedState = Arc<RwLock<LiveState>>;
