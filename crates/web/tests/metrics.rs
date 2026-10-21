// SPDX-License-Identifier: GPL-2.0-or-later
//! `/metrics`: the Prometheus exposition of a target's rates, counters and status.

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::Request;
use linkscope_web::{CycleInfo, LiveState, TargetLive};
use tower::ServiceExt;

async fn scrape(cfg_text: &str, live: Vec<(&str, TargetLive)>) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("m.cfg");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        &cfg_path,
        format!("WorkDir: {}\n{cfg_text}", work.display()),
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
    let mut st = LiveState {
        config: Some(Arc::new(cfg)),
        started: 1_700_000_000,
        cycles: 3,
        ..Default::default()
    };
    st.last_cycle = Some(CycleInfo {
        started: 1_700_000_600,
        duration_ms: 615,
        targets: 2,
        ok: 2,
        failed: 0,
        write_failures: 0,
    });
    for (name, l) in live {
        st.targets.insert(name.into(), l);
    }
    let app = linkscope_web::router(Arc::new(RwLock::new(st)));
    let res = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let ctype = res.headers()["content-type"].to_str().unwrap().to_string();
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (ctype, String::from_utf8(body.to_vec()).unwrap())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn an_interface_target_and_a_gauge() {
    let cfg = "Target[r1_eth1]: #ether1:pub-lic@10.0.0.1\nMaxBytes[r1_eth1]: 125000\nTitle[r1_eth1]: eth1\nDirectory[r1_eth1]: core\n\
               Target[temp]: 1.3.6.1.4.1.9.1&1.3.6.1.4.1.9.2:pub-lic@10.0.0.2\nOptions[temp]: gauge, perminute\nMaxBytes[temp]: 600\nTitle[temp]: T\n";
    let eth = TargetLive {
        last_poll: now(),
        in_raw: Some("7453560794".into()),
        out_raw: Some("365354".into()),
        if_index: Some("1".into()),
        cur_in: Some(51_500.0),
        cur_out: Some(2.0),
        ..Default::default()
    };
    let temp = TargetLive {
        last_poll: now(),
        in_raw: Some("42".into()),
        out_raw: Some("43".into()),
        cur_in: Some(120.0),
        cur_out: Some(60.0),
        ..Default::default()
    };
    let (ctype, m) = scrape(cfg, vec![("r1_eth1", eth), ("temp", temp)]).await;
    assert!(ctype.starts_with("text/plain; version=0.0.4"), "{ctype}");
    let l = r#"target="r1_eth1",directory="core",source="snmp",host="10.0.0.1",interface="Name:ether1",ifindex="1""#;
    for want in [
        format!("linkscope_target_counter_total{{{l},direction=\"in\"}} 7453560794"),
        format!("linkscope_target_counter_total{{{l},direction=\"out\"}} 365354"),
        format!("linkscope_target_rate{{{l},direction=\"in\"}} 51500"),
        format!("linkscope_target_max{{{l},direction=\"in\"}} 125000"),
        format!("linkscope_target_status{{{l},status=\"ok\"}} 1"),
        format!("linkscope_target_status{{{l},status=\"error\"}} 0"),
        "linkscope_cycles_total 3".to_string(),
        "linkscope_last_cycle_duration_seconds 0.615".to_string(),
        "# TYPE linkscope_target_counter_total counter".to_string(),
    ] {
        assert!(m.contains(&want), "missing {want}\n{m}");
    }
    // A gauge is a value, not a counter, and per-minute rates become per second.
    // MRTG reads the explicit OIDs as interface 1.
    let g =
        r#"target="temp",directory="",source="snmp",host="10.0.0.2",interface="If:1",ifindex="""#;
    assert!(
        m.contains(&format!(
            "linkscope_target_value{{{g},direction=\"in\"}} 42"
        )),
        "{m}"
    );
    assert!(
        !m.contains(r#"linkscope_target_counter_total{target="temp""#),
        "{m}"
    );
    assert!(
        m.contains(&format!("linkscope_target_rate{{{g},direction=\"in\"}} 2")),
        "{m}"
    );
    assert!(
        m.contains(&format!("linkscope_target_max{{{g},direction=\"in\"}} 10")),
        "{m}"
    );
    assert!(!m.contains("pub-lic"), "a community leaked: {m}");
}

#[tokio::test]
async fn a_target_not_polled_yet_is_pending_with_no_samples() {
    let (_, m) = scrape(
        "Target[a]: 2:pub-lic@10.0.0.1\nMaxBytes[a]: 100\nTitle[a]: A\n",
        vec![],
    )
    .await;
    assert!(m.contains(r#"linkscope_target_status{target="a",directory="",source="snmp",host="10.0.0.1",interface="If:2",ifindex="",status="pending"} 1"#), "{m}");
    assert!(!m.contains("linkscope_target_counter_total{"), "{m}");
    assert!(!m.contains("linkscope_target_rate{"), "{m}");
}
