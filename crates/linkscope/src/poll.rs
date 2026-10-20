// SPDX-License-Identifier: GPL-2.0-or-later
//! Data collection: a port of MRTG's `readtargets`, `getsnmparg`,
//! `populateconfcache` and `getexternal`.
//!
//! Differences from MRTG, all aimed at speed without changing results:
//! * sources are grouped per agent and all their OIDs (data, interface
//!   cross-checks, sysUptime, sysName) go out in shared, batched PDUs;
//! * agents are polled concurrently (bounded), instead of sequentially or in
//!   a handful of forked processes;
//! * an agent's interface tables are walked at most once per cycle.
//!
//! External commands run without a shell unless `ExecShell: yes`, with a
//! timeout, and with their output size bounded.

use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use linkscope_config::{Config, IfSel, SnmpSource, Source};
use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

use crate::confcache::{ConfCache, mac_string, trim_ctl};
use crate::oids::OidResolver;
use crate::snmp::{Endpoint, SessionPool, SnmpError, Val};

/// What one data source produced this cycle (MRTG's `_IN_`, `_OUT_`,
/// `_TIME_`, `_UPTIME_`, `_NAME_`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceResult {
    pub input: Option<String>,
    pub output: Option<String>,
    pub time: i64,
    pub uptime: Option<String>,
    pub name: Option<String>,
    /// Why values are missing, for the log and the UI.
    pub error: Option<String>,
    /// The ifIndex each direction resolved to (none without an interface).
    pub if_index: [Option<String>; 2],
}

/// Shared state for polling.
pub struct Poller {
    pub cfg: Arc<Config>,
    pub oids: Arc<OidResolver>,
    pub pool: SessionPool,
    pub cache: Arc<Mutex<ConfCache>>,
    /// Maximum agents polled at the same time.
    pub concurrency: usize,
    /// Maximum external commands running at the same time.
    pub exec_concurrency: usize,
    pub exec_timeout: Duration,
    /// Longest time one agent (all its requests and walks) may take per cycle.
    pub agent_budget: Duration,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// MRTG's final sanity check on in/out values (note the unescaped `.`,
/// kept for compatibility).
fn numeric_ok(v: &str) -> bool {
    static R: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-+]?\d+(.\d+)?$").unwrap());
    // Counters have at most 20 digits; longer values only cost CPU and
    // memory in arbitrary-precision arithmetic.
    v.len() <= 64 && R.is_match(v)
}

/// `s/\n|\r//g; s/^\s+//; s/\s+$//`.
fn clean(v: &str) -> String {
    v.replace(['\n', '\r'], "").trim().to_string()
}

