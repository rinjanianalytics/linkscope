// SPDX-License-Identifier: GPL-2.0-or-later
//! Async SNMP client used by the poller and cfgmaker.
//!
//! Built on the `snmp2` crate (BER, PDUs, USM). This module adds what MRTG's
//! `SNMP_Session` provided on top: timeout with retries and exponential
//! backoff, batching many OIDs per request, recovery from SNMPv1
//! `noSuchName` and `tooBig` errors, table walks with GETBULK or GETNEXT,
//! and SNMPv3 engine discovery and time-window resync.
//!
//! Unlike MRTG (whose default was lenient source matching), replies are only
//! accepted from the address the request was sent to: the UDP socket is
//! connected, so the kernel drops datagrams from anywhere else.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use linkscope_config::{SnmpOptions, SnmpSource, SnmpVersion};
use snmp2::v3::{Auth, AuthProtocol, Cipher, Security};
use snmp2::{AsyncSession, Oid, Value};
use tokio::sync::Mutex;

use crate::oids::parse_numeric;

/// An SNMP value, owned.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Int(i64),
    /// Counter32, Gauge32, Unsigned32.
    Unsigned(u64),
    Counter64(u64),
    TimeTicks(u32),
    Str(Vec<u8>),
    Oid(String),
    Ip([u8; 4]),
    Null,
    /// noSuchObject / noSuchInstance, or a v1 noSuchName for this varbind.
    Missing,
    EndOfMib,
    Opaque(Vec<u8>),
}

impl Val {
    fn from_value(v: &Value<'_>) -> Val {
        match v {
            Value::Integer(i) => Val::Int(*i),
            Value::Counter32(c) | Value::Unsigned32(c) => Val::Unsigned(u64::from(*c)),
            Value::Counter64(c) => Val::Counter64(*c),
            Value::Timeticks(t) => Val::TimeTicks(*t),
            Value::OctetString(s) => Val::Str(s.to_vec()),
            Value::ObjectIdentifier(o) => Val::Oid(o.to_id_string()),
            Value::IpAddress(a) => Val::Ip(*a),
            Value::Null => Val::Null,
            Value::Boolean(b) => Val::Int(i64::from(*b)),
            Value::NoSuchObject | Value::NoSuchInstance => Val::Missing,
            Value::EndOfMibView => Val::EndOfMib,
            Value::Opaque(o) => Val::Opaque(o.to_vec()),
            _ => Val::Missing,
        }
    }

    /// True for values that carry data.
    pub fn is_present(&self) -> bool {
        !matches!(self, Val::Missing | Val::EndOfMib | Val::Null)
    }

    /// The value as MRTG's `SNMP_util` would print it: numbers in decimal,
    /// strings raw, TimeTicks as `D days, H:MM:SS`.
    pub fn pretty(&self) -> String {
        match self {
            Val::Int(i) => i.to_string(),
            Val::Unsigned(u) | Val::Counter64(u) => u.to_string(),
            Val::TimeTicks(t) => pretty_uptime(u64::from(*t)),
            Val::Str(s) | Val::Opaque(s) => String::from_utf8_lossy(s).into_owned(),
            Val::Oid(o) => o.clone(),
            Val::Ip(a) => format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]),
            Val::Null => String::new(),
            Val::Missing | Val::EndOfMib => String::new(),
        }
    }

    /// Decimal text if the value is numeric.
    pub fn number(&self) -> Option<String> {
        match self {
            Val::Int(i) => Some(i.to_string()),
            Val::Unsigned(u) | Val::Counter64(u) => Some(u.to_string()),
            Val::TimeTicks(t) => Some(t.to_string()),
            Val::Str(s) => {
                // Some agents return numbers as strings (e.g. UCD-SNMP); MRTG
                // accepted those after trimming.
                let t = String::from_utf8_lossy(s).trim().to_string();
                let ok = !t.is_empty() && t.trim_start_matches(['+', '-']).split_once('.').map_or(
                    t.trim_start_matches(['+', '-']).bytes().all(|b| b.is_ascii_digit()),
                    |(a, b)| a.bytes().all(|x| x.is_ascii_digit()) && b.bytes().all(|x| x.is_ascii_digit()),
                );
                ok.then_some(t)
            }
            _ => None,
        }
    }
}

