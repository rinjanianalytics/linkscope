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