impl Poller {
    /// Poll every source referenced by the configuration.
    pub async fn poll_all(&self) -> Vec<SourceResult> {
        let n = self.cfg.sources.len();
        let mut results = vec![SourceResult::default(); n];
        let snmp_sem = Arc::new(Semaphore::new(self.concurrency.max(1)));
        let exec_sem = Arc::new(Semaphore::new(self.exec_concurrency.max(1)));

        // Group SNMP sources by agent session parameters.
        let mut groups: HashMap<String, (Endpoint, Vec<(usize, SnmpSource)>)> = HashMap::new();
        let mut set = tokio::task::JoinSet::new();
        for (i, src) in self.cfg.sources.iter().enumerate() {
            match src {
                Source::Snmp(s) => match Endpoint::from_source(s, self.cfg.global.single_request) {
                    Ok(ep) => {
                        let key = format!("{}@{}{}|{}|{:?}", s.community, s.host, s.snmp_info, s.ipv4only, s.snmp_options);
                        groups.entry(key).or_insert_with(|| (ep, Vec::new())).1.push((i, s.clone()));
                    }
                    Err(e) => results[i].error = Some(e),
                },
                Source::Exec(e) => {
                    let sem = exec_sem.clone();
                    let cmd = e.command.clone();
                    let shell = self.cfg.global.exec_shell;
                    let path_add = self.cfg.global.path_add.clone();
                    let timeout = self.exec_timeout;
                    set.spawn(async move {
                        let _p = sem.acquire_owned().await;
                        vec![(i, run_external(&cmd, shell, path_add.as_deref(), timeout).await)]
                    });
                }
            }
        }
        for (_, (ep, members)) in groups {
            let sem = snmp_sem.clone();
            let ctx = GroupCtx {
                pool: self.pool.clone(),
                oids: self.oids.clone(),
                cache: self.cache.clone(),
                no_mib2: self.cfg.global.no_mib2,
            };
            let budget = self.agent_budget;
            set.spawn(async move {
                let _p = sem.acquire_owned().await;
                let ids: Vec<usize> = members.iter().map(|(i, _)| *i).collect();
                let who = ep.display();
                match tokio::time::timeout(budget, ctx.poll_group(ep, members)).await {
                    Ok(r) => r,
                    Err(_) => {
                        tracing::warn!("{who} did not finish within {} s; its targets are unknown this cycle", budget.as_secs());
                        let msg = format!("{who} took longer than {} s (skipped this cycle)", budget.as_secs());
                        ids.into_iter().map(|i| (i, SourceResult { error: Some(msg.clone()), time: now(), ..Default::default() })).collect()
                    }
                }
            });
        }
        while let Some(r) = set.join_next().await {
            match r {
                Ok(v) => {
                    for (i, res) in v {
                        results[i] = res;
                    }
                }
                Err(e) => tracing::error!("polling task failed: {e}"),
            }
        }
        results
    }
}

#[derive(Clone)]
struct GroupCtx {
    pool: SessionPool,
    oids: Arc<OidResolver>,
    cache: Arc<Mutex<ConfCache>>,
    no_mib2: bool,
}

/// One planned request for a source.
struct Plan {
    idx: usize,
    src: SnmpSource,
    /// ".N" or "" per direction, or an error.
    ifnum: [String; 2],
    /// For each direction: optional cross-check OID, then the data OID.
    xcheck: [Option<String>; 2],
    data: [String; 2],
}

enum Prep {
    Ready(Plan),
    Failed(usize, String),
}

fn host_key(s: &SnmpSource) -> String {
    format!("{}@{}{}", s.community, s.host, s.snmp_info)
}