/// `BER::pretty_uptime_value`.
pub fn pretty_uptime(ticks: u64) -> String {
    let mut t = ticks / 100;
    let days = t / 86400;
    t %= 86400;
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    match days {
        0 => format!("{h}:{m:02}:{s:02}"),
        1 => format!("1 day, {h}:{m:02}:{s:02}"),
        d => format!("{d} days, {h}:{m:02}:{s:02}"),
    }
}

/// Errors from SNMP operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SnmpError {
    #[error("no response from {0}")]
    Timeout(String),
    #[error("cannot resolve {0}")]
    Resolve(String),
    #[error("{0}")]
    Protocol(String),
    #[error("SNMPv3: {0}")]
    V3(String),
}

/// SNMPv3 USM parameters, from `SnmpOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct V3Params {
    pub username: String,
    pub auth_protocol: String,
    pub auth_password: Option<String>,
    pub priv_protocol: String,
    pub priv_password: Option<String>,
    pub context_name: Option<String>,
}

impl V3Params {
    pub fn from_options(o: &SnmpOptions) -> Result<V3Params, String> {
        let get = |k: &str| o.get(k).cloned();
        let username = get("username").ok_or("SNMPv3 requires a username")?;
        if get("authkey").is_some() || get("privkey").is_some() {
            return Err("SNMPv3 authkey/privkey (localized keys) are not supported; use authpassword/privpassword".into());
        }
        // A security level is what was asked for or an error, never quietly less: a privacy
        // password without authentication used to poll in clear, with no authentication.
        let auth = get("authpassword");
        let privacy = get("privpassword");
        if auth.as_deref().is_some_and(str::is_empty) || privacy.as_deref().is_some_and(str::is_empty) {
            return Err("SNMPv3 passwords must not be empty".into());
        }
        if auth.is_none() && (privacy.is_some() || get("privprotocol").is_some()) {
            return Err("SNMPv3 privacy needs authentication: set authpassword too".into());
        }
        if auth.is_none() && get("authprotocol").is_some() {
            return Err("SNMPv3 authprotocol is set without an authpassword".into());
        }
        if privacy.is_none() && get("privprotocol").is_some() {
            return Err("SNMPv3 privprotocol is set without a privpassword".into());
        }
        Ok(V3Params {
            username,
            auth_protocol: get("authprotocol").unwrap_or_else(|| "md5".into()).to_lowercase(),
            auth_password: auth,
            priv_protocol: get("privprotocol").unwrap_or_else(|| "des".into()).to_lowercase(),
            priv_password: privacy,
            context_name: get("contextname"),
        })
    }

    fn security(&self) -> Result<Security, String> {
        let proto = match self.auth_protocol.replace(['-', '_'], "").as_str() {
            "md5" | "hmacmd5" => AuthProtocol::Md5,
            "sha" | "sha1" | "hmacsha" => AuthProtocol::Sha1,
            "sha224" => AuthProtocol::Sha224,
            "sha256" => AuthProtocol::Sha256,
            "sha384" => AuthProtocol::Sha384,
            "sha512" => AuthProtocol::Sha512,
            p => return Err(format!("unknown authprotocol '{p}'")),
        };
        let mut sec = match &self.auth_password {
            Some(a) => Security::new(self.username.as_bytes(), a.as_bytes()).with_auth_protocol(proto),
            None => Security::new(self.username.as_bytes(), b"").with_auth(Auth::NoAuthNoPriv),
        };
        if let (Some(_), Some(p)) = (&self.auth_password, &self.priv_password) {
            let cipher = match self.priv_protocol.replace(['-', '_'], "").as_str() {
                "des" | "cbcdes" => Cipher::Des,
                "aes" | "aes128" | "cfbaes128" => Cipher::Aes128,
                "aes192" => Cipher::Aes192,
                "aes256" => Cipher::Aes256,
                c => return Err(format!("unknown privprotocol '{c}'")),
            };
            sec = sec.with_auth(Auth::AuthPriv { cipher, privacy_password: p.as_bytes().to_vec() });
        } else if self.auth_password.is_some() {
            sec = sec.with_auth(Auth::AuthNoPriv);
        }
        if let Some(c) = &self.context_name {
            sec = sec.with_context_name(c);
        }
        Ok(sec)
    }
}

