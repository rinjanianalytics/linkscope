// SPDX-License-Identifier: GPL-2.0-or-later
//! No API response may contain SNMP communities, v3 passwords or external
//! command lines, even when errors quote them.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::Request;
use linkscope_web::{LiveState, TargetLive};
use tower::ServiceExt;

const SECRETS: &[&str] = &["Comm-un1ty", "authpass-SECRET", "privpass-SECRET", "--password=cmdSECRET"];

#[tokio::test]
async fn api_never_returns_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("m.cfg");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            "WorkDir: {}\n\
             Target[a]: 2:Comm-un1ty@router1 / 2\nTitle[a]: A\nMaxBytes[a]: 100\n\
             Target[b]: 3:x@router2:::::3\nSnmpOptions[b]: username=>'u', authpassword=>'authpass-SECRET', privpassword=>'privpass-SECRET'\nTitle[b]: B\nMaxBytes[b]: 100\n\
             Target[c]: `/usr/local/bin/probe --password=cmdSECRET`\nTitle[c]: C\nMaxBytes[c]: 100\n",
            work.display()
        ),
    )
    .unwrap();
    let cfg = linkscope_config::load(&cfg_path, &linkscope_config::ReadOptions { trust: linkscope_config::TrustPolicy::Off }, &linkscope_config::CheckOptions::default()).unwrap();
    let mut st = LiveState { config: Some(Arc::new(cfg.clone())), ..Default::default() };
    for t in &cfg.targets {
        // Worst case: an error message that quotes the whole Target text.
        st.targets.insert(t.name.clone(), TargetLive { error: Some(format!("failed: {}", t.target_text)), last_poll: 1, ..Default::default() });
    }
    st.warnings.push("warning mentioning Comm-un1ty and /usr/local/bin/probe --password=cmdSECRET".into());
    let state = Arc::new(RwLock::new(st));
    let app = linkscope_web::router(state);
    for uri in ["/api/summary", "/api/targets", "/api/targets/a", "/api/targets/b", "/api/targets/c", "/metrics"] {
        let res = app.clone().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        for s in SECRETS {
            assert!(!text.contains(s), "{uri} leaks {s}: {text}");
        }
    }
}