impl GroupCtx {
    async fn poll_group(&self, ep: Endpoint, members: Vec<(usize, SnmpSource)>) -> Vec<(usize, SourceResult)> {
        let fail = |members: &[(usize, SnmpSource)], msg: String| -> Vec<(usize, SourceResult)> {
            members.iter().map(|(i, _)| (*i, SourceResult { error: Some(msg.clone()), time: now(), ..Default::default() })).collect()
        };
        let session = match self.pool.session(&ep).await {
            Ok(s) => s,
            Err(e) => return fail(&members, e.to_string()),
        };
        let mut sess = session.lock().await;
        let hk = host_key(&members[0].1);
        let mut populated = false;
        let mut retried: HashSet<usize> = HashSet::new();
        let mut out: Vec<(usize, SourceResult)> = Vec::new();
        let mut todo: Vec<(usize, SnmpSource)> = members;

        // At most two rounds: the second re-resolves interfaces whose
        // cross-check failed (MRTG's `goto RETRY`).
        for round in 0..2 {
            if todo.is_empty() {
                break;
            }
            // Resolve interface numbers, scanning the agent's tables once if needed.
            let needs_scan = todo.iter().any(|(_, s)| {
                (0..2).any(|d| !matches!(s.sel[d], IfSel::If(_) | IfSel::None) && self.lookup(&hk, &s.sel[d]).is_none())
            });
            if needs_scan && !populated {
                populated = true;
                if let Err(e) = self.populate(&mut sess, &hk).await {
                    tracing::warn!("interface scan of {} failed: {e}", ep.display());
                }
            }
            let mut plans = Vec::new();
            for (i, s) in &todo {
                match self.plan(*i, s, &hk) {
                    Prep::Ready(p) => plans.push(p),
                    Prep::Failed(i, msg) => out.push((i, SourceResult { error: Some(msg), time: now(), ..Default::default() })),
                }
            }
            if plans.is_empty() {
                break;
            }
            // Collect the OIDs for one batched request.
            let want_sys = !self.no_mib2;
            let mut oids: Vec<String> = Vec::new();
            let mut index: HashMap<String, usize> = HashMap::new();
            let mut slot = |spec: &str, oids: &mut Vec<String>| -> Option<usize> {
                let n = self.oids.resolve(spec)?;
                Some(*index.entry(n.clone()).or_insert_with(|| {
                    oids.push(n);
                    oids.len() - 1
                }))
            };
            struct Slots {
                x: [Option<Option<usize>>; 2],
                d: [Option<usize>; 2],
                pseudo: [Option<Pseudo>; 2],
            }
            let mut slots = Vec::with_capacity(plans.len());
            for p in &plans {
                let mut s = Slots { x: [None, None], d: [None, None], pseudo: [None, None] };
                for d in 0..2 {
                    if let Some(x) = &p.xcheck[d] {
                        s.x[d] = Some(slot(x, &mut oids));
                    }
                    match Pseudo::parse(&p.data[d]) {
                        Some(ps) => s.pseudo[d] = Some(ps),
                        None => s.d[d] = slot(&p.data[d], &mut oids),
                    }
                }
                slots.push(s);
            }
            let (up_slot, name_slot) = if want_sys {
                (slot("sysUptime", &mut oids), slot("sysName", &mut oids))
            } else {
                (None, None)
            };
            let vals = match sess.get(&oids).await {
                Ok(v) => v,
                Err(e) => {
                    let msg = match &e {
                        SnmpError::Timeout(_) => format!("no response from {} (skipping this cycle)", ep.display()),
                        other => other.to_string(),
                    };
                    for p in &plans {
                        out.push((p.idx, SourceResult { error: Some(msg.clone()), time: now(), ..Default::default() }));
                    }
                    return out;
                }
            };
            let time = now();
            let get = |s: Option<usize>| s.and_then(|i| vals.get(i)).filter(|v| v.is_present());
            let uptime = get(up_slot).map(|v| clean(&v.pretty()));
            let name = get(name_slot).map(|v| clean(&v.pretty()));

            let mut next_todo = Vec::new();
            for (p, s) in plans.into_iter().zip(slots) {
                // Cross-check the cached interface mapping.
                let mut mismatch = None;
                for d in 0..2 {
                    let Some(xs) = s.x[d] else { continue };
                    let got = xs.and_then(|i| vals.get(i));
                    if !crosscheck_ok(&p.src.sel[d], &p.ifnum[d], got) {
                        mismatch = Some(d);
                        break;
                    }
                }
                if let Some(d) = mismatch {
                    if round == 0 && retried.insert(p.idx) {
                        // Forget the stale mapping and rescan once.
                        self.cache.lock().unwrap_or_else(|e| e.into_inner()).store(&hk, p.src.sel[d].method(), p.src.sel[d].key(), None);
                        populated = false;
                        next_todo.push((p.idx, p.src));
                    } else {
                        out.push((p.idx, SourceResult {
                            error: Some(format!("could not match {} '{}' on {}", p.src.sel[d].method(), p.src.sel[d].key(), ep.display())),
                            time,
                            ..Default::default()
                        }));
                    }
                    continue;
                }
                let mut io: [Option<String>; 2] = [None, None];
                for d in 0..2 {
                    let raw = if let Some(ps) = &s.pseudo[d] {
                        ps.eval(&mut sess, &self.oids, &p.ifnum[d]).await
                    } else {
                        get(s.d[d]).map(|v| data_value(v, &p.data[d]))
                    };
                    io[d] = raw.map(|v| clean(&v)).filter(|v| numeric_ok(v));
                }
                let error = if io[0].is_none() || io[1].is_none() {
                    Some(format!("no value for {} or {} on {}", p.data[0], p.data[1], ep.display()))
                } else {
                    None
                };
                let [input, output] = io;
                let if_index = [0, 1].map(|d| p.ifnum[d].strip_prefix('.').map(str::to_string));
                out.push((p.idx, SourceResult { input, output, time, uptime: uptime.clone(), name: name.clone(), error, if_index }));
            }
            todo = next_todo;
        }
        out
    }

