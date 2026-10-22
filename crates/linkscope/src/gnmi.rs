// SPDX-License-Identifier: GPL-2.0-or-later
//! Reading interface counters over gNMI, a LinkScope addition beside SNMP.
//!
//! Each cycle, LinkScope sends one gNMI `Get` per device for the in and out octet counters of
//! every interface its targets name, and one more for the device's name, boot time and the
//! interfaces' ifIndex (those may fail without failing the targets). A device that does not
//! implement `Get` is read with a `Subscribe` in `ONCE` mode instead. Connections are kept
//! between cycles; a broken one is opened again at the next cycle.
//!
//! TLS is on by default and checks the device's certificate. The password is read from its file
//! or environment variable at each cycle, so a new password needs no restart, and it never
//! appears in a message, a label or the log.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use linkscope_config::{GnmiModel, GnmiOptions, GnmiSource, TlsMode};
use linkscope_gnmi::g_nmi_client::GNmiClient;
use linkscope_gnmi::{
    CapabilityRequest, Encoding, GetRequest, Notification, Path, PathElem, SubscribeRequest,
    Subscription, SubscriptionList, SubscriptionMode, TypedValue, get_request, subscribe_request,
    subscribe_response, subscription_list, typed_value,
};
use tonic::transport::{Channel, Endpoint, Uri};

use crate::poll::{SourceResult, now};

/// Largest answer accepted from a device (a discovery of a big chassis is a few MB).
const MAX_MESSAGE: usize = 64 << 20;

/// One device: where it is and how to read it. Targets with the same key share a connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey {
    pub host: String,
    pub port: u16,
    pub options: GnmiOptions,
}

impl DeviceKey {
    pub fn of(s: &GnmiSource) -> DeviceKey {
        DeviceKey {
            host: s.host.clone(),
            port: s.port,
            options: s.options.clone(),
        }
    }

    /// `host:port`, for messages.
    pub fn display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Clone)]
struct Conn {
    client: GNmiClient<Channel>,
    encoding: Encoding,
    /// The gNMI `target` to name in each request (`target=`, for gateways), or empty.
    target: String,
    /// The encodings the device offered in `Capabilities`.
    offered: Vec<i32>,
    /// For OpenConfig paths: whether the origin `openconfig` is sent (unknown until it is tried).
    oc_origin: Option<bool>,
    /// False once the device answered `Get` with "unimplemented".
    use_get: bool,
}

/// Connections kept between cycles, one per device.
#[derive(Default, Clone)]
pub struct GnmiPool {
    conns: Arc<std::sync::Mutex<HashMap<DeviceKey, Conn>>>,
}

impl GnmiPool {
    fn cached(&self, key: &DeviceKey) -> Option<Conn> {
        self.conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    fn store(&self, key: &DeviceKey, c: Conn) {
        self.conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone(), c);
    }

    pub fn forget(&self, key: &DeviceKey) {
        self.conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }
}

// ---------------------------------------------------------------- paths

/// A gNMI path written as text: `interfaces/interface[name=ethernet-1/1]/state/counters`.
/// A `/` inside brackets belongs to the key value.
pub fn path(text: &str) -> Path {
    let mut elem = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for ch in text.trim_start_matches('/').chars() {
        match ch {
            '[' => {
                depth += 1;
                cur.push(ch);
            }
            ']' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            '/' if depth == 0 => {
                elem.push(path_elem(&cur));
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        elem.push(path_elem(&cur));
    }
    Path {
        elem,
        ..Default::default()
    }
}

fn path_elem(text: &str) -> PathElem {
    let (name, rest) = match text.find('[') {
        Some(i) => (&text[..i], &text[i..]),
        None => (text, ""),
    };
    let mut key = HashMap::new();
    let mut rest = rest;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let kv = &rest[start + 1..start + end];
        if let Some((k, v)) = kv.split_once('=') {
            key.insert(k.to_string(), v.to_string());
        }
        rest = &rest[start + end + 1..];
    }
    PathElem {
        name: name.to_string(),
        key,
    }
}

/// A path's elements without module prefixes (`openconfig-interfaces:interfaces` is
/// `interfaces`), so the requested path and the device's answer compare equal.
fn bare(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, n)| n)
}

/// The text form used to match answers to requests: names without module prefixes, keys sorted.
fn canonical(elems: &[PathElem]) -> String {
    let mut out = String::new();
    for e in elems {
        out.push('/');
        out.push_str(bare(&e.name));
        let mut keys: Vec<_> = e.key.iter().collect();
        keys.sort();
        for (k, v) in keys {
            out.push('[');
            out.push_str(bare(k));
            out.push('=');
            out.push_str(v);
            out.push(']');
        }
    }
    out
}

/// What LinkScope reads for one interface, by model.
struct InterfacePaths {
    input: String,
    output: String,
    if_index: String,
}

