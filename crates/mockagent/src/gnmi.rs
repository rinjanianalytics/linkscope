// SPDX-License-Identifier: GPL-2.0-or-later
//! A gNMI server that answers like a router, for tests and demos.
//!
//! It holds a few interfaces with counters that grow at a set rate, and answers `Capabilities`,
//! `Get` and `Subscribe` in `ONCE` mode for the OpenConfig paths or SR Linux's own, as a real
//! device does: a path it does not have fails the whole request with `NOT_FOUND`, a wrong login
//! with `UNAUTHENTICATED`. Switches make it behave like the awkward devices LinkScope must still
//! read: no `Get` at all, or a leaf answered with its parent container as JSON.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use linkscope_gnmi::g_nmi_server::{GNmi, GNmiServer};
use linkscope_gnmi::{
    CapabilityRequest, CapabilityResponse, Encoding, GetRequest, GetResponse, ModelData,
    Notification, Path, PathElem, SetRequest, SetResponse, SubscribeRequest, SubscribeResponse,
    TypedValue, Update, subscribe_request, subscribe_response, subscription_list, typed_value,
};
use tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

/// Which paths the device answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    OpenConfig,
    Srl,
}

/// What the device does with a path's `origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origins {
    /// Any origin, or none (most devices).
    Any,
    /// OpenConfig paths only with origin `openconfig` (SR Linux, whose default is its own model).
    OpenConfigNeedsIt,
    /// No origin accepted (`INVALID_ARGUMENT` for any).
    Refused,
}

/// One interface of the mock device.
#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub description: String,
    pub up: bool,
    /// Bits per second, as the device reports its speed.
    pub speed_bps: u64,
    pub if_index: u32,
    pub in_octets: u64,
    pub out_octets: u64,
    /// Bytes per second the counters grow by (0 keeps them fixed, for tests).
    pub in_rate: u64,
    pub out_rate: u64,
}

impl Interface {
    pub fn new(name: &str, if_index: u32, in_octets: u64, out_octets: u64) -> Interface {
        Interface {
            name: name.into(),
            description: format!("to {name}"),
            up: true,
            speed_bps: 10_000_000_000,
            if_index,
            in_octets,
            out_octets,
            in_rate: 0,
            out_rate: 0,
        }
    }
}

/// The device and how it behaves.
#[derive(Debug, Clone)]
pub struct Device {
    pub model: Model,
    pub hostname: String,
    pub interfaces: Vec<Interface>,
    /// `(user, password)` every request must carry, or none.
    pub login: Option<(String, String)>,
    /// False: `Get` answers `UNIMPLEMENTED`, as on devices that only stream.
    pub get: bool,
    /// True: a leaf is answered with its parent container, as one JSON object.
    pub containers: bool,
    /// The encodings offered in `Capabilities`.
    pub encodings: Vec<Encoding>,
    /// Seconds the device has been up when the server starts.
    pub uptime_secs: u64,
    /// A gateway's name for this device: requests must carry it in their prefix's `target`.
    pub target: Option<String>,
    pub origins: Origins,
    /// True: an unknown interface is answered with no updates, not `NOT_FOUND` (SR Linux).
    pub missing_is_empty: bool,
}

impl Device {
    pub fn new(model: Model, hostname: &str, interfaces: Vec<Interface>) -> Device {
        Device {
            model,
            hostname: hostname.into(),
            interfaces,
            login: None,
            get: true,
            containers: false,
            encodings: vec![Encoding::JsonIetf, Encoding::Json, Encoding::Proto],
            uptime_secs: 90_061,
            target: None,
            origins: Origins::Any,
            missing_is_empty: false,
        }
    }
}

/// Requests seen, for tests to check what the client asked.
#[derive(Debug, Default)]
pub struct Seen {
    pub gets: usize,
    pub subscribes: usize,
    pub encodings: Vec<i32>,
}

pub struct Mock {
    dev: Device,
    started: Instant,
    boot_ns: u64,
    pub seen: Arc<std::sync::Mutex<Seen>>,
}

impl Mock {
    pub fn new(dev: Device) -> Mock {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let boot_ns = now.saturating_sub(dev.uptime_secs) * 1_000_000_000;
        Mock {
            dev,
            started: Instant::now(),
            boot_ns,
            seen: Default::default(),
        }
    }

    pub fn into_server(self) -> GNmiServer<Mock> {
        GNmiServer::new(self)
    }