    fn lookup(&self, hk: &str, sel: &IfSel) -> Option<String> {
        let c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        c.read(hk, sel.method(), trim_ctl(sel.key())).map(str::to_string)
    }

    fn plan(&self, idx: usize, s: &SnmpSource, hk: &str) -> Prep {
        let mut ifnum = [String::new(), String::new()];
        for d in 0..2 {
            ifnum[d] = match &s.sel[d] {
                IfSel::If(k) => format!(".{k}"),
                IfSel::None => String::new(),
                sel => match self.lookup(hk, sel) {
                    Some(v) => format!(".{v}"),
                    None => {
                        return Prep::Failed(idx, format!("could not match host '{}' ref '{}' key '{}'", s.host, sel.method(), sel.key()));
                    }
                },
            };
            let ok = ifnum[d].is_empty() || (ifnum[d].len() > 1 && ifnum[d][1..].bytes().all(|b| b.is_ascii_digit()));
            if !ok {
                return Prep::Failed(idx, format!("cannot determine ifIndex for '{}' (cache says {})", s.sel[d].key(), &ifnum[d][1..]));
            }
        }
        let xcheck = [0, 1].map(|d| match &s.sel[d] {
            IfSel::Eth(_) => Some(format!("ifPhysAddress{}", ifnum[d])),
            IfSel::Ip(k) => Some(format!("ipAdEntIfIndex.{k}")),
            IfSel::Descr(_) => Some(format!("ifDescr{}", ifnum[d])),
            IfSel::Type(_) => Some(format!("ifType{}", ifnum[d])),
            IfSel::Name(_) => Some(format!("ifName{}", ifnum[d])),
            _ => None,
        });
        let data = [0, 1].map(|d| format!("{}{}", s.oid[d], ifnum[d]));
        Prep::Ready(Plan { idx, src: s.clone(), ifnum, xcheck, data })
    }

    /// `populateconfcache`: walk ifName, ifDescr, ifType, ipAdEntIfIndex and
    /// ifPhysAddress and record which ifIndex each value maps to.
    async fn populate(&self, sess: &mut crate::snmp::Session, hk: &str) -> Result<(), SnmpError> {
        let who = sess.endpoint().display();
        if self.cache.lock().unwrap_or_else(|e| e.into_inner()).is_dead(hk) {
            return Err(SnmpError::Timeout(who));
        }
        let mut walked: Vec<(&str, Vec<(String, Val)>)> = Vec::new();
        for (node, method) in [("ifName", "Name"), ("ifDescr", "Descr"), ("ifType", "Type"), ("ipAdEntIfIndex", "Ip")] {
            let base = self.oids.resolve(node).expect("built-in OID");
            match sess.walk(&base).await {
                Ok(rows) => walked.push((method, rows)),
                Err(SnmpError::Timeout(t)) => {
                    self.cache.lock().unwrap_or_else(|e| e.into_inner()).mark_dead(hk);
                    return Err(SnmpError::Timeout(t));
                }
                Err(e) => tracing::debug!("skipping {node} scan on {who}: {e}"),
            }
        }
        let eth = sess.walk(&self.oids.resolve("ifPhysAddress").expect("built-in OID")).await.unwrap_or_default();
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        c.clear_host(hk);
        for (method, rows) in walked {
            for (suffix, v) in rows.into_iter().take(MAX_CACHE_ROWS) {
                if method == "Ip" {
                    c.store(hk, "Ip", &suffix, Some(&v.pretty()));
                } else {
                    let desc: String = trim_ctl(&v.pretty()).chars().take(MAX_KEY_CHARS).map(|ch| if ch <= ' ' { ' ' } else { ch }).collect();
                    c.store(hk, method, &desc, Some(&suffix));
                }
            }
        }
        for (suffix, v) in eth.into_iter().take(MAX_CACHE_ROWS) {
            let bytes = match &v {
                Val::Str(b) | Val::Opaque(b) => b.clone(),
                _ => continue,
            };
            c.store(hk, "Eth", &mac_string(&bytes), Some(&suffix));
        }
        Ok(())
    }
}