fn interface_paths(model: GnmiModel, ifname: &str) -> InterfacePaths {
    match model {
        GnmiModel::OpenConfig => {
            let base = format!("interfaces/interface[name={ifname}]/state");
            InterfacePaths {
                input: format!("{base}/counters/in-octets"),
                output: format!("{base}/counters/out-octets"),
                if_index: format!("{base}/ifindex"),
            }
        }
        GnmiModel::Srl => {
            let base = format!("interface[name={ifname}]");
            InterfacePaths {
                input: format!("{base}/statistics/in-octets"),
                output: format!("{base}/statistics/out-octets"),
                if_index: format!("{base}/ifindex"),
            }
        }
    }
}

/// The device's host name and boot time, by model.
fn system_paths(model: GnmiModel) -> [&'static str; 2] {
    match model {
        GnmiModel::OpenConfig => ["system/state/hostname", "system/state/boot-time"],
        GnmiModel::Srl => ["system/name/host-name", "system/information/last-booted"],
    }
}

/// OpenConfig paths are sent with origin `openconfig` (SR Linux, which serves its own model by
/// default, needs it); a device that refuses the origin is asked again without one, and the
/// answer is remembered per connection. A vendor model has no origin.
const OPENCONFIG_ORIGIN: &str = "openconfig";

// ---------------------------------------------------------------- values

/// What a device answered, by canonical path.
#[derive(Debug, Default)]
pub struct Answers {
    values: HashMap<String, serde_json::Value>,
    /// Every update in the order received, with its full path (for wildcard answers).
    items: Vec<(Vec<PathElem>, serde_json::Value)>,
}

impl Answers {
    pub fn add(&mut self, n: &Notification) {
        let prefix = n
            .prefix
            .as_ref()
            .map(|p| p.elem.clone())
            .unwrap_or_default();
        for u in &n.update {
            let mut elems = prefix.clone();
            if let Some(p) = &u.path {
                elems.extend(p.elem.iter().cloned());
            }
            if let Some(v) = u.val.as_ref().and_then(json_of) {
                self.items.push((elems.clone(), v.clone()));
                // A container may come in parts (one update per leaf asked): merge them.
                match (self.values.get_mut(&canonical(&elems)), v) {
                    (Some(serde_json::Value::Object(have)), serde_json::Value::Object(more)) => {
                        have.extend(more)
                    }
                    (_, v) => {
                        self.values.insert(canonical(&elems), v);
                    }
                }
            }
        }
    }

    /// The value at `want`, also when the device answered with a container holding it
    /// (some devices answer a leaf request with its parent, as JSON).
    pub fn get(&self, want: &Path) -> Option<&serde_json::Value> {
        let key = canonical(&want.elem);
        if let Some(v) = self.values.get(&key) {
            return Some(v);
        }
        for (k, v) in &self.values {
            if let Some(rest) = key.strip_prefix(k.as_str())
                && rest.starts_with('/')
            {
                let mut cur = v;
                for name in rest.trim_start_matches('/').split('/') {
                    let name = name.split('[').next().unwrap_or(name);
                    cur = child(cur, name)?;
                }
                return Some(cur);
            }
        }
        None
    }