/// Everything needed to talk to one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// Host as written in the config (IPv6 may be bracketed).
    pub host: String,
    pub port: u16,
    pub version: SnmpVersion,
    pub community: String,
    pub v3: Option<V3Params>,
    pub timeout: Duration,
    pub retries: u32,
    pub backoff: f64,
    pub ipv4only: bool,
    pub max_repetitions: u32,
    pub use_getbulk: bool,
    /// Maximum varbinds per GET request (1 with `SingleRequest`).
    pub max_varbinds: usize,
}

impl Endpoint {
    /// Build from a parsed Target source, applying MRTG's defaults
    /// (timeout 2 s, 5 retries, backoff 1.0) and `SnmpOptions` overrides.
    pub fn from_source(s: &SnmpSource, single_request: bool) -> Result<Endpoint, String> {
        let o = &s.snmp_options;
        // Only finite numbers count; "nan" or "inf" fall back to defaults.
        let num = |k: &str| o.get(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
        let fin = |v: Option<f64>| v.filter(|x| x.is_finite());
        let v3 = if s.conn.version == SnmpVersion::V3 { Some(V3Params::from_options(o)?) } else { None };
        Ok(Endpoint {
            host: s.host.clone(),
            port: s.conn.port.or_else(|| num("port").map(|p| p as u16)).unwrap_or(161),
            version: s.conn.version,
            community: s.community.clone(),
            v3,
            timeout: Duration::from_secs_f64(fin(s.conn.timeout).or_else(|| num("timeout")).unwrap_or(2.0).clamp(0.05, 300.0)),
            retries: s.conn.retries.or_else(|| num("retries").map(|r| r as u32)).unwrap_or(5).min(50),
            backoff: fin(s.conn.backoff).or_else(|| num("backoff")).unwrap_or(1.0).clamp(1.0, 10.0),
            ipv4only: s.ipv4only || s.conn.v4only,
            max_repetitions: num("default_max_repetitions").or_else(|| num("maxrepetitions")).map(|r| r as u32).unwrap_or(25).clamp(1, 200),
            use_getbulk: num("use_getbulk").map(|v| v != 0.0).unwrap_or(true),
            max_varbinds: if single_request { 1 } else { 32 },
        })
    }

    /// MRTG's confcache host key: `cleanhostkey("community@host:snmpinfo")`.
    pub fn display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

async fn resolve(ep: &Endpoint) -> Result<SocketAddr, SnmpError> {
    let host = ep.host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, ep.port))
        .await
        .map_err(|_| SnmpError::Resolve(ep.host.clone()))?
        .collect();
    // MRTG uses the first address getaddrinfo returns, or the first IPv4
    // address for IPv4-only targets.
    let pick = if ep.ipv4only { addrs.iter().find(|a| a.is_ipv4()) } else { addrs.first() };
    pick.copied().ok_or_else(|| SnmpError::Resolve(ep.host.clone()))
}

/// One open session to one agent. Requests on a session are sequential.
pub struct Session {
    inner: AsyncSession,
    ep: Endpoint,
    initialized: bool,
}

#[derive(Debug)]
struct Resp {
    error_status: u32,
    error_index: u32,
    varbinds: Vec<(String, Val)>,
}

enum Req<'a> {
    Get(&'a [Oid<'static>]),
    GetNext(&'a Oid<'static>),
    Bulk(&'a [Oid<'static>], u32),
}

const ERR_TOO_BIG: u32 = 1;
const ERR_NO_SUCH_NAME: u32 = 2;

fn to_oid(s: &str) -> Result<Oid<'static>, SnmpError> {
    let parts = parse_numeric(s).ok_or_else(|| SnmpError::Protocol(format!("bad OID {s}")))?;
    Oid::from(&parts).map_err(|_| SnmpError::Protocol(format!("bad OID {s}")))
}