    fn check_login<T>(&self, req: &Request<T>) -> Result<(), Status> {
        let Some((u, p)) = &self.dev.login else {
            return Ok(());
        };
        let md = req.metadata();
        let got = |k: &str| md.get(k).and_then(|v| v.to_str().ok()).unwrap_or("");
        if got("username") == u && got("password") == p {
            Ok(())
        } else {
            Err(Status::unauthenticated("authentication failed"))
        }
    }

    /// Every leaf the device has now, with its value.
    fn leaves(&self) -> Vec<(Vec<PathElem>, Leaf)> {
        let elapsed = self.started.elapsed().as_secs();
        let mut out = Vec::new();
        let e = |name: &str| PathElem {
            name: name.into(),
            key: HashMap::new(),
        };
        let keyed = |name: &str, v: &str| PathElem {
            name: name.into(),
            key: HashMap::from([("name".to_string(), v.to_string())]),
        };
        for i in &self.dev.interfaces {
            let inn = i.in_octets + i.in_rate * elapsed;
            let outn = i.out_octets + i.out_rate * elapsed;
            match self.dev.model {
                Model::OpenConfig => {
                    let base = || vec![e("interfaces"), keyed("interface", &i.name), e("state")];
                    let with = |rest: &[&str]| {
                        let mut v = base();
                        v.extend(rest.iter().map(|n| e(n)));
                        v
                    };
                    out.push((with(&["name"]), Leaf::Text(i.name.clone())));
                    out.push((with(&["description"]), Leaf::Text(i.description.clone())));
                    let status = if i.up { "UP" } else { "DOWN" };
                    out.push((with(&["oper-status"]), Leaf::Text(status.into())));
                    out.push((with(&["ifindex"]), Leaf::Uint(i.if_index.into())));
                    out.push((with(&["counters", "in-octets"]), Leaf::Uint(inn)));
                    out.push((with(&["counters", "out-octets"]), Leaf::Uint(outn)));
                    let speed = format!(
                        "openconfig-if-ethernet:SPEED_{}GB",
                        i.speed_bps / 1_000_000_000
                    );
                    let mut eth = vec![e("interfaces"), keyed("interface", &i.name)];
                    eth.extend([e("ethernet"), e("state"), e("port-speed")]);
                    out.push((eth, Leaf::Text(speed)));
                }
                Model::Srl => {
                    let with = |rest: &[&str]| {
                        let mut v = vec![keyed("interface", &i.name)];
                        v.extend(rest.iter().map(|n| e(n)));
                        v
                    };
                    out.push((with(&["name"]), Leaf::Text(i.name.clone())));
                    out.push((with(&["description"]), Leaf::Text(i.description.clone())));
                    let status = if i.up { "up" } else { "down" };
                    out.push((with(&["oper-state"]), Leaf::Text(status.into())));
                    out.push((with(&["ifindex"]), Leaf::Uint(i.if_index.into())));
                    out.push((with(&["statistics", "in-octets"]), Leaf::Uint(inn)));
                    out.push((with(&["statistics", "out-octets"]), Leaf::Uint(outn)));
                    let speed = format!("{}G", i.speed_bps / 1_000_000_000);
                    out.push((with(&["ethernet", "port-speed"]), Leaf::Text(speed)));
                }
            }
        }
        match self.dev.model {
            Model::OpenConfig => {
                out.push((
                    vec![e("system"), e("state"), e("hostname")],
                    Leaf::Text(self.dev.hostname.clone()),
                ));
                out.push((
                    vec![e("system"), e("state"), e("boot-time")],
                    Leaf::Uint(self.boot_ns),
                ));
            }
            Model::Srl => {
                out.push((
                    vec![e("system"), e("name"), e("host-name")],
                    Leaf::Text(self.dev.hostname.clone()),
                ));
                let secs = self.boot_ns / 1_000_000_000;
                out.push((
                    vec![e("system"), e("information"), e("last-booted")],
                    Leaf::Text(rfc3339(secs)),
                ));
            }
        }
        out
    }