    /// A counter or other whole number at `want`, as decimal text.
    pub fn number(&self, want: &Path) -> Option<String> {
        match self.get(want)? {
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(|u| u.to_string())
                .or_else(|| n.as_i64().map(|i| i.to_string())),
            serde_json::Value::String(s)
                if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) =>
            {
                Some(s.clone())
            }
            _ => None,
        }
    }

    pub fn text(&self, want: &Path) -> Option<String> {
        match self.get(want)? {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

/// A member of a JSON object, with or without its module prefix.
fn child<'a>(v: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    let o = v.as_object()?;
    o.get(name)
        .or_else(|| o.iter().find(|(k, _)| bare(k) == name).map(|(_, v)| v))
}

/// A gNMI value as JSON, whatever encoding the device used.
fn json_of(v: &TypedValue) -> Option<serde_json::Value> {
    use serde_json::Value as J;
    Some(match v.value.as_ref()? {
        typed_value::Value::StringVal(s) => J::String(s.clone()),
        typed_value::Value::AsciiVal(s) => J::String(s.clone()),
        typed_value::Value::IntVal(i) => J::from(*i),
        typed_value::Value::UintVal(u) => J::from(*u),
        typed_value::Value::BoolVal(b) => J::Bool(*b),
        typed_value::Value::DoubleVal(d) => J::from(*d),
        #[allow(deprecated)]
        typed_value::Value::FloatVal(f) => J::from(*f as f64),
        typed_value::Value::JsonVal(b) | typed_value::Value::JsonIetfVal(b) => {
            serde_json::from_slice(b).ok()?
        }
        _ => return None,
    })
}

/// Uptime as MRTG shows it, from an OpenConfig boot time (nanoseconds since 1970) or an RFC 3339
/// date (SR Linux's `last-booted`).
fn uptime_from_boot(v: &serde_json::Value, now_secs: i64) -> Option<String> {
    let boot_secs = match v {
        serde_json::Value::Number(n) => (n.as_u64()? / 1_000_000_000) as i64,
        serde_json::Value::String(s) if s.bytes().all(|b| b.is_ascii_digit()) => {
            (s.parse::<u64>().ok()? / 1_000_000_000) as i64
        }
        serde_json::Value::String(s) => rfc3339_secs(s)?,
        _ => return None,
    };
    let up = now_secs.checked_sub(boot_secs).filter(|u| *u >= 0)?;
    Some(crate::snmp::pretty_uptime(up as u64 * 100))
}

/// Seconds since 1970 of `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)`.
fn rfc3339_secs(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 && (r.starts_with('+') || r.starts_with('-')) => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            sign * (r[1..3].parse::<i64>().ok()? * 3600 + r[4..6].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    // Days since 1970 (Howard Hinnant's days_from_civil).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

// ---------------------------------------------------------------- connecting

fn read_password(o: &GnmiOptions) -> Result<Option<String>, String> {
    if let Some(f) = &o.password_file {
        let text = std::fs::read_to_string(f)
            .map_err(|e| format!("cannot read its password-file= ({})", e.kind()))?;
        return Ok(Some(text.lines().next().unwrap_or("").to_string()));
    }
    if let Some(var) = &o.password_env {
        return std::env::var(var)
            .map(Some)
            .map_err(|_| format!("environment variable {var} (password-env=) is not set"));
    }
    Ok(None)
}

/// The name the device's certificate must carry.
fn server_name(key: &DeviceKey) -> Result<rustls::pki_types::ServerName<'static>, String> {
    let name = key.options.tls_name.clone().unwrap_or_else(|| {
        key.host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string()
    });
    rustls::pki_types::ServerName::try_from(name)
        .map_err(|_| "tls-name= is not a valid host name".into())
}

fn tls_config(o: &GnmiOptions) -> Result<rustls::ClientConfig, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup: {e}"))?;
    let builder = match o.tls {
        TlsMode::Insecure => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCheck(provider))),
        _ => {
            let mut roots = rustls::RootCertStore::empty();
            match &o.ca {
                Some(ca) => {
                    let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(ca)
                        .map_err(|_| "cannot read its ca= file".to_string())?
                        .collect::<Result<_, _>>()
                        .map_err(|_| "its ca= file is not PEM certificates".to_string())?;
                    let (added, _) = roots.add_parsable_certificates(certs);
                    if added == 0 {
                        return Err("its ca= file holds no usable certificate".into());
                    }
                }
                None => {
                    let found = rustls_native_certs::load_native_certs();
                    roots.add_parsable_certificates(found.certs);
                }
            }
            builder.with_root_certificates(roots)
        }
    };
    let mut cfg = match (&o.cert, &o.key) {
        (Some(cert), Some(key)) => {
            let chain: Vec<CertificateDer> = CertificateDer::pem_file_iter(cert)
                .map_err(|_| "cannot read its cert= file".to_string())?
                .collect::<Result<_, _>>()
                .map_err(|_| "its cert= file is not PEM certificates".to_string())?;
            let key = PrivateKeyDer::from_pem_file(key)
                .map_err(|_| "cannot read a private key from its key= file".to_string())?;
            builder
                .with_client_auth_cert(chain, key)
                .map_err(|e| format!("its client certificate: {e}"))?
        }
        _ => builder.with_no_client_auth(),
    };
    cfg.alpn_protocols = vec![b"h2".to_vec()];
    Ok(cfg)
}