/// Interface-table rows kept per host and walk (a router has far fewer).
const MAX_CACHE_ROWS: usize = 100_000;
/// Longest ifDescr/ifName kept as a cache key.
const MAX_KEY_CHARS: usize = 256;

/// Data value as MRTG sees it: `if(Admin|Oper)Hack` is 1 when up else 0;
/// TimeTicks are given as plain ticks (MRTG printed a duration here, which
/// then failed its own numeric check).
fn data_value(v: &Val, oid_spec: &str) -> String {
    if oid_spec.starts_with("ifAdminHack") || oid_spec.starts_with("ifOperHack") {
        // MRTG compares numerically (`== 1`).
        return if v.number().and_then(|n| n.parse::<f64>().ok()) == Some(1.0) { "1".into() } else { "0".into() };
    }
    match v {
        Val::TimeTicks(t) => t.to_string(),
        other => other.pretty(),
    }
}

/// MRTG's reality check of a cached interface mapping.
fn crosscheck_ok(sel: &IfSel, ifnum: &str, got: Option<&Val>) -> bool {
    match sel {
        IfSel::Eth(k) => match got {
            Some(Val::Str(b)) | Some(Val::Opaque(b)) => mac_string(b) == *k,
            _ => false,
        },
        IfSel::Ip(_) => got.filter(|v| v.is_present()).is_some_and(|v| format!(".{}", v.pretty()) == ifnum),
        IfSel::Descr(k) | IfSel::Name(k) | IfSel::Type(k) => {
            let s = got.filter(|v| v.is_present()).map(|v| v.pretty()).unwrap_or_else(|| "Empty".into());
            trim_ctl(&s) == k.as_str()
        }
        _ => true,
    }
}

/// MRTG's pseudo OIDs, usable in complex targets.
#[derive(Debug, Clone)]
enum Pseudo {
    Zero,
    One,
    /// `WaLK<n><oid>`: the n-th value of a walk.
    Walk(usize, String),
    /// `GeTNEXT<oid>`
    GetNext(String),
    /// `CnTWaLK<oid>`: number of rows in a walk.
    CountWalk(String),
    /// `IndexPOS` placeholder replaced by the interface number.
    IndexPos(String),
}

