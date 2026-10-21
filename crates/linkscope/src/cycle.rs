// SPDX-License-Identifier: GPL-2.0-or-later
//! One polling cycle: collect, evaluate, update logs, check thresholds.
//!
//! This is MRTG's main loop body (`readtargets`, then per target
//! `getcurrent`, `writegraphics` and `threshcheck`). The rate engine runs
//! in-process (MRTG forked `rateup` once per target). Logs are re-read from
//! disk each cycle, like rateup, so memory stays flat with target count.
//!
//! All file work runs on blocking threads (never on the async runtime that
//! also serves the web UI), each target is isolated so a panic marks only
//! that target failed, and threshold actions run in the background with
//! bounded concurrency so they cannot hold up polling.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bigdecimal::BigDecimal;
use linkscope_config::{Config, IfSel, SnmpConn, SnmpSource, Source, Target};
use linkscope_rate::{Mode, Period, RateLog, SummaryOpts, UpdateOutcome};
use linkscope_web::{CycleInfo, SharedState, TargetLive};
use tokio::sync::Semaphore;

use crate::alerts::{self, Book};
use crate::confcache::ConfCache;
use crate::oids::OidResolver;
use crate::poll::{Poller, SourceResult, now};
use crate::snmp::{Endpoint, SessionPool};
use crate::thresh;

/// Tunables that MRTG did not have.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// Agents polled concurrently.
    pub concurrency: usize,
    /// External commands run concurrently.
    pub exec_concurrency: usize,
    /// Timeout for one external command.
    pub exec_timeout: Duration,
    /// Worker threads for log updates (0 = number of CPUs).
    pub threads: usize,
    /// Longest time one agent may take per cycle (0 = 80% of Interval).
    pub agent_budget: Duration,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            concurrency: 256,
            exec_concurrency: 16,
            exec_timeout: Duration::from_secs(120),
            threads: 0,
            agent_budget: Duration::ZERO,
        }
    }
}

pub struct Engine {
    pub cfg: Arc<Config>,
    poller: Poller,
    pub state: SharedState,
    pub cache_path: PathBuf,
    opts: EngineOptions,
    /// Threshold actions still running from earlier cycles.
    actions: tokio::task::JoinSet<()>,
    /// LinkScope's alert state, kept in `<cache>.alerts`.
    alerts: Book,
    alerts_path: PathBuf,
    /// Alerts that ended with the last reload, still to be reported.
    removed: Vec<alerts::Event>,
}

/// Device uptime/name fetched through `RouterUptime[]` / `RouterName[]`.
#[derive(Default, Clone)]
struct Extra {
    uptime: Option<String>,
    name: Option<String>,
}

/// Directory holding a target's log. `Directory[]` is already normalized to
/// a relative path by the config checker, so this stays under LogDir.
pub fn log_dir(cfg: &Config, t: &Target) -> PathBuf {
    cfg.global.log_dir.join(&t.directory)
}

/// Refuse a `Directory[]` path whose components (below LogDir) are
/// symlinks: another account could otherwise redirect the writes.
fn check_no_symlinks(base: &Path, rel: &str) -> Result<(), String> {
    let mut p = base.to_path_buf();
    for c in Path::new(rel).components() {
        p.push(c);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symlink; refusing to write logs through it", p.display())),
            Ok(m) if !m.is_dir() => return Err(format!("{} is not a directory", p.display())),
            Ok(_) => {}
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Ok(())
}

/// The program name of an external command, for messages (the full
/// command line may contain credentials).
pub fn program_name(cmd: &str) -> String {
    let first = shell_words::split(cmd).ok().and_then(|v| v.into_iter().next()).unwrap_or_default();
    Path::new(&first).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "command".into())
}

struct Outcome {
    live: TargetLive,
    actions: Vec<thresh::Action>,
    /// This poll's threshold comparisons, for the alert state.
    evals: Vec<thresh::Eval>,
    /// MRTG's exit-code notion: both values were defined.
    defined: bool,
    /// The log could not be read or written.
    write_failed: bool,
}

