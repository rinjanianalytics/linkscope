// SPDX-License-Identifier: GPL-2.0-or-later
//! Async SNMP client used by the poller and cfgmaker.
//!
//! Built on the `async-snmp` crate (BER, PDUs, USM, engine discovery and time
//! windows), which replaced `snmp2` in 2026-10 (AK): `snmp2` changed a v3
//! session's engine state from a reply before authenticating it. This module
//! adds what MRTG's `SNMP_Session` provided on top: timeout with retries and
//! MRTG's backoff (the timeout itself grows), batching many OIDs per request,
//! recovery from SNMPv1 `noSuchName` and `tooBig` errors, and table walks with
//! GETBULK or GETNEXT.
//!
//! Unlike MRTG (whose default was lenient source matching), replies are only
//! accepted from the address the request was sent to (`strict_source`): a
//! datagram from anywhere else is dropped and the request keeps waiting.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_snmp::{
    Auth, AuthProtocol, Client, Oid, PrivProtocol, Retry, UdpHandle, UsmConfig, Value, VarBind,
};
use linkscope_config::{SnmpOptions, SnmpSource, SnmpVersion};
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
    fn from_value(v: &Value) -> Val {
        match v {
            Value::Integer(i) => Val::Int(i64::from(*i)),
            Value::Counter32(c) | Value::Gauge32(c) | Value::UInteger32(c) => {
                Val::Unsigned(u64::from(*c))
            }
            Value::Counter64(c) => Val::Counter64(*c),
            Value::TimeTicks(t) => Val::TimeTicks(*t),
            Value::OctetString(s) => Val::Str(s.to_vec()),
            Value::ObjectIdentifier(o) => Val::Oid(o.to_string()),
            Value::IpAddress(a) => Val::Ip(*a),
            Value::Null => Val::Null,
            Value::NoSuchObject | Value::NoSuchInstance => Val::Missing,
            Value::EndOfMibView => Val::EndOfMib,
            Value::Opaque(o) | Value::Nsap(o) => Val::Opaque(o.to_vec()),
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
                let ok = !t.is_empty()
                    && t.trim_start_matches(['+', '-']).split_once('.').map_or(
                        t.trim_start_matches(['+', '-'])
                            .bytes()
                            .all(|b| b.is_ascii_digit()),
                        |(a, b)| {
                            a.bytes().all(|x| x.is_ascii_digit())
                                && b.bytes().all(|x| x.is_ascii_digit())
                        },
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
        // USM derives its keys from a password of at least eight octets (RFC 3414); net-snmp and
        // the SNMP library refuse shorter ones too.
        if auth.as_deref().is_some_and(|a| a.len() < 8)
            || privacy.as_deref().is_some_and(|p| p.len() < 8)
        {
            return Err("SNMPv3 passwords must be at least 8 characters long".into());
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
            auth_protocol: get("authprotocol")
                .unwrap_or_else(|| "md5".into())
                .to_lowercase(),
            auth_password: auth,
            priv_protocol: get("privprotocol")
                .unwrap_or_else(|| "des".into())
                .to_lowercase(),
            priv_password: privacy,
            context_name: get("contextname"),
        })
    }

    /// The USM user these parameters describe.
    fn usm(&self) -> Result<UsmConfig, String> {
        let auth = match self.auth_protocol.replace(['-', '_'], "").as_str() {
            "md5" | "hmacmd5" => AuthProtocol::Md5,
            "sha" | "sha1" | "hmacsha" => AuthProtocol::Sha1,
            "sha224" => AuthProtocol::Sha224,
            "sha256" => AuthProtocol::Sha256,
            "sha384" => AuthProtocol::Sha384,
            "sha512" => AuthProtocol::Sha512,
            p => return Err(format!("unknown authprotocol '{p}'")),
        };
        let usm = UsmConfig::new(self.username.clone().into_bytes());
        let usm = match (&self.auth_password, &self.priv_password) {
            (Some(a), Some(p)) => {
                let cipher = match self.priv_protocol.replace(['-', '_'], "").as_str() {
                    "des" | "cbcdes" => PrivProtocol::Des,
                    "aes" | "aes128" | "cfbaes128" => PrivProtocol::Aes128,
                    // AES-192 and AES-256 as net-snmp extends their keys (Blumenthal), and as
                    // Cisco does (Reeder, the `c` names).
                    "aes192" => PrivProtocol::Aes192Blumenthal,
                    "aes256" => PrivProtocol::Aes256Blumenthal,
                    "aes192c" => PrivProtocol::Aes192Reeder,
                    "aes256c" => PrivProtocol::Aes256Reeder,
                    c => return Err(format!("unknown privprotocol '{c}'")),
                };
                usm.auth_priv(auth, a.as_bytes(), cipher, p.as_bytes())
                    .map_err(|e| e.to_string())?
            }
            (Some(a), None) => usm.auth(auth, a.as_bytes()).map_err(|e| e.to_string())?,
            _ => usm,
        };
        Ok(match &self.context_name {
            Some(c) => usm.context_name(c.clone().into_bytes()),
            None => usm,
        })
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
        let num = |k: &str| {
            o.get(k)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
        };
        let fin = |v: Option<f64>| v.filter(|x| x.is_finite());
        let v3 = if s.conn.version == SnmpVersion::V3 {
            Some(V3Params::from_options(o)?)
        } else {
            None
        };
        Ok(Endpoint {
            host: s.host.clone(),
            port: s
                .conn
                .port
                .or_else(|| num("port").map(|p| p as u16))
                .unwrap_or(161),
            version: s.conn.version,
            community: s.community.clone(),
            v3,
            timeout: Duration::from_secs_f64(
                fin(s.conn.timeout)
                    .or_else(|| num("timeout"))
                    .unwrap_or(2.0)
                    .clamp(0.05, 300.0),
            ),
            retries: s
                .conn
                .retries
                .or_else(|| num("retries").map(|r| r as u32))
                .unwrap_or(5)
                .min(50),
            backoff: fin(s.conn.backoff)
                .or_else(|| num("backoff"))
                .unwrap_or(1.0)
                .clamp(1.0, 10.0),
            ipv4only: s.ipv4only || s.conn.v4only,
            max_repetitions: num("default_max_repetitions")
                .or_else(|| num("maxrepetitions"))
                .map(|r| r as u32)
                .unwrap_or(25)
                .clamp(1, 200),
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
    let pick = if ep.ipv4only {
        addrs.iter().find(|a| a.is_ipv4())
    } else {
        addrs.first()
    };
    pick.copied()
        .ok_or_else(|| SnmpError::Resolve(ep.host.clone()))
}

/// One open session to one agent. Requests on a session are sequential.
pub struct Session {
    client: Client<UdpHandle>,
    ep: Endpoint,
    /// The last exchange did not finish with an answer: it failed, or it was dropped mid-way (an
    /// agent's time budget ran out). The next one starts on a new socket, so no late reply to it
    /// can be read as the next answer, whatever the library does with request ids.
    unsettled: bool,
}

#[derive(Debug)]
struct Resp {
    error_status: u32,
    error_index: u32,
    varbinds: Vec<(String, Val)>,
}

enum Req<'a> {
    Get(&'a [Oid]),
    GetNext(&'a Oid),
    Bulk(&'a [Oid], u32),
}

const ERR_TOO_BIG: u32 = 1;
const ERR_NO_SUCH_NAME: u32 = 2;

/// The values of one GET reply, each placed where it was asked for: matched by OID, not only by
/// position, so an answer for another object (a stray or a stale reply) is no value for this one.
fn place_values(take: &[usize], want: &[String], varbinds: &[(String, Val)], out: &mut [Val]) {
    for (k, &i) in take.iter().enumerate() {
        if let Some((oid, v)) = varbinds.get(k)
            && *oid == want[i]
        {
            out[i] = v.clone();
        }
    }
}

fn to_oid(s: &str) -> Result<Oid, SnmpError> {
    // Arcs are 32-bit on the wire (RFC 2578); one that does not fit is no OID, not a truncated one.
    let parts: Option<Vec<u32>> =
        parse_numeric(s).and_then(|p| p.into_iter().map(|a| u32::try_from(a).ok()).collect());
    let parts = parts.ok_or_else(|| SnmpError::Protocol(format!("bad OID {s}")))?;
    Ok(Oid::from_slice(&parts))
}

fn varbinds(v: Vec<VarBind>) -> Vec<(String, Val)> {
    v.into_iter()
        .map(|b| (b.oid.to_string(), Val::from_value(&b.value)))
        .collect()
}

impl Session {
    /// Open a session (resolves the host; for v3 the engine is discovered on the first request).
    pub async fn open(ep: &Endpoint) -> Result<Session, SnmpError> {
        let addr = resolve(ep).await?;
        let auth: Auth = match ep.version {
            SnmpVersion::V1 => Auth::v1(ep.community.as_str()),
            SnmpVersion::V2c => Auth::v2c(ep.community.as_str()),
            SnmpVersion::V3 => ep
                .v3
                .as_ref()
                .ok_or_else(|| SnmpError::V3("missing USM parameters".into()))?
                .usm()
                .map_err(SnmpError::V3)?
                .into(),
        };
        // One attempt per call: the retries, and MRTG's growing timeout, are this module's.
        let client = Client::builder(addr, auth)
            .strict_source(true)
            .request_timeout(MAX_TIMEOUT)
            .retry(Retry::none())
            .max_oids_per_request(ep.max_varbinds.max(1))
            .connect()
            .await
            .map_err(|e| SnmpError::Protocol(format!("socket for {}: {e}", ep.display())))?;
        Ok(Session {
            client,
            ep: ep.clone(),
            unsettled: false,
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.ep
    }

    async fn exchange(&mut self, req: &Req<'_>) -> Result<Resp, SnmpError> {
        // A session whose last exchange ended without an answer starts over on a new socket, so a
        // late reply to that exchange can never be read as this one's.
        if self.unsettled {
            self.reopen().await?;
        }
        self.unsettled = true;
        let (res, rejected) = self.exchange_once(req).await;
        let res = match res {
            // When v3 replies came but were refused, or authentication failed (the agent was
            // restarted with new keys, or a reply was forged), start over once: a new socket and a
            // fresh discovery. Plain timeouts (the device is down) do not retry here.
            Err(e)
                if self.ep.version == SnmpVersion::V3
                    && (rejected || matches!(e, SnmpError::V3(_))) =>
            {
                tracing::debug!(
                    "SNMPv3 session for {} reopened after: {e}",
                    self.ep.display()
                );
                self.reopen().await?;
                self.unsettled = true;
                self.exchange_once(req).await.0
            }
            r => r,
        };
        self.unsettled = res.is_err();
        res
    }

    /// The same endpoint on a new socket (and for v3 a new discovery).
    async fn reopen(&mut self) -> Result<(), SnmpError> {
        let ep = self.ep.clone();
        *self = Session::open(&ep).await?;
        Ok(())
    }

    /// One request with its retries. Also says whether a reply arrived but was refused.
    async fn exchange_once(&mut self, req: &Req<'_>) -> (Result<Resp, SnmpError>, bool) {
        let mut rejected = false;
        let mut t = self.ep.timeout;
        let mut attempts = 0;
        loop {
            let client = &self.client;
            let fut = async {
                match req {
                    Req::Get(oids) => client.get_many(oids).await.map(|r| varbinds(r.varbinds)),
                    Req::GetNext(oid) => client.get_next(oid).await.map(|r| varbinds(r.varbinds)),
                    Req::Bulk(oids, reps) => client.get_bulk(oids, 0, *reps).await.map(varbinds),
                }
            };
            let outcome = tokio::time::timeout(t, fut).await;
            match outcome {
                Ok(Ok(varbinds)) => {
                    return (
                        Ok(Resp {
                            error_status: 0,
                            error_index: 0,
                            varbinds,
                        }),
                        rejected,
                    );
                }
                Ok(Err(e)) => match *e {
                    // The agent answered with an error-status: the callers decide (v1 noSuchName,
                    // tooBig, ...), as they did with the PDU's fields.
                    async_snmp::Error::Snmp { status, index, .. } => {
                        // An error answer without a known status counts as genErr (5).
                        let status = match u32::try_from(status.as_i32()) {
                            Ok(0) | Err(_) => 5,
                            Ok(s) => s,
                        };
                        return (
                            Ok(Resp {
                                error_status: status,
                                error_index: index,
                                varbinds: Vec::new(),
                            }),
                            rejected,
                        );
                    }
                    async_snmp::Error::Auth { .. } | async_snmp::Error::Report { .. } => {
                        return (
                            Err(SnmpError::V3(format!("{e} ({})", self.ep.display()))),
                            true,
                        );
                    }
                    // A reply that could not be used (it did not decode, or it broke the rules):
                    // refused, not a silence.
                    async_snmp::Error::Decode(..)
                    | async_snmp::Error::MalformedResponse { .. }
                    | async_snmp::Error::ResponseShape { .. } => {
                        rejected = true;
                    }
                    // Configuration the library refuses cannot get better by asking again.
                    async_snmp::Error::Config(_) | async_snmp::Error::Privacy(_) => {
                        return (
                            Err(SnmpError::V3(format!("{e} ({})", self.ep.display()))),
                            false,
                        );
                    }
                    _ => {}
                },
                Err(_) => {}
            }
            attempts += 1;
            if attempts > self.ep.retries {
                return (Err(SnmpError::Timeout(self.ep.display())), rejected);
            }
            t = next_timeout(t, self.ep.backoff);
        }
    }

    /// GET `oids` (numeric strings). The result has one entry per OID;
    /// unknown objects are `Val::Missing`. Requests are batched and split
    /// automatically on `tooBig` / v1 `noSuchName`.
    pub async fn get(&mut self, oids: &[String]) -> Result<Vec<Val>, SnmpError> {
        let mut out = vec![Val::Missing; oids.len()];
        let parsed: Vec<Option<Oid>> = oids.iter().map(|o| to_oid(o).ok()).collect();
        let mut chunk = self.ep.max_varbinds.max(1);
        // The OID each answer must carry, as the library writes it.
        let want: Vec<String> = parsed
            .iter()
            .map(|o| o.as_ref().map(|o| o.to_string()).unwrap_or_default())
            .collect();
        let mut pending: Vec<usize> = (0..oids.len()).filter(|&i| parsed[i].is_some()).collect();
        while !pending.is_empty() {
            let take: Vec<usize> = pending.iter().copied().take(chunk).collect();
            let batch: Vec<Oid> = take.iter().filter_map(|&i| parsed[i].clone()).collect();
            let r = self.exchange(&Req::Get(&batch)).await?;
            match r.error_status {
                0 => {
                    place_values(&take, &want, &r.varbinds, &mut out);
                    pending.drain(..take.len());
                }
                ERR_NO_SUCH_NAME => {
                    // v1: one bad varbind fails the request. Mark it and resend the rest.
                    let bad = (r.error_index as usize)
                        .checked_sub(1)
                        .filter(|&b| b < take.len())
                        .unwrap_or(0);
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
        Ok(r.varbinds
            .into_iter()
            .next()
            .filter(|(_, v)| *v != Val::EndOfMib))
    }

    /// Walk the subtree under `base` and return `(suffix, value)` pairs,
    /// where suffix is the OID part after `base.`. Uses GETBULK on v2c/v3.
    pub async fn walk(&mut self, base: &str) -> Result<Vec<(String, Val)>, SnmpError> {
        self.walk_rows(base, MAX_WALK_ROWS, false).await
    }

    /// Walk `base` for at most `max_rows` rows and stop there (the rest is not read).
    pub async fn walk_first(
        &mut self,
        base: &str,
        max_rows: usize,
    ) -> Result<Vec<(String, Val)>, SnmpError> {
        self.walk_rows(base, max_rows.min(MAX_WALK_ROWS), true)
            .await
    }

    /// A walk: at most `max_rows` rows and [`MAX_WALK_BYTES`] (counting each row's overhead), and
    /// 600 s. Past the rows, `stop_early` ends it with what it has; otherwise it is an error.
    async fn walk_rows(
        &mut self,
        base: &str,
        max_rows: usize,
        stop_early: bool,
    ) -> Result<Vec<(String, Val)>, SnmpError> {
        let base = base.trim_start_matches('.').to_string();
        let prefix = format!("{base}.");
        let mut out = Vec::new();
        let mut cur = base.clone();
        let bulk = self.ep.version != SnmpVersion::V1 && self.ep.use_getbulk;
        let started = Instant::now();
        let mut bytes = 0usize;
        loop {
            if out.len() >= max_rows && stop_early {
                break;
            }
            if started.elapsed() > Duration::from_secs(600)
                || out.len() > max_rows
                || bytes > MAX_WALK_BYTES
            {
                return Err(SnmpError::Protocol(format!(
                    "walk of {base} on {} did not terminate",
                    self.ep.display()
                )));
            }
            let o = to_oid(&cur)?;
            let r = if bulk {
                self.exchange(&Req::Bulk(
                    std::slice::from_ref(&o),
                    self.ep.max_repetitions,
                ))
                .await?
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
                if out.len() >= max_rows && stop_early {
                    done = true;
                    break;
                }
                cur = oid.clone();
                bytes += WALK_ROW_OVERHEAD
                    + oid.len()
                    + match &v {
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
/// Rows one walk may return: far more than any interface table (it was a million).
pub const MAX_WALK_ROWS: usize = 100_000;
/// What a row costs besides its OID and value (strings, the value, the vector), counted against
/// [`MAX_WALK_BYTES`] so that many tiny rows cannot use far more memory than the cap says.
const WALK_ROW_OVERHEAD: usize = 96;

/// Numeric OID comparison.
pub fn oid_gt(a: &str, b: &str) -> bool {
    let pa = parse_numeric(a).unwrap_or_default();
    let pb = parse_numeric(b).unwrap_or_default();
    pa > pb
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
        if let Some(s) = self
            .v3
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(e, _)| e == ep)
            .map(|(_, s)| s.clone())
        {
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
    fn get_values_are_matched_by_oid() {
        let want = vec![
            "1.3.6.1.2.1.2.2.1.10.1".to_string(),
            "1.3.6.1.2.1.2.2.1.16.1".to_string(),
        ];
        let mut out = vec![Val::Missing, Val::Missing];
        // The right objects in order: both placed.
        let ok = vec![
            (want[0].clone(), Val::Unsigned(5)),
            (want[1].clone(), Val::Unsigned(7)),
        ];
        place_values(&[0, 1], &want, &ok, &mut out);
        assert_eq!(out, [Val::Unsigned(5), Val::Unsigned(7)]);
        // A reply for other objects (another interface, a stale answer): nothing placed.
        let mut out = vec![Val::Missing, Val::Missing];
        let other = vec![
            ("1.3.6.1.2.1.2.2.1.10.2".to_string(), Val::Unsigned(9)),
            (want[1].clone(), Val::Unsigned(7)),
        ];
        place_values(&[0, 1], &want, &other, &mut out);
        assert_eq!(out, [Val::Missing, Val::Unsigned(7)]);
    }

    /// A session whose request got no answer asks again from a new socket, so a late reply to the
    /// old request (the library reuses its id) cannot be read as the new one's. Nothing answers
    /// here: the test only watches where the requests come from.
    #[tokio::test]
    async fn an_unanswered_request_is_followed_by_a_new_socket() {
        let sink = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ep = Endpoint {
            host: "127.0.0.1".into(),
            port: sink.local_addr().unwrap().port(),
            version: SnmpVersion::V2c,
            community: "public".into(),
            v3: None,
            timeout: Duration::from_millis(100),
            retries: 0,
            backoff: 1.0,
            ipv4only: true,
            max_repetitions: 25,
            use_getbulk: true,
            max_varbinds: 32,
        };
        let mut s = Session::open(&ep).await.unwrap();
        let oid = vec!["1.3.6.1.2.1.1.5.0".to_string()];
        assert!(s.get(&oid).await.is_err());
        assert!(s.get(&oid).await.is_err());
        let mut buf = [0u8; 1500];
        let (_, first) = sink.recv_from(&mut buf).await.unwrap();
        let (_, second) = sink.recv_from(&mut buf).await.unwrap();
        assert_ne!(
            first.port(),
            second.port(),
            "the second request came from the same socket"
        );
    }

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
            &[
                ("username", "u"),
                ("authpassword", "authpass1"),
                ("authprotocol", "sha256"),
            ],
            &[
                ("username", "u"),
                ("authpassword", "authpass1"),
                ("privpassword", "privpass1"),
                ("privprotocol", "aes"),
            ],
        ] {
            assert!(V3Params::from_options(&opts(ok)).is_ok(), "{ok:?}");
        }
        // Less than was asked for, or a password that would panic the library: refused.
        for bad in [
            &[("username", "u"), ("privpassword", "privpass1")][..],
            &[("username", "u"), ("privprotocol", "aes")],
            &[("username", "u"), ("authprotocol", "sha")],
            &[
                ("username", "u"),
                ("authpassword", "authpass1"),
                ("privprotocol", "aes"),
            ],
            &[("username", "u"), ("authpassword", "")],
            &[
                ("username", "u"),
                ("authpassword", "authpass1"),
                ("privpassword", ""),
            ],
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