    /// The updates answering `paths`, or `NOT_FOUND` for the first path the device lacks.
    fn answer(
        &self,
        prefix: Option<&Path>,
        paths: &[Path],
        encoding: i32,
    ) -> Result<Vec<Update>, Status> {
        let leaves = self.leaves();
        let named = prefix.map(|p| p.target.as_str()).unwrap_or("");
        if let Some(t) = &self.dev.target
            && named != t
        {
            return Err(Status::not_found(format!("unknown target '{named}'")));
        }
        let mut updates = Vec::new();
        for p in paths {
            let origin = if p.origin.is_empty() {
                prefix.map(|x| x.origin.as_str()).unwrap_or("")
            } else {
                p.origin.as_str()
            };
            match (self.dev.origins, self.dev.model, origin) {
                (Origins::Refused, _, o) if !o.is_empty() => {
                    return Err(Status::invalid_argument(format!("unknown origin '{o}'")));
                }
                (Origins::OpenConfigNeedsIt, Model::OpenConfig, o) if o != "openconfig" => {
                    return Err(Status::invalid_argument(
                        "Path not valid - unknown element 'interfaces'",
                    ));
                }
                _ => {}
            }
            let mut want: Vec<PathElem> = prefix.map(|x| x.elem.clone()).unwrap_or_default();
            want.extend(p.elem.iter().cloned());
            let found: Vec<&(Vec<PathElem>, Leaf)> =
                leaves.iter().filter(|(l, _)| covers(&want, l)).collect();
            if found.is_empty() && self.dev.missing_is_empty {
                continue;
            }
            if found.is_empty() {
                return Err(Status::not_found(format!("path {} not found", text(&want))));
            }
            for (leaf_path, leaf) in found {
                if self.dev.containers && leaf_path.len() == want.len() && leaf_path.len() > 1 {
                    // The parent container, as one JSON object holding the leaf.
                    let (parent, last) = leaf_path.split_at(leaf_path.len() - 1);
                    let obj =
                        serde_json::json!({ format!("mock:{}", last[0].name): leaf.json(true) });
                    updates.push(Update {
                        path: Some(Path {
                            elem: parent.to_vec(),
                            ..Default::default()
                        }),
                        val: Some(TypedValue {
                            value: Some(typed_value::Value::JsonIetfVal(
                                obj.to_string().into_bytes(),
                            )),
                        }),
                        ..Default::default()
                    });
                } else {
                    updates.push(Update {
                        path: Some(Path {
                            elem: leaf_path.clone(),
                            ..Default::default()
                        }),
                        val: Some(leaf.typed(encoding)),
                        ..Default::default()
                    });
                }
            }
        }
        Ok(updates)
    }
}

#[derive(Debug, Clone)]
enum Leaf {
    Uint(u64),
    Text(String),
}

impl Leaf {
    fn json(&self, ietf: bool) -> serde_json::Value {
        match self {
            // RFC 7951: 64-bit integers are strings in JSON_IETF.
            Leaf::Uint(u) if ietf => serde_json::Value::String(u.to_string()),
            Leaf::Uint(u) => serde_json::Value::from(*u),
            Leaf::Text(t) => serde_json::Value::String(t.clone()),
        }
    }

    fn typed(&self, encoding: i32) -> TypedValue {
        let value = if encoding == Encoding::Proto as i32 {
            match self {
                Leaf::Uint(u) => typed_value::Value::UintVal(*u),
                Leaf::Text(t) => typed_value::Value::StringVal(t.clone()),
            }
        } else if encoding == Encoding::Json as i32 {
            typed_value::Value::JsonVal(self.json(false).to_string().into_bytes())
        } else {
            typed_value::Value::JsonIetfVal(self.json(true).to_string().into_bytes())
        };
        TypedValue { value: Some(value) }
    }
}

fn bare(n: &str) -> &str {
    n.rsplit_once(':').map_or(n, |(_, b)| b)
}

/// True when the requested path `want` is `leaf` or one of its containers; a key `*` matches any.
fn covers(want: &[PathElem], leaf: &[PathElem]) -> bool {
    want.len() <= leaf.len()
        && want.iter().zip(leaf).all(|(w, l)| {
            bare(&w.name) == l.name
                && w.key
                    .iter()
                    .all(|(k, v)| v == "*" || l.key.get(bare(k)) == Some(v))
        })
}