impl Session {
    /// Open a session (resolves the host; for v3 the engine is discovered
    /// lazily on the first request).
    pub async fn open(ep: &Endpoint) -> Result<Session, SnmpError> {
        let addr = resolve(ep).await?;
        let req_id = rand_req_id();
        let inner = match ep.version {
            SnmpVersion::V1 => AsyncSession::new_v1(addr, ep.community.as_bytes(), req_id).await,
            SnmpVersion::V2c => AsyncSession::new_v2c(addr, ep.community.as_bytes(), req_id).await,
            SnmpVersion::V3 => {
                let sec = ep.v3.as_ref().ok_or_else(|| SnmpError::V3("missing USM parameters".into()))?.security().map_err(SnmpError::V3)?;
                AsyncSession::new_v3(addr, req_id, sec).await
            }
        }
        .map_err(|e| SnmpError::Protocol(format!("socket for {}: {e}", ep.display())))?;
        Ok(Session { inner, ep: ep.clone(), initialized: ep.version != SnmpVersion::V3 })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.ep
    }

    async fn ensure_init(&mut self) -> Result<(), SnmpError> {
        if self.initialized {
            return Ok(());
        }
        let mut t = self.ep.timeout;
        for _ in 0..=self.ep.retries {
            match tokio::time::timeout(t, self.inner.init()).await {
                Ok(Ok(())) => {
                    self.initialized = true;
                    return Ok(());
                }
                Ok(Err(e)) => return Err(SnmpError::V3(format!("{} ({})", e, self.ep.display()))),
                Err(_) => t = next_timeout(t, self.ep.backoff),
            }
        }
        Err(SnmpError::Timeout(self.ep.display()))
    }

    async fn exchange(&mut self, req: &Req<'_>) -> Result<Resp, SnmpError> {
        let (res, rejected) = self.exchange_once(req).await;
        match res {
            // The v3 library sets a session's engine boots and time from a reply before it has
            // checked the reply's authentication, so one forged datagram can make every genuine
            // reply fail from then on. When replies came but were refused, or authentication
            // failed, start over once: a new socket (a new port to guess) and a fresh discovery.
            // Plain timeouts (the device is down) do not reopen it.
            Err(e) if self.ep.version == SnmpVersion::V3 && (rejected || matches!(e, SnmpError::V3(_))) => {
                tracing::debug!("SNMPv3 session for {} reopened after: {e}", self.ep.display());
                let ep = self.ep.clone();
                *self = Session::open(&ep).await?;
                self.exchange_once(req).await.0
            }
            r => r,
        }
    }

    /// One request with its retries. Also says whether a reply arrived but was refused.
    async fn exchange_once(&mut self, req: &Req<'_>) -> (Result<Resp, SnmpError>, bool) {
        if let Err(e) = self.ensure_init().await {
            return (Err(e), false);
        }
        let mut rejected = false;
        let mut t = self.ep.timeout;
        let mut attempts = 0;
        let mut resyncs = 0;
        loop {
            let inner = &mut self.inner;
            let fut = async {
                let pdu = match req {
                    Req::Get(oids) => {
                        let refs: Vec<&Oid> = oids.iter().collect();
                        inner.get_many(&refs).await
                    }
                    Req::GetNext(oid) => inner.getnext(oid).await,
                    Req::Bulk(oids, reps) => {
                        let refs: Vec<&Oid> = oids.iter().collect();
                        inner.getbulk(&refs, 0, *reps).await
                    }
                };
                pdu.map(|mut p| Resp {
                    error_status: p.error_status,
                    error_index: p.error_index,
                    varbinds: p.varbinds.by_ref().map(|(o, v)| (o.to_id_string(), Val::from_value(&v))).collect(),
                })
            };
            let outcome = tokio::time::timeout(t, fut).await;
            // A reply that came but was refused, not a silence.
            let refused = matches!(&outcome, Ok(Err(_)));
            match outcome {
                Ok(Ok(r)) => return (Ok(r), rejected),
                Ok(Err(snmp2::Error::AuthUpdated)) if resyncs < 3 => {
                    // v3 time window or engine boots changed; the security
                    // state is updated, so simply repeat the request.
                    resyncs += 1;
                    continue;
                }
                Ok(Err(e @ snmp2::Error::AuthFailure(_))) => {
                    return (Err(SnmpError::V3(format!("{e} ({})", self.ep.display()))), true);
                }
                Ok(Err(_)) | Err(_) => {
                    rejected |= refused;
                    attempts += 1;
                    if attempts > self.ep.retries {
                        return (Err(SnmpError::Timeout(self.ep.display())), rejected);
                    }
                    t = next_timeout(t, self.ep.backoff);
                }
            }
        }
    }

