// SPDX-License-Identifier: GPL-2.0-or-later
//! The UI answers only to IP addresses, `localhost` and the names in `WebHosts:`. It has no
//! login, so a web page whose own name is made to resolve to the UI's address (DNS rebinding)
//! must not be able to read it from a browser on the same network.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use linkscope_web::{LiveState, host_allowed};
use tower::ServiceExt;

#[test]
fn addresses_localhost_and_listed_names_only() {
    let none: Vec<String> = vec![];
    for ok in ["127.0.0.1", "127.0.0.1:8080", "192.0.2.10:8090", "[::1]:8080", "[2001:db8::1]", "localhost", "LOCALHOST:8080", "localhost."] {
        assert!(host_allowed(ok, &none), "{ok}");
    }
    for no in ["evil.example", "evil.example:8080", "127.0.0.1.evil.example", "localhost.evil.example", ""] {
        assert!(!host_allowed(no, &none), "{no}");
    }
    let listed = vec!["ui.example.com".to_owned()];
    assert!(host_allowed("UI.example.com:8443", &listed) && !host_allowed("other.example.com", &listed));
    assert!(host_allowed("anything.example", &["*".to_owned()]));
}

#[tokio::test]
async fn a_request_for_an_unknown_host_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("m.cfg");
    std::fs::write(&cfg_path, format!("WorkDir: {}\nWebHosts: ui.example.com, noc.example.com\nTarget[a]: 2:public@192.0.2.1\nTitle[a]: A\nMaxBytes[a]: 100\n", dir.path().display())).unwrap();
    let cfg = linkscope_config::load(&cfg_path, &linkscope_config::ReadOptions { trust: linkscope_config::TrustPolicy::Off }, &linkscope_config::CheckOptions::default()).unwrap();
    assert_eq!(cfg.global.web_hosts, ["ui.example.com", "noc.example.com"]);
    let app = linkscope_web::router(Arc::new(RwLock::new(LiveState { config: Some(Arc::new(cfg)), ..Default::default() })));
    let get = |host: Option<&'static str>| {
        let app = app.clone();
        async move {
            let mut req = Request::get("/api/summary");
            if let Some(h) = host {
                req = req.header("host", h);
            }
            app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap().status()
        }
    };
    assert_eq!(get(Some("127.0.0.1:8080")).await, StatusCode::OK);
    assert_eq!(get(Some("noc.example.com:8443")).await, StatusCode::OK);
    assert_eq!(get(None).await, StatusCode::OK);
    assert_eq!(get(Some("evil.example")).await, StatusCode::MISDIRECTED_REQUEST);
}