impl Engine {
    pub fn new(cfg: Arc<Config>, oids: Arc<OidResolver>, state: SharedState, cache_path: PathBuf, opts: EngineOptions) -> Engine {
        let cache = Arc::new(Mutex::new(ConfCache::load(&cache_path)));
        let budget = if opts.agent_budget.is_zero() {
            Duration::from_secs((cfg.global.interval_secs * 4 / 5).max(30))
        } else {
            opts.agent_budget
        };
        let poller = Poller {
            cfg: cfg.clone(),
            oids,
            pool: SessionPool::default(),
            cache,
            concurrency: opts.concurrency,
            exec_concurrency: opts.exec_concurrency,
            exec_timeout: opts.exec_timeout,
            agent_budget: budget,
        };
        let alerts_path = Book::path_for(&cache_path);
        let mut alerts = Book::load(&alerts_path);
        let removed = alerts.prune(&cfg);
        {
            let mut s = state.write().unwrap_or_else(|e| e.into_inner());
            s.config = Some(cfg.clone());
            s.warnings = cfg.warnings.clone();
            for t in &cfg.targets {
                if let Some(l) = s.targets.get_mut(&t.name) {
                    l.alerts = alerts.live(t);
                }
            }
        }
        Engine { cfg, poller, state, cache_path, opts, actions: tokio::task::JoinSet::new(), alerts, alerts_path, removed }
    }

    /// Wait for threshold actions still running (call before exiting).
    pub async fn finish(&mut self) {
        while self.actions.join_next().await.is_some() {}
    }

    /// Run one complete cycle.
    pub async fn cycle(&mut self) -> CycleInfo {
        let started = now();
        let t0 = Instant::now();
        let results = Arc::new(self.poller.poll_all().await);
        let extras = Arc::new(self.router_extras().await);
        let poll_ms = t0.elapsed().as_millis();
        let cfg = self.cfg.clone();
        let threads = if self.opts.threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            self.opts.threads
        };