    /// GET `oids` (numeric strings). The result has one entry per OID;
    /// unknown objects are `Val::Missing`. Requests are batched and split
    /// automatically on `tooBig` / v1 `noSuchName`.
    pub async fn get(&mut self, oids: &[String]) -> Result<Vec<Val>, SnmpError> {
        let mut out = vec![Val::Missing; oids.len()];
        let parsed: Vec<Option<Oid<'static>>> = oids.iter().map(|o| to_oid(o).ok()).collect();
        let mut chunk = self.ep.max_varbinds.max(1);
        let mut pending: Vec<usize> = (0..oids.len()).filter(|&i| parsed[i].is_some()).collect();
        while !pending.is_empty() {
            let take: Vec<usize> = pending.iter().copied().take(chunk).collect();
            let batch: Vec<Oid<'static>> = take.iter().map(|&i| parsed[i].clone().expect("parsed")).collect();
            let r = self.exchange(&Req::Get(&batch)).await?;
            match r.error_status {
                0 => {
                    for (k, &i) in take.iter().enumerate() {
                        if let Some((_, v)) = r.varbinds.get(k) {
                            out[i] = v.clone();
                        }
                    }
                    pending.drain(..take.len());
                }
                ERR_NO_SUCH_NAME => {
                    // v1: one bad varbind fails the request. Mark it and resend the rest.
                    let bad = (r.error_index as usize).checked_sub(1).filter(|&b| b < take.len()).unwrap_or(0);
                    let idx = take[bad];
                    out[idx] = Val::Missing;
                    pending.retain(|&i| i != idx);
                }
                ERR_TOO_BIG if chunk > 1 => chunk = chunk.div_ceil(2),
                s => {
                    // Other errors (genErr, ...) apply to the whole batch; leave those
                    // values missing, but keep polling the rest.
                    tracing::debug!("{}: error-status {s} for batch", self.ep.display());
                    pending.drain(..take.len());
                }
            }
        }
        Ok(out)
    }

    /// GETNEXT of one OID: the following (OID, value) or `None` at end of MIB.
    pub async fn get_next(&mut self, oid: &str) -> Result<Option<(String, Val)>, SnmpError> {
        let o = to_oid(oid)?;
        let r = self.exchange(&Req::GetNext(&o)).await?;
        if r.error_status != 0 {
            return Ok(None);
        }
        Ok(r.varbinds.into_iter().next().filter(|(_, v)| *v != Val::EndOfMib))
    }

    /// Walk the subtree under `base` and return `(suffix, value)` pairs,
    /// where suffix is the OID part after `base.`. Uses GETBULK on v2c/v3.
    pub async fn walk(&mut self, base: &str) -> Result<Vec<(String, Val)>, SnmpError> {
        let base = base.trim_start_matches('.').to_string();
        let prefix = format!("{base}.");
        let mut out = Vec::new();
        let mut cur = base.clone();
        let bulk = self.ep.version != SnmpVersion::V1 && self.ep.use_getbulk;
        let started = Instant::now();
        let mut bytes = 0usize;
        loop {
            if started.elapsed() > Duration::from_secs(600) || out.len() > 1_000_000 || bytes > MAX_WALK_BYTES {
                return Err(SnmpError::Protocol(format!("walk of {base} on {} did not terminate", self.ep.display())));
            }
            let o = to_oid(&cur)?;
            let r = if bulk {
                self.exchange(&Req::Bulk(std::slice::from_ref(&o), self.ep.max_repetitions)).await?
            } else {
                self.exchange(&Req::GetNext(&o)).await?
            };
            if r.error_status != 0 || r.varbinds.is_empty() {
                break;
            }
            let mut done = false;
            for (oid, v) in r.varbinds {
                if v == Val::EndOfMib || !oid.starts_with(&prefix) {
                    done = true;
                    break;
                }
                if !oid_gt(&oid, &cur) {
                    // Agent is not making progress; stop rather than loop.
                    done = true;
                    break;
                }
                cur = oid.clone();
                bytes += oid.len() + match &v {
                    Val::Str(b) | Val::Opaque(b) => b.len(),
                    _ => 8,
                };
                out.push((oid[prefix.len()..].to_string(), v));
            }
            if done {
                break;
            }
        }
        Ok(out)
    }
}