impl Pseudo {
    fn parse(spec: &str) -> Option<Pseudo> {
        static WALK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^WaLK(\d*)(.*)$").unwrap());
        static IDX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"IndexPOS.*\.(\d*)").unwrap());
        if spec.starts_with("PseudoZero") {
            return Some(Pseudo::Zero);
        }
        if spec.starts_with("PseudoOne") {
            return Some(Pseudo::One);
        }
        if let Some(c) = WALK.captures(spec) {
            return Some(Pseudo::Walk(c[1].parse().unwrap_or(0), c[2].to_string()));
        }
        if let Some(r) = spec.strip_prefix("GeTNEXT") {
            return Some(Pseudo::GetNext(r.to_string()));
        }
        if let Some(r) = spec.strip_prefix("CnTWaLK") {
            return Some(Pseudo::CountWalk(r.to_string()));
        }
        if let Some(c) = IDX.captures(spec) {
            let idx = c[1].to_string();
            let mut s = spec.replacen("IndexPOS", &idx, 1);
            if let Some(p) = s.rfind('.') {
                if s[p + 1..].bytes().all(|b| b.is_ascii_digit()) {
                    s.truncate(p);
                }
            }
            return Some(Pseudo::IndexPos(s));
        }
        None
    }

    async fn eval(&self, sess: &mut crate::snmp::Session, oids: &OidResolver, _ifnum: &str) -> Option<String> {
        match self {
            Pseudo::Zero => Some("0".into()),
            Pseudo::One => Some("1".into()),
            Pseudo::Walk(n, o) => {
                let rows = sess.walk(&oids.resolve(o)?).await.ok()?;
                rows.get(*n).map(|(_, v)| v.pretty())
            }
            Pseudo::GetNext(o) => sess.get_next(&oids.resolve(o)?).await.ok()?.map(|(_, v)| v.pretty()),
            Pseudo::CountWalk(o) => Some(sess.walk(&oids.resolve(o)?).await.map(|r| r.len()).unwrap_or(0).to_string()),
            Pseudo::IndexPos(o) => {
                let v = sess.get(&[oids.resolve(o)?]).await.ok()?;
                v.into_iter().next().filter(|v| v.is_present()).map(|v| v.pretty())
            }
        }
    }
}

/// Maximum bytes read from an external command (only 4 lines are used).
const EXEC_MAX_OUTPUT: usize = 64 * 1024;