        // Log updates and the final sync run on blocking threads.
        let outcomes = {
            let (cfg, results, extras) = (cfg.clone(), results.clone(), extras.clone());
            tokio::task::spawn_blocking(move || {
                let n = cfg.targets.len();
                let chunk = n.div_ceil(threads.max(1)).max(1);
                let mut out: Vec<Outcome> = Vec::with_capacity(n);
                std::thread::scope(|scope| {
                    let handles: Vec<_> = (0..n)
                        .step_by(chunk)
                        .map(|start| {
                            let (cfg, results, extras) = (&cfg, &results, &extras);
                            scope.spawn(move || {
                                (start..(start + chunk).min(n))
                                    .map(|ti| {
                                        let t = &cfg.targets[ti];
                                        std::panic::catch_unwind(AssertUnwindSafe(|| process_target(cfg, t, results, &extras[ti])))
                                            .unwrap_or_else(|_| {
                                                tracing::error!("internal error while updating {}; target skipped this cycle", t.name);
                                                Outcome {
                                                    live: TargetLive { error: Some("internal error; see the daemon log".into()), failed: true, last_poll: now(), ..Default::default() },
                                                    actions: Vec::new(),
                                                    evals: Vec::new(),
                                                    defined: false,
                                                    write_failed: true,
                                                }
                                            })
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect();
                    for h in handles {
                        out.extend(h.join().unwrap_or_default());
                    }
                });
                // One flush for every log written this cycle.
                sync_filesystem(&cfg.global.log_dir);
                out
            })
            .await
            .unwrap_or_default()
        };

        let n = cfg.targets.len();
        let (mut ok, mut write_failures) = (0, 0);
        let mut actions = Vec::new();
        // LinkScope's alerts: which bounds fired or cleared this poll.
        let view = linkscope_web::api::view(&cfg);
        for ev in std::mem::take(&mut self.removed) {
            actions.extend(alerts::webhook(&cfg, &view, &ev));
        }
        let mut lives = Vec::with_capacity(n);
        for (t, mut o) in cfg.targets.iter().zip(outcomes) {
            ok += usize::from(o.defined);
            write_failures += usize::from(o.write_failed);
            actions.extend(o.actions);
            if t.thresholds.any() {
                for ev in self.alerts.observe(t, &o.evals, o.live.last_poll) {
                    tracing::info!(
                        "alert {} {} {} {}: {}",
                        t.name,
                        ev.entry.direction,
                        ev.entry.bound,
                        ev.entry.threshold,
                        if ev.change == alerts::Change::Fired { "firing" } else { "resolved" }
                    );
                    if t.thresholds.for_polls.is_some() {
                        if let Some(e) = &ev.eval {
                            actions.extend(thresh::transition(&cfg, t, e));
                        }
                    }
                    actions.extend(alerts::webhook(&cfg, &view, &ev));
                }
            }
            o.live.alerts = self.alerts.live(t);
            lives.push((t.name.clone(), o.live));
        }
        {
            let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
            st.targets.extend(lives);
        }
        if write_failures > 0 {
            tracing::error!("{write_failures} of {n} logs could not be read or written this cycle; check permissions of {}", cfg.global.log_dir.display());
        }
        tracing::debug!("cycle timing: polling {poll_ms} ms, log updates and sync {} ms", t0.elapsed().as_millis() - poll_ms);

        // Threshold programs and mail run in the background, bounded.
        while self.actions.try_join_next().is_some() {}
        if !actions.is_empty() {
            if self.actions.len() > 4 {
                tracing::warn!("threshold actions from earlier cycles are still running; skipping {} new actions", actions.len());
            } else {
                let cfg = cfg.clone();
                self.actions.spawn(async move { thresh::execute(&cfg, actions).await });
            }
        }

        let cache = self.poller.cache.clone();
        let path = self.cache_path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = cache.lock().unwrap_or_else(|e| e.into_inner()).save(&path) {
                tracing::warn!("writing {}: {e}", path.display());
            }
        })
        .await;
        if self.alerts.is_dirty() {
            let mut book = std::mem::take(&mut self.alerts);
            let path = self.alerts_path.clone();
            self.alerts = tokio::task::spawn_blocking(move || {
                if let Err(e) = book.save(&path) {
                    tracing::warn!("writing {}: {e}", path.display());
                }
                book
            })
            .await
            .unwrap_or_default();
        }
        let info = CycleInfo { started, duration_ms: t0.elapsed().as_millis() as u64, targets: n, ok, failed: n - ok, write_failures };
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        st.cycles += 1;
        st.last_cycle = Some(info.clone());
        info
    }

    /// Fetch uptime/name for targets with `RouterUptime[]` or `RouterName[]`
    /// (MRTG's second half of `getcurrent`). Each distinct location and OID
    /// set is fetched once per cycle, concurrently; a location that does not
    /// answer is not asked again in the same cycle.
    async fn router_extras(&self) -> Vec<Extra> {
        let mut out = vec![Extra::default(); self.cfg.targets.len()];
        // (location, oids) -> targets needing it, and what each answer fills.
        #[derive(Clone, Copy)]
        enum Fill {
            UptimeName,
            Name,
        }
        let mut wanted: HashMap<(String, Vec<String>), Vec<(usize, Fill)>> = HashMap::new();
        for (ti, t) in self.cfg.targets.iter().enumerate() {
            if t.router_uptime.is_none() && t.router_name.is_none() {
                continue;
            }
            let unique_loc = t.unique_source.and_then(|u| match &self.cfg.sources[u] {
                Source::Snmp(s) => Some(format!("{}@{}{}", s.community, s.host, s.snmp_info)),
                Source::Exec(_) => None,
            });
            let mut uploc: Option<String> = None;
            if let Some(ru) = &t.router_uptime {
                let (noidcomm, nloc) = ru.rsplit_once('@').unwrap_or((ru.as_str(), ""));
                let (noid, comm) = match noidcomm.split_once(':') {
                    Some((o, c)) => (o.to_string(), c.to_string()),
                    None => ("sysUptime".to_string(), noidcomm.to_string()),
                };
                // MRTG: with no location, use the unique target's host.
                let loc = if nloc.is_empty() {
                    unique_loc.as_ref().and_then(|u| u.split_once('@')).map(|(_, rest)| format!("{comm}@{rest}"))
                } else {
                    Some(format!("{comm}@{nloc}"))
                };
                if let Some(loc) = loc {
                    uploc = Some(loc.clone());
                    wanted.entry((loc, vec![noid, "sysName".into()])).or_default().push((ti, Fill::UptimeName));
                }
            }
            if let Some(rn) = &t.router_name {
                let (noid, nloc) = match rn.split_once(':') {
                    Some((o, l)) => (o.to_string(), Some(l.to_string())),
                    None => (rn.clone(), None),
                };
                let loc = nloc.filter(|l| !l.is_empty()).or_else(|| unique_loc.clone()).or_else(|| uploc.clone());
                if let Some(loc) = loc {
                    wanted.entry((loc, vec![noid])).or_default().push((ti, Fill::Name));
                }
            }
        }
        if wanted.is_empty() {
            return out;
        }
        let sem = Arc::new(Semaphore::new(32));
        let dead: Arc<Mutex<std::collections::HashSet<String>>> = Arc::default();
        let mut set = tokio::task::JoinSet::new();
        for ((loc, oids), who) in wanted {
            let ep = match self.location_endpoint(&loc, &self.cfg.targets[who[0].0]) {
                Some(ep) => ep,
                None => continue,
            };
            let numeric: Vec<String> = oids.iter().map(|o| self.poller.oids.resolve(o).unwrap_or_default()).collect();
            let (pool, sem, dead) = (self.poller.pool.clone(), sem.clone(), dead.clone());
            set.spawn(async move {
                let _p = sem.acquire_owned().await;
                let key = format!("{}:{}", ep.host, ep.port);
                if dead.lock().unwrap_or_else(|e| e.into_inner()).contains(&key) {
                    return (who, None);
                }
                let vals = match pool.session(&ep).await {
                    Ok(s) => s.lock().await.get(&numeric).await.ok(),
                    Err(_) => None,
                };
                if vals.is_none() {
                    dead.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
                }
                let vals = vals.map(|v| v.into_iter().map(|v| v.is_present().then(|| v.pretty().trim().to_string())).collect::<Vec<_>>());
                (who, vals)
            });
        }
        while let Some(r) = set.join_next().await {
            let Ok((who, Some(vals))) = r else { continue };
            for (ti, fill) in who {
                match fill {
                    Fill::UptimeName => {
                        out[ti].uptime = vals.first().cloned().flatten();
                        out[ti].name = vals.get(1).cloned().flatten();
                    }
                    Fill::Name => out[ti].name = vals.first().cloned().flatten(),
                }
            }
        }
        out
    }

    fn location_endpoint(&self, loc: &str, t: &Target) -> Option<Endpoint> {
        let (community, rest) = loc.rsplit_once('@')?;
        let (host, info) = if rest.starts_with('[') {
            let end = rest.find(']')? + 1;
            (rest[..end].to_string(), rest[end..].to_string())
        } else {
            match rest.split_once(':') {
                Some((h, i)) => (h.to_string(), format!(":{i}")),
                None => (rest.to_string(), String::new()),
            }
        };
        let conn = SnmpConn::parse(&info).ok()?;
        let src = SnmpSource {
            community: community.to_string(),
            host,
            snmp_info: info,
            conn,
            conversion: None,
            oid: [String::new(), String::new()],
            sel: [IfSel::None, IfSel::None],
            ipv4only: !self.cfg.global.enable_ipv6,
            snmp_options: t.snmp_options.clone(),
        };
        Endpoint::from_source(&src, self.cfg.global.single_request).ok()
    }
}

/// A value as the expression reads it. Bounded: a device or a command chooses the text, and a
/// value written with a huge exponent would cost gigabytes to align or print (MRTG drops it too).
fn dec(v: &Option<String>) -> Option<BigDecimal> {
    v.as_deref().and_then(linkscope_config::expr::bounded_decimal)
}

/// MRTG's compensation for agents that report counters as negative
/// numbers: add 2^31.
fn fix_negative(v: String) -> String {
    if v.starts_with('-') {
        if let Some(d) = linkscope_config::expr::bounded_decimal(&v) {
            let fixed = d + BigDecimal::from(1u64 << 31);
            return fixed.with_scale(0).to_string();
        }
    }
    v
}

fn process_target(cfg: &Config, t: &Target, results: &[SourceResult], extra: &Extra) -> Outcome {
    let mut live = TargetLive::default();
    let mut errors: Vec<String> = Vec::new();
    let get_in = |i: usize| results.get(i).and_then(|r| dec(&r.input));
    let get_out = |i: usize| results.get(i).and_then(|r| dec(&r.output));
    let mut used = Vec::new();
    t.expr.sources(&mut used);
    for i in &used {
        if let Some(e) = results.get(*i).and_then(|r| r.error.as_ref()) {
            if !errors.contains(e) {
                errors.push(e.clone());
            }
        }
    }
    // Errors name the target, never the Target text (it holds credentials).
    let eval = |get: &dyn Fn(usize) -> Option<BigDecimal>, dir: &str, errors: &mut Vec<String>| match t.expr.eval_value(get) {
        Ok(v) => Some(fix_negative(v)),
        Err(linkscope_config::EvalError::Undefined(_)) => None,
        Err(e) => {
            errors.push(format!("Target[{}][{dir}]: {e}", t.name));
            None
        }
    };
    let input = eval(&get_in, "_IN_", &mut errors);
    let output = eval(&get_out, "_OUT_", &mut errors);

    let (mut time, mut uptime, mut name) = (0, None, None);
    if let Some(u) = t.unique_source.and_then(|u| results.get(u)) {
        live.if_index = u.if_index[0].clone();
        time = u.time;
        uptime = u.uptime.clone();
        name = u.name.clone();
    }
    if extra.uptime.is_some() {
        uptime = extra.uptime.clone();
    }
    if extra.name.is_some() {
        name = extra.name.clone();
    }
    if time == 0 {
        time = now();
    }
    live.last_poll = time;
    live.in_raw = input.clone();
    live.out_raw = output.clone();
    live.uptime = uptime;
    live.sys_name = name;
    // Like MRTG, the exit code only reflects whether values were defined.
    let defined = input.is_some() && output.is_some();
    let fail = |mut live: TargetLive, errors: Vec<String>| {
        for e in &errors {
            tracing::warn!("{e}");
        }
        live.error = Some(errors.join("; "));
        live.failed = true;
        Outcome { live, actions: Vec::new(), evals: Vec::new(), defined, write_failed: true }
    };

    if !t.directory.is_empty() {
        if let Err(e) = check_no_symlinks(&cfg.global.log_dir, &t.directory) {
            errors.push(e);
            return fail(live, errors);
        }
    }
    let dir = log_dir(cfg, t);
    // MaxAge: refuse to update a log that has not been written for too long.
    if let Some(max_age) = cfg.global.max_age {
        let path = dir.join(format!("{}.log", t.name));
        if let Ok(m) = std::fs::symlink_metadata(&path).and_then(|m| m.modified()) {
            let age = std::time::SystemTime::now().duration_since(m).map(|d| d.as_secs()).unwrap_or(0);
            if age > max_age {
                errors.push(format!("skipping update of {}: its log is older than MaxAge ({max_age} s)", t.name));
                return fail(live, errors);
            }
        }
    }

    let mut log = match RateLog::load(&dir, &t.name, time) {
        Ok(l) => l,
        Err(e) => {
            errors.push(format!("{}: {e}", t.name));
            return fail(live, errors);
        }
    };
    let mode = Mode::from_letter(t.options.rate_letter()).unwrap_or(Mode::Counter);
    let abs_max = i64::try_from(t.abs_max).unwrap_or(i64::MAX);
    let mut actions = Vec::new();
    let mut evals = Vec::new();
    let mut write_failed = false;
    match log.update(time, input.as_deref().unwrap_or("-1"), output.as_deref().unwrap_or("-1"), abs_max, mode, t.options.unknaszero) {
        UpdateOutcome::Updated => {
            if let Some((_, i, o)) = log.latest_rate() {
                live.cur_in = Some(i);
                live.cur_out = Some(o);
            }
            if let Err(e) = log.save_unsynced(&dir, &t.name) {
                errors.push(format!("writing log for {}: {e}", t.name));
                write_failed = true;
            } else if t.thresholds.any() {
                let opts = SummaryOpts {
                    maxx: t.xsize as usize,
                    bits: t.options.bits,
                    peak: false,
                    withzeroes: t.options.withzeroes,
                    no_i: t.options.noi,
                    no_o: t.options.noo,
                    dorelpercent: t.options.dorelpercent,
                };
                let s = log.summarize(time, Period::Day, &opts);
                evals = thresh::evaluate(t, Some(s.cur_in as f64), Some(s.cur_out as f64));
                // ThreshFor[] targets act on alert changes instead (see alerts.rs).
                if t.thresholds.for_polls.is_none() {
                    actions = thresh::check_evals(cfg, t, &evals);
                }
            }
        }
        UpdateOutcome::TimeWarp => errors.push(format!("{}: log file time is newer than now; log unchanged (is the clock right?)", t.name)),
        UpdateOutcome::Rejected(e) => errors.push(format!("{}: value rejected: {e}", t.name)),
    }
    for e in &errors {
        tracing::warn!("{e}");
    }
    if !errors.is_empty() {
        live.error = Some(errors.join("; "));
    }
    live.failed = write_failed;
    Outcome { live, actions, evals, defined, write_failed }
}

/// Flush the filesystem holding `dir` (Linux `syncfs`; `sync` elsewhere).
fn sync_filesystem(dir: &Path) {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        if let Ok(d) = std::fs::File::open(dir) {
            // SAFETY: syncfs on a valid open descriptor.
            if unsafe { libc::syncfs(d.as_raw_fd()) } == 0 {
                return;
            }
        }
    }
    // SAFETY: sync takes no arguments and cannot fail.
    unsafe { libc::sync() };
}

/// Default `.ok` cache path: the config path with its extension replaced.
pub fn default_cache_path(cfg_path: &Path) -> PathBuf {
    cfg_path.with_extension("ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name_hides_arguments() {
        assert_eq!(program_name("/usr/bin/snmpget -c s3cret host oid"), "snmpget");
        assert_eq!(program_name("'/opt/my tools/x.sh' --pw=x"), "x.sh");
    }

    #[test]
    fn symlinked_directory_is_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert!(check_no_symlinks(d.path(), "real").is_ok());
        assert!(check_no_symlinks(d.path(), "link").is_err());
    }
}