fn text(p: &[PathElem]) -> String {
    p.iter()
        .map(|e| {
            let mut s = e.name.clone();
            for (k, v) in &e.key {
                s.push_str(&format!("[{k}={v}]"));
            }
            s
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DDTHH:MM:SS.000Z` for seconds since 1970 (civil_from_days).
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

type SubStream = Pin<Box<dyn Stream<Item = Result<SubscribeResponse, Status>> + Send>>;

#[tonic::async_trait]
impl GNmi for Mock {
    async fn capabilities(
        &self,
        req: Request<CapabilityRequest>,
    ) -> Result<Response<CapabilityResponse>, Status> {
        self.check_login(&req)?;
        let model = match self.dev.model {
            Model::OpenConfig => "openconfig-interfaces",
            Model::Srl => "urn:nokia.com:srlinux:chassis:interfaces",
        };
        Ok(Response::new(CapabilityResponse {
            supported_models: vec![ModelData {
                name: model.into(),
                organization: "LinkScope mock".into(),
                version: "1".into(),
            }],
            supported_encodings: self.dev.encodings.iter().map(|e| *e as i32).collect(),
            g_nmi_version: "0.10.0".into(),
            ..Default::default()
        }))
    }

    async fn get(&self, req: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        self.check_login(&req)?;
        if !self.dev.get {
            return Err(Status::unimplemented("Get is not supported; use Subscribe"));
        }
        let r = req.get_ref();
        {
            let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            s.gets += 1;
            s.encodings.push(r.encoding);
        }
        let updates = self.answer(r.prefix.as_ref(), &r.path, r.encoding)?;
        Ok(Response::new(GetResponse {
            notification: vec![Notification {
                timestamp: now_ns(),
                update: updates,
                ..Default::default()
            }],
            ..Default::default()
        }))
    }

    async fn set(&self, _req: Request<SetRequest>) -> Result<Response<SetResponse>, Status> {
        Err(Status::permission_denied("this mock is read-only"))
    }

    type SubscribeStream = SubStream;

    async fn subscribe(
        &self,
        req: Request<Streaming<SubscribeRequest>>,
    ) -> Result<Response<SubStream>, Status> {
        self.check_login(&req)?;
        let mut stream = req.into_inner();
        let first = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("no subscription"))?;
        let Some(subscribe_request::Request::Subscribe(list)) = first.request else {
            return Err(Status::invalid_argument(
                "the first message must be a subscription",
            ));
        };
        if list.mode != subscription_list::Mode::Once as i32 {
            return Err(Status::unimplemented(
                "this mock answers ONCE subscriptions only",
            ));
        }
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribes += 1;
        let paths: Vec<Path> = list
            .subscription
            .iter()
            .filter_map(|s| s.path.clone())
            .collect();
        let updates = self.answer(list.prefix.as_ref(), &paths, list.encoding)?;
        let msgs = vec![
            Ok(SubscribeResponse {
                response: Some(subscribe_response::Response::Update(Notification {
                    timestamp: now_ns(),
                    update: updates,
                    ..Default::default()
                })),
                ..Default::default()
            }),
            Ok(SubscribeResponse {
                response: Some(subscribe_response::Response::SyncResponse(true)),
                ..Default::default()
            }),
        ];
        Ok(Response::new(Box::pin(tokio_stream::iter(msgs))))
    }
}

/// Serve `dev` on `listener`, with TLS when `identity` (certificate and key, PEM) is given.
/// Returns when the listener fails.
pub async fn serve(
    dev: Device,
    listener: tokio::net::TcpListener,
    identity: Option<(Vec<u8>, Vec<u8>)>,
) -> Result<(), tonic::transport::Error> {
    let mut b = tonic::transport::Server::builder();
    if let Some((cert, key)) = identity {
        b = b.tls_config(
            tonic::transport::ServerTlsConfig::new()
                .identity(tonic::transport::Identity::from_pem(cert, key)),
        )?;
    }
    b.add_service(Mock::new(dev).into_server())
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(1_792_648_800), "2026-10-22T06:00:00.000Z");
    }

    #[test]
    fn wildcards_and_containers_cover_leaves() {
        let m = Mock::new(Device::new(
            Model::Srl,
            "srl",
            vec![
                Interface::new("ethernet-1/1", 1, 5, 6),
                Interface::new("ethernet-1/2", 2, 7, 8),
            ],
        ));
        let p = |s: &str| Path {
            elem: s
                .split('/')
                .map(|e| match e.split_once("[name=") {
                    Some((n, v)) => PathElem {
                        name: n.into(),
                        key: HashMap::from([("name".into(), v.trim_end_matches(']').into())]),
                    },
                    None => PathElem {
                        name: e.into(),
                        key: HashMap::new(),
                    },
                })
                .collect(),
            ..Default::default()
        };
        let all = m
            .answer(None, &[p("interface[name=*]/statistics")], 4)
            .unwrap();
        assert_eq!(all.len(), 4);
        assert!(
            m.answer(None, &[p("interface[name=ethernet-1/9]/ifindex")], 4)
                .is_err()
        );
    }
}