/// Longest wait for one reply after backoff growth.
const MAX_TIMEOUT: Duration = Duration::from_secs(300);

fn next_timeout(t: Duration, backoff: f64) -> Duration {
    // t <= 300 s and backoff <= 10, so the product cannot overflow.
    t.mul_f64(backoff).min(MAX_TIMEOUT)
}

/// Total value bytes a single walk may return.
pub const MAX_WALK_BYTES: usize = 32 * 1024 * 1024;

/// Numeric OID comparison.
pub fn oid_gt(a: &str, b: &str) -> bool {
    let pa = parse_numeric(a).unwrap_or_default();
    let pb = parse_numeric(b).unwrap_or_default();
    pa > pb
}

fn rand_req_id() -> i32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    (h.finish() & 0x3fff_ffff) as i32
}

/// Cache of SNMPv3 sessions (engine discovery is a round trip, so v3
/// sessions are kept between polling cycles). v1/v2c sessions are cheap and
/// are opened per cycle.
#[derive(Default, Clone)]
pub struct SessionPool {
    v3: Arc<std::sync::Mutex<Vec<(Endpoint, Arc<Mutex<Session>>)>>>,
}

impl SessionPool {
    const MAX_V3: usize = 512;

    /// Get a session for `ep`.
    pub async fn session(&self, ep: &Endpoint) -> Result<Arc<Mutex<Session>>, SnmpError> {
        if ep.version != SnmpVersion::V3 {
            return Ok(Arc::new(Mutex::new(Session::open(ep).await?)));
        }
        if let Some(s) = self.v3.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(e, _)| e == ep).map(|(_, s)| s.clone()) {
            return Ok(s);
        }
        let s = Arc::new(Mutex::new(Session::open(ep).await?));
        let mut v = self.v3.lock().unwrap_or_else(|e| e.into_inner());
        if v.len() >= Self::MAX_V3 {
            v.remove(0);
        }
        v.push((ep.clone(), s.clone()));
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_settings_are_complete_or_refused() {
        let opts = |kv: &[(&str, &str)]| {
            let mut o = SnmpOptions::new();
            for (k, v) in kv {
                o.insert((*k).into(), (*v).into());
            }
            o
        };
        // What was asked for: no security, authentication, authentication and privacy.
        for ok in [
            &[("username", "u")][..],
            &[("username", "u"), ("authpassword", "authpass1"), ("authprotocol", "sha256")],
            &[("username", "u"), ("authpassword", "authpass1"), ("privpassword", "privpass1"), ("privprotocol", "aes")],
        ] {
            assert!(V3Params::from_options(&opts(ok)).is_ok(), "{ok:?}");
        }
        // Less than was asked for, or a password that would panic the library: refused.
        for bad in [
            &[("username", "u"), ("privpassword", "privpass1")][..],
            &[("username", "u"), ("privprotocol", "aes")],
            &[("username", "u"), ("authprotocol", "sha")],
            &[("username", "u"), ("authpassword", "authpass1"), ("privprotocol", "aes")],
            &[("username", "u"), ("authpassword", "")],
            &[("username", "u"), ("authpassword", "authpass1"), ("privpassword", "")],
        ] {
            assert!(V3Params::from_options(&opts(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn uptime_format_matches_ber_pm() {
        assert_eq!(pretty_uptime(0), "0:00:00");
        assert_eq!(pretty_uptime(8_640_000), "1 day, 0:00:00");
        assert_eq!(pretty_uptime(8_640_000 * 3 + 366_100), "3 days, 1:01:01");
    }

    #[test]
    fn oid_ordering() {
        assert!(oid_gt("1.3.6.1.2.1.2.2.1.10.10", "1.3.6.1.2.1.2.2.1.10.9"));
        assert!(!oid_gt("1.3.6.1.2", "1.3.6.1.2"));
    }
}
