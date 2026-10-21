// SPDX-License-Identifier: GPL-2.0-or-later
//! What every answer of the web listener says about itself: live data is never cached, every
//! answer keeps other origins out, an unknown API path is an error (not the UI's page), only
//! GET and HEAD are served, and the summary names the configuration without its path.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use linkscope_web::LiveState;
use tower::ServiceExt;

#[tokio::test]
async fn headers_methods_and_unknown_paths() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("private-dir-name").join("mrtg.cfg");
    std::fs::create_dir(cfg_path.parent().unwrap()).unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            "WorkDir: {}\nTarget[a]: 2:public@192.0.2.1\nTitle[a]: A\nMaxBytes[a]: 100\n",
            dir.path().display()
        ),
    )
    .unwrap();
    let cfg = linkscope_config::load(
        &cfg_path,
        &linkscope_config::ReadOptions {
            trust: linkscope_config::TrustPolicy::Off,
        },
        &linkscope_config::CheckOptions::default(),
    )
    .unwrap();
    let app = linkscope_web::router(Arc::new(RwLock::new(LiveState {
        config: Some(Arc::new(cfg)),
        ..Default::default()
    })));
    let send = |method: Method, uri: &'static str| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let (parts, body) = res.into_parts();
            (
                parts,
                String::from_utf8_lossy(&axum::body::to_bytes(body, 1 << 20).await.unwrap())
                    .into_owned(),
            )
        }
    };

    let (summary, body) = send(Method::GET, "/api/summary").await;
    assert_eq!(summary.status, StatusCode::OK);
    assert_eq!(summary.headers["cache-control"], "no-store");
    assert_eq!(
        summary.headers["cross-origin-resource-policy"],
        "same-origin"
    );
    assert_eq!(summary.headers["cross-origin-opener-policy"], "same-origin");
    assert!(
        body.contains(r#""config_name":"mrtg.cfg""#) && !body.contains("private-dir-name"),
        "{body}"
    );

    let (metrics, _) = send(Method::GET, "/metrics").await;
    assert_eq!(metrics.headers["cache-control"], "no-store");

    // The UI's own files keep their caching; an unknown page is still the UI (its router).
    let (page, _) = send(Method::GET, "/target/a").await;
    assert_eq!(
        (page.status, &page.headers["cache-control"]),
        (StatusCode::OK, &"no-cache".parse().unwrap())
    );

    assert_eq!(
        send(Method::GET, "/api/nothing").await.0.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(Method::GET, "/api").await.0.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(Method::POST, "/anything").await.0.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        send(Method::POST, "/api/summary").await.0.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
}