/// `tls=insecure`: any certificate is accepted, but the handshake's signatures are still checked,
/// so the session is encrypted to whoever answered.
#[derive(Debug)]
struct NoCheck(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoCheck {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

async fn open(key: &DeviceKey, timeout: Duration) -> Result<Channel, String> {
    let addr = key.display();
    let ep = Endpoint::from_shared(format!("http://{addr}"))
        .map_err(|_| format!("{addr} is not a valid address"))?
        .connect_timeout(timeout)
        .timeout(timeout)
        .tcp_nodelay(true);
    if key.options.tls == TlsMode::Off {
        return ep.connect().await.map_err(|e| connect_error(&e));
    }
    let cfg = Arc::new(tls_config(&key.options)?);
    let name = server_name(key)?;
    let connector = tower::service_fn(move |_: Uri| {
        let cfg = cfg.clone();
        let name = name.clone();
        let addr = addr.clone();
        async move {
            let tcp = tokio::net::TcpStream::connect(&addr).await?;
            tcp.set_nodelay(true)?;
            let tls = tokio_rustls::TlsConnector::from(cfg)
                .connect(name, tcp)
                .await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(tls))
        }
    });
    ep.connect_with_connector(connector)
        .await
        .map_err(|e| connect_error(&e))
}

/// The innermost reason of a connection error ("connection refused", "invalid peer certificate:
/// UnknownIssuer"), without tonic's wrapping.
fn connect_error(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cur: &(dyn std::error::Error + 'static) = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    let msg = cur.to_string();
    if msg.contains("certificate")
        || msg.contains("UnknownIssuer")
        || msg.contains("NotValidForName")
    {
        format!("TLS: {msg}")
    } else {
        msg
    }
}

/// The encoding to ask for: JSON_IETF where offered, then JSON, then PROTO.
fn pick_encoding(offered: &[i32]) -> Encoding {
    for e in [Encoding::JsonIetf, Encoding::Json, Encoding::Proto] {
        if offered.contains(&(e as i32)) {
            return e;
        }
    }
    Encoding::JsonIetf
}

fn with_login<T>(
    msg: T,
    user: Option<&str>,
    password: Option<&str>,
) -> Result<tonic::Request<T>, String> {
    let mut req = tonic::Request::new(msg);
    if let (Some(u), Some(p)) = (user, password) {
        let md = req.metadata_mut();
        md.insert(
            "username",
            u.parse()
                .map_err(|_| "user= holds characters gNMI cannot send".to_string())?,
        );
        md.insert(
            "password",
            p.parse()
                .map_err(|_| "the password holds characters gNMI cannot send".to_string())?,
        );
    }
    Ok(req)
}

/// A device's answer to a failed request, in words.
fn status_text(s: &tonic::Status, model: GnmiModel) -> String {
    use tonic::Code;
    // The device's own message, short and on one line.
    let detail: String = s
        .message()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    match s.code() {
        Code::Unauthenticated => "login refused (wrong user or password)".into(),
        Code::PermissionDenied => format!("not allowed to read these paths: {detail}"),
        Code::NotFound | Code::InvalidArgument => format!(
            "the device does not answer these {} paths (wrong model= or interface?): {detail}",
            model.name()
        ),
        Code::Unavailable | Code::Unknown if detail.contains("certificate") => {
            format!("TLS: {detail}")
        }
        Code::Unavailable if detail.contains("OpenConfig") => format!(
            "the device's OpenConfig is not running (try model=srl, or turn OpenConfig on): {detail}"
        ),
        Code::Unavailable => format!("unavailable: {detail}"),
        Code::ResourceExhausted => format!("the device's gNMI request limit was reached: {detail}"),
        Code::DeadlineExceeded => "no answer in time".into(),
        Code::Unimplemented => format!("not supported by the device: {detail}"),
        c => format!("{c:?}: {detail}"),
    }
}

/// True when the connection itself is suspect and should be opened again next time.
fn breaks_connection(s: &tonic::Status) -> bool {
    matches!(
        s.code(),
        tonic::Code::Unavailable
            | tonic::Code::Unknown
            | tonic::Code::Internal
            | tonic::Code::Cancelled
    )
}

// ---------------------------------------------------------------- reading

impl Conn {
    async fn fetch(
        &mut self,
        paths: &[Path],
        model: GnmiModel,
        login: (Option<&str>, Option<&str>),
    ) -> Result<Answers, tonic::Status> {
        if model != GnmiModel::OpenConfig {
            return self.fetch_as(paths, "", login).await;
        }
        match self.oc_origin {
            Some(true) => self.fetch_as(paths, OPENCONFIG_ORIGIN, login).await,
            Some(false) => self.fetch_as(paths, "", login).await,
            None => match self.fetch_as(paths, OPENCONFIG_ORIGIN, login).await {
                Ok(a) => {
                    self.oc_origin = Some(true);
                    Ok(a)
                }
                Err(first)
                    if matches!(
                        first.code(),
                        tonic::Code::InvalidArgument | tonic::Code::NotFound
                    ) =>
                {
                    match self.fetch_as(paths, "", login).await {
                        Ok(a) => {
                            self.oc_origin = Some(false);
                            Ok(a)
                        }
                        Err(_) => Err(first),
                    }
                }
                Err(e) => Err(e),
            },
        }
    }

    /// One request for `paths`, each with `origin` (empty for none).
    async fn fetch_as(
        &mut self,
        paths: &[Path],
        origin: &str,
        login: (Option<&str>, Option<&str>),
    ) -> Result<Answers, tonic::Status> {
        let paths: Vec<Path> = paths
            .iter()
            .map(|p| Path {
                origin: origin.to_string(),
                ..p.clone()
            })
            .collect();
        let prefix = (!self.target.is_empty()).then(|| Path {
            target: self.target.clone(),
            ..Default::default()
        });
        if self.use_get {
            let msg = GetRequest {
                prefix: prefix.clone(),
                path: paths.clone(),
                r#type: get_request::DataType::All as i32,
                encoding: self.encoding as i32,
                ..Default::default()
            };
            let req = with_login(msg, login.0, login.1).map_err(tonic::Status::invalid_argument)?;
            match self.client.get(req).await {
                Ok(r) => {
                    let mut a = Answers::default();
                    for n in &r.get_ref().notification {
                        a.add(n);
                    }
                    return Ok(a);
                }
                Err(s) if s.code() == tonic::Code::Unimplemented => self.use_get = false,
                Err(s) => return Err(s),
            }
        }
        self.subscribe_once(&paths, prefix, login).await
    }

    async fn subscribe_once(
        &mut self,
        paths: &[Path],
        prefix: Option<Path>,
        login: (Option<&str>, Option<&str>),
    ) -> Result<Answers, tonic::Status> {
        let list = SubscriptionList {
            prefix,
            subscription: paths
                .iter()
                .map(|p| Subscription {
                    path: Some(p.clone()),
                    mode: SubscriptionMode::TargetDefined as i32,
                    ..Default::default()
                })
                .collect(),
            mode: subscription_list::Mode::Once as i32,
            encoding: self.encoding as i32,
            ..Default::default()
        };
        // Keep the request stream open until the answer is complete: some devices end the
        // subscription when the client closes its side.
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(SubscribeRequest {
            request: Some(subscribe_request::Request::Subscribe(list)),
            ..Default::default()
        })
        .await
        .map_err(|_| tonic::Status::internal("subscribe request not sent"))?;
        let req = with_login(
            tokio_stream::wrappers::ReceiverStream::new(rx),
            login.0,
            login.1,
        )
        .map_err(tonic::Status::invalid_argument)?;
        let mut stream = self.client.subscribe(req).await?.into_inner();
        let mut a = Answers::default();
        while let Some(m) = stream.message().await? {
            match m.response {
                Some(subscribe_response::Response::Update(n)) => a.add(&n),
                Some(subscribe_response::Response::SyncResponse(true)) => break,
                _ => {}
            }
        }
        drop(tx);
        Ok(a)
    }
}

async fn connection(
    pool: &GnmiPool,
    key: &DeviceKey,
    timeout: Duration,
    login: (Option<&str>, Option<&str>),
) -> Result<Conn, String> {
    if let Some(c) = pool.cached(key) {
        return Ok(c);
    }
    let channel = open(key, timeout).await?;
    let mut client = GNmiClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE)
        .max_encoding_message_size(MAX_MESSAGE);
    // Capabilities tells which encodings the device speaks. A device that refuses it is asked
    // for JSON_IETF, the encoding gNMI devices most commonly support.
    let mut offered = Vec::new();
    let encoding = match client
        .capabilities(with_login(CapabilityRequest::default(), login.0, login.1)?)
        .await
    {
        Ok(r) => {
            offered = r.get_ref().supported_encodings.clone();
            pick_encoding(&offered)
        }
        Err(s) if s.code() == tonic::Code::Unauthenticated => {
            return Err(status_text(&s, key.options.model));
        }
        Err(s) if breaks_connection(&s) => return Err(status_text(&s, key.options.model)),
        Err(_) => Encoding::JsonIetf,
    };
    let c = Conn {
        client,
        encoding,
        target: key.options.target.clone().unwrap_or_default(),
        offered,
        oc_origin: None,
        use_get: true,
    };
    pool.store(key, c.clone());
    Ok(c)
}

/// Read every interface of one device for this cycle.
pub async fn poll_device(
    pool: &GnmiPool,
    key: DeviceKey,
    members: Vec<(usize, GnmiSource)>,
    timeout: Duration,
) -> Vec<(usize, SourceResult)> {
    let started = now();
    let model = key.options.model;
    let fail = |msg: String| -> Vec<(usize, SourceResult)> {
        members
            .iter()
            .map(|(i, _)| {
                (
                    *i,
                    SourceResult {
                        error: Some(msg.clone()),
                        time: started,
                        ..Default::default()
                    },
                )
            })
            .collect()
    };
    let who = key.display();
    let password = match read_password(&key.options) {
        Ok(p) => p,
        Err(e) => return fail(format!("gNMI {who}: {e}")),
    };
    let login = (key.options.user.as_deref(), password.as_deref());
    let mut conn = match connection(pool, &key, timeout, login).await {
        Ok(c) => c,
        Err(e) => return fail(format!("gNMI {who}: {e}")),
    };

    // The counters: one request for every interface, or one per interface when the device
    // refuses the whole request (an unknown interface makes most devices fail all of it).
    let mut ifnames: Vec<String> = members.iter().map(|(_, s)| s.interface.clone()).collect();
    ifnames.sort();
    ifnames.dedup();
    let counter_paths = |names: &[String]| -> Vec<Path> {
        names
            .iter()
            .flat_map(|n| {
                let p = interface_paths(model, n);
                [path(&p.input), path(&p.output)]
            })
            .collect()
    };
    let mut answers = Answers::default();
    let mut per_if_error: HashMap<String, String> = HashMap::new();
    match conn.fetch(&counter_paths(&ifnames), model, login).await {
        Ok(a) => answers = a,
        Err(s) if breaks_connection(&s) || s.code() == tonic::Code::Unauthenticated => {
            pool.forget(&key);
            return fail(format!("gNMI {who}: {}", status_text(&s, model)));
        }
        Err(s) if ifnames.len() == 1 => {
            per_if_error.insert(ifnames[0].clone(), status_text(&s, model));
        }
        Err(s) => {
            // One request per interface finds the unknown one; past this many, the device's request
            // limit (SR Linux allows 60 a minute) matters more than naming it.
            const MAX_SPLIT: usize = 24;
            for n in ifnames.iter().skip(MAX_SPLIT) {
                per_if_error.insert(n.clone(), status_text(&s, model));
            }
            for n in ifnames.iter().take(MAX_SPLIT) {
                match conn
                    .fetch(&counter_paths(std::slice::from_ref(n)), model, login)
                    .await
                {
                    Ok(a) => {
                        answers.values.extend(a.values);
                        answers.items.extend(a.items);
                    }
                    Err(s) => {
                        per_if_error.insert(n.clone(), status_text(&s, model));
                    }
                }
            }
        }
    }
    let time = now();

    // Name, uptime and ifIndex: nice to have, never a reason to fail a target.
    let [host_path, boot_path] = system_paths(model).map(path);
    let mut extra_paths = vec![host_path.clone(), boot_path.clone()];
    extra_paths.extend(
        ifnames
            .iter()
            .map(|n| path(&interface_paths(model, n).if_index)),
    );
    let extra = match conn.fetch(&extra_paths, model, login).await {
        Ok(a) => a,
        Err(_) => conn
            .fetch(&[host_path.clone(), boot_path.clone()], model, login)
            .await
            .unwrap_or_default(),
    };
    let name = extra
        .text(&host_path)
        .map(|n| n.chars().take(crate::poll::MAX_DEVICE_TEXT).collect());
    let uptime = extra
        .get(&boot_path)
        .and_then(|v| uptime_from_boot(v, time));
    pool.store(&key, conn);

    members
        .into_iter()
        .map(|(i, s)| {
            let p = interface_paths(model, &s.interface);
            let mut r = SourceResult {
                time,
                uptime: uptime.clone(),
                name: name.clone(),
                ..Default::default()
            };
            if let Some(e) = per_if_error.get(&s.interface) {
                r.error = Some(format!("gNMI {who} {}: {e}", s.interface));
                return (i, r);
            }
            r.input = answers.number(&path(&p.input));
            r.output = answers.number(&path(&p.output));
            if r.input.is_none() || r.output.is_none() {
                r.error = Some(format!(
                    "gNMI {who} {}: no {} counter in the answer (wrong model= or interface name?)",
                    s.interface,
                    model.name()
                ));
            }
            let idx = extra.number(&path(&p.if_index));
            r.if_index = [idx.clone(), idx];
            (i, r)
        })
        .collect()
}

// ---------------------------------------------------------------- discovery

/// One interface found on a device.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FoundInterface {
    pub name: String,
    pub description: Option<String>,
    /// Operational state: up, down, or not reported.
    pub up: Option<bool>,
    pub speed_bps: Option<u64>,
    pub if_index: Option<String>,
}

