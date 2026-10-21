// SPDX-License-Identifier: GPL-2.0-or-later
//! A target whose threshold alert is firing shows as warn, with the bound
//! that is broken; one still waiting out `ThreshFor[]` stays ok.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::Request;
use linkscope_web::{AlertLive, LiveState, TargetLive};
use tower::ServiceExt;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn a_firing_alert_is_warn_with_its_bound() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("m.cfg");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            "WorkDir: {}\n\
             Target[busy]: 1:pub-lic@10.0.0.1\nTitle[busy]: Busy\nMaxBytes[busy]: 125000\nThreshMaxI[busy]: 30%\nThreshFor[busy]: 2\n\
             Target[soon]: 2:pub-lic@10.0.0.1\nTitle[soon]: Soon\nMaxBytes[soon]: 125000\nThreshMaxI[soon]: 30%\nThreshFor[soon]: 2\n",
            work.display()
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
    assert_eq!(cfg.targets[0].thresholds.for_polls, Some(2));
    let alert = |firing: bool, polls: u32| AlertLive {
        direction: "in".into(),
        bound: "max".into(),
        threshold: "30%".into(),
        threshold_rate: 37_500.0,
        rate: 52_118.0,
        percent: Some(41.7),
        firing,
        polls,
        polls_needed: 2,
        since: now() - 600,
        fired_at: firing.then(|| now() - 300),
    };
    let live = |a: AlertLive| TargetLive {
        last_poll: now(),
        in_raw: Some("1".into()),
        out_raw: Some("2".into()),
        cur_in: Some(52_118.0),
        cur_out: Some(64.0),
        alerts: vec![a],
        ..Default::default()
    };
    let mut st = LiveState {
        config: Some(Arc::new(cfg)),
        ..Default::default()
    };
    st.targets.insert("busy".into(), live(alert(true, 3)));
    st.targets.insert("soon".into(), live(alert(false, 1)));
    let app = linkscope_web::router(Arc::new(RwLock::new(st)));

    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let res = app
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let body = axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        }
    };
    let rows = get("/api/targets").await;
    let by = |n: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(by("busy")["status"], "warn");
    assert_eq!(by("busy")["alerts"][0]["bound"], "max");
    assert_eq!(by("busy")["alerts"][0]["direction"], "in");
    assert_eq!(by("busy")["alerts"][0]["threshold"], "30%");
    assert_eq!(by("soon")["status"], "ok");
    assert_eq!(by("soon")["alerts"][0]["firing"], false);
    let s = get("/api/summary").await;
    assert_eq!(
        (s["targets_warn"].as_u64(), s["targets_ok"].as_u64()),
        (Some(1), Some(1))
    );
    let d = get("/api/targets/busy").await;
    assert_eq!(d["thresholds"]["for_polls"], 2);
}
