// SPDX-License-Identifier: GPL-2.0-or-later
//! Built-in web UI for LinkScope: a JSON API plus a single-page app whose
//! files are embedded in the binary (no CDN, works on isolated networks).

pub mod api;
pub mod docs;
pub mod metrics;
pub mod state;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use axum::middleware::{self, Next};
use axum::routing::get;
use include_dir::{Dir, include_dir};

pub use state::{AlertLive, CycleInfo, LiveState, SharedState, TargetLive};

static UI: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/ui/dist");

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

async fn static_file(req: Request<Body>) -> Response<Body> {
    // The UI's files are read, nothing else: an unknown API path is an error, not the UI's page.
    if !matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD) {
        return Response::builder().status(StatusCode::METHOD_NOT_ALLOWED).header(header::ALLOW, "GET, HEAD").body(Body::empty()).unwrap_or_default();
    }
    let path = req.uri().path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("no such API endpoint\n"))
            .unwrap_or_default();
    }
    let (file, path) = match UI.get_file(path) {
        Some(f) if !path.is_empty() => (f, path),
        // A stale hashed asset must fail clearly, not return the page.
        None if path.starts_with("assets/") => {
            return Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap();
        }
        // Single-page app: unknown paths get index.html.
        _ => match UI.get_file("index.html") {
            Some(f) => (f, "index.html"),
            None => return Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap(),
        },
    };
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Response::builder()
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.contents()))
        .unwrap()
}

/// The host name of a `Host` header, without its port; an IPv6 literal keeps its brackets off.
fn host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// Whether the UI answers a request for `host`: an IP address, `localhost`, or a name in
/// `WebHosts:` (`*`: any).
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let name = host_name(host.trim()).trim_end_matches('.');
    name.parse::<std::net::IpAddr>().is_ok()
        || name.eq_ignore_ascii_case("localhost")
        || allowed.iter().any(|a| a == "*" || a.eq_ignore_ascii_case(name))
}

/// Only requests for a known host are answered. The UI has no login: without this, a web page
/// whose name is made to resolve to the UI's address (DNS rebinding) reads it from a browser on
/// the same network. A request without a `Host` header is not from a browser and is answered.
async fn known_host(axum::extract::State(state): axum::extract::State<SharedState>, req: Request<Body>, next: Next) -> Response<Body> {
    if let Some(host) = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) {
        let allowed = state.read().unwrap_or_else(|e| e.into_inner()).config.as_ref().map(|c| c.global.web_hosts.clone()).unwrap_or_default();
        if !host_allowed(host, &allowed) {
            return Response::builder()
                .status(StatusCode::MISDIRECTED_REQUEST)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from("This LinkScope does not answer to this host name. Add it to WebHosts: in the configuration.\n"))
                .unwrap_or_default();
        }
    }
    next.run(req).await
}

/// Security headers on every response.
async fn harden(req: Request<Body>, next: Next) -> Response<Body> {
    // Live data is never kept by a browser or a proxy; the UI's files have their own caching.
    let live = req.uri().path().starts_with("/api/") || req.uri().path() == "/metrics";
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, k: &'static str, v: &'static str| {
        h.insert(k, HeaderValue::from_static(v));
    };
    set(h, "content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'");
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "no-referrer");
    set(h, "x-frame-options", "DENY");
    set(h, "cross-origin-opener-policy", "same-origin");
    set(h, "cross-origin-resource-policy", "same-origin");
    if live {
        set(h, "cache-control", "no-store");
    }
    res
}

/// The application router (exposed for tests).
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/summary", get(api::summary))
        .route("/api/targets", get(api::targets))
        .route("/api/targets/{name}", get(api::target))
        .route("/api/targets/{name}/series", get(api::series))
        .route("/api/docs", get(docs::index))
        .route("/api/docs/{lang}/{page}", get(docs::page))
        .route("/metrics", get(metrics::metrics))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(static_file)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, known_host))
        .layer(middleware::from_fn(harden))
}

/// Serve the web UI on `addr` until the process exits. The UI is read-only.
pub async fn serve(addr: SocketAddr, state: SharedState) -> std::io::Result<()> {
    serve_listener(tokio::net::TcpListener::bind(addr).await?, state).await
}

/// Connections served at once. More wait in the kernel's accept queue and hold none of the
/// daemon's file descriptors: the poller needs those for its sockets and logs, and a client
/// holding connections open must not starve it (without a cap, about 1,100 half-sent requests
/// used every descriptor and the next cycle polled nothing, 2026-10-21).
pub const MAX_CONNECTIONS: usize = 256;
/// A request's headers must arrive within this.
pub const HEADER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// A connection, however it is used, is closed after this; the UI simply reconnects.
pub const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Serve on an already bound listener (bound before privileges are dropped).
pub async fn serve_listener(listener: tokio::net::TcpListener, state: SharedState) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    tracing::info!("web UI on http://{addr}/");
    if !addr.ip().is_loopback() {
        tracing::warn!("web UI is listening on {addr}, which is reachable from the network; it has no login, so put it behind a reverse proxy with authentication if needed");
    }
    let app = router(state);
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        // A permit first, then accept: past the cap, clients wait in the kernel's queue.
        let Ok(permit) = permits.clone().acquire_owned().await else { return Ok(()) };
        let socket = match listener.accept().await {
            Ok((socket, _)) => socket,
            Err(e) => {
                tracing::warn!("web UI: accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
        };
        let service = hyper_util::service::TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let mut builder = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
            builder.http1().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
            let conn = builder.serve_connection(hyper_util::rt::TokioIo::new(socket), service);
            let _ = tokio::time::timeout(CONNECTION_TIMEOUT, conn).await;
            drop(permit);
        });
    }
}