/// `getexternal`: run a command and read in, out, uptime and name lines.
///
/// Like MRTG, only the first four lines matter: once they are read (or the
/// output cap is reached) the command's process group is terminated. The
/// command runs in its own process group, so a timeout also stops anything
/// it started. Messages name the program only, never its arguments, which
/// may contain credentials.
pub async fn run_external(cmd: &str, allow_shell: bool, path_add: Option<&str>, timeout: Duration) -> SourceResult {
    let prog = crate::cycle::program_name(cmd);
    let mut res = SourceResult { time: now(), uptime: Some("unknown".into()), name: Some("unknown".into()), ..Default::default() };
    let mut command = if linkscope_config::check::needs_shell(cmd) {
        if !allow_shell {
            res.error = Some(format!("external command ({prog}) needs a shell; set ExecShell: yes or wrap it in a script"));
            return res;
        }
        let mut c = tokio::process::Command::new("/bin/sh");
        c.arg("-c").arg(cmd);
        c
    } else {
        let argv = match shell_words::split(cmd) {
            Ok(a) if !a.is_empty() => a,
            _ => {
                res.error = Some(format!("cannot parse external command ({prog})"));
                return res;
            }
        };
        let mut c = tokio::process::Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    };
    if let Some(p) = path_add {
        let path = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{path}", p.trim_end_matches('/')));
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true).process_group(0);
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            res.error = Some(format!("running {prog}: {e}"));
            return res;
        }
    };
    let pgid = child.id().map(|p| p as i32);
    let kill_group = || {
        if let Some(pg) = pgid {
            // SAFETY: kill(2) with a negative pid signals our child's process group.
            unsafe { libc::kill(-pg, libc::SIGKILL) };
        }
    };
    let mut stdout = child.stdout.take().expect("piped");
    let mut buf = Vec::new();
    let read = async {
        let mut chunk = [0u8; 4096];
        loop {
            let n = stdout.read(&mut chunk).await?;
            if n == 0 {
                return Ok::<bool, std::io::Error>(true);
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.iter().filter(|&&b| b == b'\n').count() >= 4 || buf.len() >= EXEC_MAX_OUTPUT {
                return Ok(false);
            }
        }
    };
    let finished = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(eof)) => eof,
        Ok(Err(e)) => {
            kill_group();
            res.error = Some(format!("running {prog}: {e}"));
            return res;
        }
        Err(_) => {
            kill_group();
            let _ = child.wait().await;
            res.error = Some(format!("external command ({prog}) did not finish within {}s", timeout.as_secs()));
            return res;
        }
    };
    if finished {
        // Output closed: give the command a moment to exit, then make sure.
        if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            kill_group();
        }
    } else {
        // We have what MRTG reads; stop the rest of the output.
        kill_group();
    }
    let _ = child.wait().await;
    res.time = now();
    let text = String::from_utf8_lossy(&buf);
    let lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([-+]?\d+(.\d+)?)").unwrap());
    let num = |label: &str, l: Option<&str>, res: &mut SourceResult| -> Option<String> {
        match l {
            None => {
                res.error = Some(format!("{prog}: expected a number for '{label}' but got nothing"));
                None
            }
            Some("UNKNOWN") => None,
            Some(l) => match NUM.captures(l) {
                Some(c) => Some(c[1].to_string()),
                None => {
                    res.error = Some(format!("{prog}: expected a number for '{label}'"));
                    None
                }
            },
        }
    };
    // `<EXTERNAL>` at EOF yields nothing: an empty final segment is not a line.
    let all: Vec<&str> = {
        let mut v: Vec<&str> = lines.take(5).collect();
        if v.last() == Some(&"") {
            v.pop();
        }
        v
    };
    res.input = num("in", all.first().copied(), &mut res);
    res.output = num("out", all.get(1).copied(), &mut res);
    if let Some(u) = all.get(2) {
        res.uptime = Some(u.trim_start().chars().take(256).collect());
    }
    if let Some(n) = all.get(3) {
        res.name = Some(n.trim_start().chars().take(256).collect());
    }
    res.input = res.input.map(|v| clean(&v)).filter(|v| numeric_ok(v));
    res.output = res.output.map(|v| clean(&v)).filter(|v| numeric_ok(v));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn external_parsing() {
        let r = run_external("/usr/bin/printf '12\\n 34 apples\\nup 3 days\\n  myhost\\n'", false, None, Duration::from_secs(5)).await;
        assert_eq!(r.input.as_deref(), Some("12"));
        assert_eq!(r.output.as_deref(), Some("34"));
        assert_eq!(r.uptime.as_deref(), Some("up 3 days"));
        assert_eq!(r.name.as_deref(), Some("myhost"));
        let r = run_external("/usr/bin/printf 'UNKNOWN\\n5\\n'", false, None, Duration::from_secs(5)).await;
        assert_eq!(r.input, None);
        assert_eq!(r.output.as_deref(), Some("5"));
        assert_eq!(r.uptime.as_deref(), Some("unknown"));
    }

    #[tokio::test]
    async fn external_needs_opt_in_for_shell_and_times_out() {
        let r = run_external("echo 1 | cat", false, None, Duration::from_secs(5)).await;
        assert!(r.error.unwrap().contains("needs a shell"));
        // Long output: the four lines MRTG reads are enough, no timeout.
        let t = std::time::Instant::now();
        let r = run_external("printf '1\n2\nup\nname\n'; yes 1234567890", true, None, Duration::from_secs(20)).await;
        assert_eq!((r.input.as_deref(), r.output.as_deref()), (Some("1"), Some("2")));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        // Arguments never appear in messages.
        let r = run_external("/nonexistent/prog --password=s3cret", false, None, Duration::from_secs(5)).await;
        assert!(!r.error.unwrap().contains("s3cret"));
        let r = run_external("echo 1; echo 2", true, None, Duration::from_secs(5)).await;
        assert_eq!((r.input.as_deref(), r.output.as_deref()), (Some("1"), Some("2")));
        let r = run_external("/bin/sleep 5", false, None, Duration::from_millis(200)).await;
        assert!(r.error.unwrap().contains("did not finish"));
    }

    #[test]
    fn pseudo_parsing() {
        assert!(matches!(Pseudo::parse("PseudoZero"), Some(Pseudo::Zero)));
        assert!(matches!(Pseudo::parse("WaLK2ifDescr"), Some(Pseudo::Walk(2, ref o)) if o == "ifDescr"));
        assert!(Pseudo::parse("ifInOctets.3").is_none());
    }
}