/// What `cfgmaker --gnmi` learned about a device.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub model: GnmiModel,
    pub hostname: Option<String>,
    pub interfaces: Vec<FoundInterface>,
}

/// A port speed as the models write it: OpenConfig `openconfig-if-ethernet:SPEED_10GB`,
/// `SPEED_2500MB`; SR Linux `10G`, `100M`.
pub fn speed_bps(text: &str) -> Option<u64> {
    let t = bare(text).trim().to_ascii_uppercase();
    let t = t.strip_prefix("SPEED_").unwrap_or(&t);
    let t = t.strip_suffix('B').unwrap_or(t);
    let (num, mult) = if let Some(n) = t.strip_suffix('T') {
        (n, 1_000_000_000_000)
    } else if let Some(n) = t.strip_suffix('G') {
        (n, 1_000_000_000)
    } else if let Some(n) = t.strip_suffix('M') {
        (n, 1_000_000)
    } else if let Some(n) = t.strip_suffix('K') {
        (n, 1_000)
    } else {
        return None;
    };
    num.parse::<u64>().ok()?.checked_mul(mult)
}

/// The paths discovery reads, by model: the first lists the interfaces (it must answer), the
/// others add what they can.
fn discovery_paths(model: GnmiModel) -> (&'static str, [&'static str; 3]) {
    match model {
        GnmiModel::OpenConfig => (
            "interfaces/interface[name=*]/state/oper-status",
            [
                "interfaces/interface[name=*]/state/description",
                "interfaces/interface[name=*]/state/ifindex",
                "interfaces/interface[name=*]/ethernet/state/port-speed",
            ],
        ),
        GnmiModel::Srl => (
            "interface[name=*]/oper-state",
            [
                "interface[name=*]/description",
                "interface[name=*]/ifindex",
                "interface[name=*]/ethernet/port-speed",
            ],
        ),
    }
}

