// SPDX-License-Identifier: GPL-2.0-or-later
//! No API response may contain SNMP communities, v3 passwords, external
//! command lines or the alert settings' secrets (the webhook URL's token,
//! its signing key, the mail password), even when errors quote them.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::Request;
use linkscope_web::{LiveState, TargetLive};
use tower::ServiceExt;

const SECRETS: &[&str] = &["Comm-un1ty", "authpass-SECRET", "privpass-SECRET", "--password=cmdSECRET", "hooktoken-SECRET", "hmackey-SECRET", "mailpass-SECRET"];
/// Not secrets, but the server's business on a UI without a login: where the threshold programs
/// live (a program of the test's own, see below) and whom threshold mail goes to.
const PRIVATE: &[&str] = &["thresh-hook-SECRET", "noc-private@example.com"];

#[tokio::test]
async fn api_never_returns_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("m.cfg");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    // A threshold program must exist and be executable for the config to load, as in MRTG.
    let hook = dir.path().join("thresh-hook-SECRET");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            "WorkDir: {}\n\
             ThreshMailServer: localhost:587\nThreshMailSender: lab@example.com\nAlertMailUser: lab\nAlertMailPassword: mailpass-SECRET\n\
             AlertWebhook: https://hooks.example.com/t/hooktoken-SECRET\nAlertWebhookSecret: hmackey-SECRET-0123456789\n\
             Target[a]: 2:Comm-un1ty@router1 / 2\nTitle[a]: A\nMaxBytes[a]: 100\nThreshMaxI[a]: 30%\nThreshFor[a]: 2\nThreshMailAddress[a]: noc-private@example.com\nThreshProgI[a]: {}\n\
             Target[b]: 3:x@router2:::::3\nSnmpOptions[b]: username=>'u', authpassword=>'authpass-SECRET', privpassword=>'privpass-SECRET'\nTitle[b]: B\nMaxBytes[b]: 100\n\
             Target[c]: `/usr/local/bin/probe --password=cmdSECRET`\nTitle[c]: C\nMaxBytes[c]: 100\n",
            work.display(),
            hook.display()
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
    st.warnings.push("posting to https://hooks.example.com/t/hooktoken-SECRET with hmackey-SECRET-0123456789 failed; mail login mailpass-SECRET refused".into());
    let state = Arc::new(RwLock::new(st));
    let app = linkscope_web::router(state);
    for uri in ["/api/summary", "/api/targets", "/api/targets/a", "/api/targets/b", "/api/targets/c", "/metrics"] {
        let res = app.clone().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        for s in SECRETS.iter().chain(PRIVATE) {
            assert!(!text.contains(s), "{uri} leaks {s}: {text}");
        }
    }
}