/// Every scalar of an answer as (interface name, leaf name, value), whatever shape it came in:
/// one leaf per update (PROTO), a container as one JSON object, or the whole interface list as
/// one JSON array under `interfaces` (SR Linux's wildcard answer in JSON_IETF).
fn interface_leaves(a: &Answers) -> Vec<(String, String, serde_json::Value)> {
    type Out = Vec<(String, String, serde_json::Value)>;
    fn walk(name: Option<&str>, leaf: &str, v: &serde_json::Value, out: &mut Out) {
        match v {
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    let k = bare(k);
                    if k == "interface" && v.is_array() {
                        for item in v.as_array().into_iter().flatten() {
                            let n = item
                                .get("name")
                                .or_else(|| item.get("config").and_then(|c| c.get("name")))
                                .and_then(|n| n.as_str());
                            if let Some(n) = n {
                                walk(Some(n), "", item, out);
                            }
                        }
                    } else {
                        walk(name, k, v, out);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(name, leaf, item, out);
                }
            }
            v => {
                if let Some(n) = name {
                    out.push((n.to_string(), leaf.to_string(), v.clone()));
                }
            }
        }
    }
    let mut out = Vec::new();
    for (elems, v) in &a.items {
        let name = elems
            .iter()
            .find(|e| bare(&e.name) == "interface")
            .and_then(|e| {
                e.key
                    .iter()
                    .find(|(k, _)| bare(k) == "name")
                    .map(|(_, v)| v.as_str())
            });
        let leaf = elems.last().map(|e| bare(&e.name)).unwrap_or("");
        walk(name, leaf, v, &mut out);
    }
    out
}

async fn discover_model(
    conn: &mut Conn,
    model: GnmiModel,
    login: (Option<&str>, Option<&str>),
) -> Result<Vec<FoundInterface>, tonic::Status> {
    let (list, more) = discovery_paths(model);
    let mut found: Vec<FoundInterface> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut take = |a: &Answers, found: &mut Vec<FoundInterface>, create: bool| {
        for (name, leaf, v) in interface_leaves(a) {
            let i = match index.get(&name) {
                Some(&i) => i,
                None if create => {
                    found.push(FoundInterface {
                        name: name.clone(),
                        ..Default::default()
                    });
                    index.insert(name.clone(), found.len() - 1);
                    found.len() - 1
                }
                None => continue,
            };
            let f = &mut found[i];
            let text = match &v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            match leaf.as_str() {
                "oper-status" | "oper-state" => f.up = Some(text.eq_ignore_ascii_case("up")),
                "description" => f.description = Some(text),
                "ifindex" => f.if_index = Some(text),
                "port-speed" => f.speed_bps = speed_bps(&text),
                _ => {}
            }
        }
    };
    let a = conn.fetch(&[path(list)], model, login).await?;
    take(&a, &mut found, true);
    for p in more {
        if let Ok(a) = conn.fetch(&[path(p)], model, login).await {
            take(&a, &mut found, false);
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}

/// List a device's interfaces. With `model=` in the options, that model is used; otherwise
/// OpenConfig is tried first, then SR Linux's model.
pub async fn discover(
    key: &DeviceKey,
    model_given: bool,
    timeout: Duration,
) -> Result<Discovered, String> {
    let who = key.display();
    let password = read_password(&key.options).map_err(|e| format!("gNMI {who}: {e}"))?;
    let login = (key.options.user.as_deref(), password.as_deref());
    let pool = GnmiPool::default();
    let mut conn = connection(&pool, key, timeout, login)
        .await
        .map_err(|e| format!("gNMI {who}: {e}"))?;
    // PROTO answers a wildcard with one update per leaf; JSON may nest the whole list in one.
    if conn.offered.contains(&(Encoding::Proto as i32)) {
        conn.encoding = Encoding::Proto;
    }
    let models: Vec<GnmiModel> = if model_given {
        vec![key.options.model]
    } else {
        vec![GnmiModel::OpenConfig, GnmiModel::Srl]
    };
    let mut tried = Vec::new();
    for model in models {
        match discover_model(&mut conn, model, login).await {
            Ok(found) if !found.is_empty() => {
                let [host_path, _] = system_paths(model).map(path);
                let hostname = conn
                    .fetch(std::slice::from_ref(&host_path), model, login)
                    .await
                    .ok()
                    .and_then(|a| a.text(&host_path));
                return Ok(Discovered {
                    model,
                    hostname,
                    interfaces: found,
                });
            }
            Ok(_) => tried.push(format!("{}: no interfaces", model.name())),
            // A device whose OpenConfig is off answers UNAVAILABLE: try the next model.
            Err(s) if s.code() == tonic::Code::Unauthenticated => {
                return Err(format!("gNMI {who}: {}", status_text(&s, model)));
            }
            Err(s) => tried.push(format!("{}: {}", model.name(), status_text(&s, model))),
        }
    }
    Err(format!(
        "gNMI {who} answers none of LinkScope's interface models ({})",
        tried.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use linkscope_gnmi::Update;

    #[test]
    fn a_wildcard_answered_as_one_json_list_is_read() {
        // SR Linux's JSON_IETF answer to openconfig:/interfaces/interface[name=*]/state.
        let mut a = Answers::default();
        a.add(&Notification {
            update: vec![Update {
                path: Some(path("openconfig-interfaces:interfaces")),
                val: Some(TypedValue {
                    value: Some(typed_value::Value::JsonIetfVal(
                        br#"{"interface":[{"name":"ethernet-1/1","state":{"oper-status":"UP","ifindex":32768}},
                                          {"name":"mgmt0","state":{"oper-status":"DOWN"}}]}"#
                            .to_vec(),
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        let l = interface_leaves(&a);
        assert!(l.contains(&(
            "ethernet-1/1".into(),
            "oper-status".into(),
            serde_json::json!("UP")
        )));
        assert!(l.contains(&(
            "ethernet-1/1".into(),
            "ifindex".into(),
            serde_json::json!(32768)
        )));
        assert!(l.contains(&(
            "mgmt0".into(),
            "oper-status".into(),
            serde_json::json!("DOWN")
        )));
    }

    #[test]
    fn speeds() {
        assert_eq!(
            speed_bps("openconfig-if-ethernet:SPEED_10GB"),
            Some(10_000_000_000)
        );
        assert_eq!(speed_bps("SPEED_2500MB"), Some(2_500_000_000));
        assert_eq!(speed_bps("100G"), Some(100_000_000_000));
        assert_eq!(speed_bps("100M"), Some(100_000_000));
        assert_eq!(speed_bps("SPEED_UNKNOWN"), None);
        assert_eq!(speed_bps("auto"), None);
    }

    #[test]
    fn paths_keep_slashes_inside_keys() {
        let p = path("/interfaces/interface[name=ethernet-1/1]/state/counters/in-octets");
        assert_eq!(p.elem.len(), 5);
        assert_eq!(p.elem[1].name, "interface");
        assert_eq!(p.elem[1].key["name"], "ethernet-1/1");
        assert_eq!(
            canonical(&p.elem),
            "/interfaces/interface[name=ethernet-1/1]/state/counters/in-octets"
        );
    }

    fn update(p: &str, v: typed_value::Value) -> Update {
        Update {
            path: Some(path(p)),
            val: Some(TypedValue { value: Some(v) }),
            ..Default::default()
        }
    }

    #[test]
    fn answers_in_every_encoding() {
        let mut a = Answers::default();
        a.add(&Notification {
            prefix: Some(path("openconfig-interfaces:interfaces/interface[name=e1]")),
            update: vec![
                // JSON_IETF: uint64 as a string (RFC 7951).
                update(
                    "state/counters/in-octets",
                    typed_value::Value::JsonIetfVal(b"\"18446744073709551615\"".to_vec()),
                ),
                // PROTO.
                update("state/counters/out-octets", typed_value::Value::UintVal(42)),
            ],
            ..Default::default()
        });
        // A container answered as JSON for a leaf request.
        a.add(&Notification {
            update: vec![update(
                "interface[name=e2]/statistics",
                typed_value::Value::JsonIetfVal(
                    br#"{"srl_nokia-interfaces:in-octets":"7","out-octets":8}"#.to_vec(),
                ),
            )],
            ..Default::default()
        });
        let n = |p: &str| a.number(&path(p));
        assert_eq!(
            n("interfaces/interface[name=e1]/state/counters/in-octets").as_deref(),
            Some("18446744073709551615")
        );
        assert_eq!(
            n("interfaces/interface[name=e1]/state/counters/out-octets").as_deref(),
            Some("42")
        );
        assert_eq!(
            n("interface[name=e2]/statistics/in-octets").as_deref(),
            Some("7")
        );
        assert_eq!(
            n("interface[name=e2]/statistics/out-octets").as_deref(),
            Some("8")
        );
        assert_eq!(n("interface[name=e3]/statistics/in-octets"), None);
    }

    #[test]
    fn negative_or_fractional_counters_are_refused() {
        let mut a = Answers::default();
        a.add(&Notification {
            update: vec![
                update("c/a", typed_value::Value::JsonVal(b"\"-5\"".to_vec())),
                update("c/b", typed_value::Value::JsonVal(b"1.5".to_vec())),
            ],
            ..Default::default()
        });
        assert_eq!(a.number(&path("c/a")), None);
        assert_eq!(a.number(&path("c/b")), None);
    }

    #[test]
    fn boot_times_become_uptime() {
        let now = 1_792_625_272;
        let boot_ns = serde_json::json!(((now - 90_061) as u64 * 1_000_000_000).to_string());
        assert_eq!(
            uptime_from_boot(&boot_ns, now).as_deref(),
            Some("1 day, 1:01:01")
        );
        assert_eq!(rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_secs("2026-10-22T06:00:00.123Z"),
            Some(1_792_648_800)
        );
        assert_eq!(
            rfc3339_secs("2026-10-22T13:00:00+07:00"),
            Some(1_792_648_800)
        );
        assert_eq!(rfc3339_secs("yesterday"), None);
    }

    #[test]
    fn encodings_are_chosen_in_order() {
        assert_eq!(pick_encoding(&[0, 2, 4]), Encoding::JsonIetf);
        assert_eq!(pick_encoding(&[2, 0]), Encoding::Json);
        assert_eq!(pick_encoding(&[2]), Encoding::Proto);
        assert_eq!(pick_encoding(&[]), Encoding::JsonIetf);
    }
}
