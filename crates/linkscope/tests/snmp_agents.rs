// SPDX-License-Identifier: GPL-2.0-or-later
//! Integration tests against the agents started by testenv/start.sh.
//! Each test skips when its agent is not reachable.

use std::time::Duration;

use linkscope_config::{SnmpVersion, SnmpOptions};
use linkscope::snmp::{Endpoint, Session, V3Params, Val};

fn ep(port: u16, version: SnmpVersion, community: &str) -> Endpoint {
    Endpoint {
        host: "127.0.0.1".into(),
        port,
        version,
        community: community.into(),
        v3: None,
        timeout: Duration::from_millis(1500),
        retries: 1,
        backoff: 1.0,
        ipv4only: true,
        max_repetitions: 25,
        use_getbulk: true,
        max_varbinds: 32,
    }
}

async fn up(e: &Endpoint) -> Option<Session> {
    let mut s = Session::open(e).await.ok()?;
    match s.get(&["1.3.6.1.2.1.1.5.0".into()]).await {
        Ok(v) if v[0].is_present() => Some(s),
        _ => {
            eprintln!("skipping: agent {}:{} not reachable", e.host, e.port);
            None
        }
    }
}

#[tokio::test]
async fn v2c_get_batch_and_walk() {
    let Some(mut s) = up(&ep(20161, SnmpVersion::V2c, "dev1")).await else { return };
    let oids: Vec<String> = (1..=24).flat_map(|i| [format!("1.3.6.1.2.1.2.2.1.10.{i}"), format!("1.3.6.1.2.1.31.1.1.1.6.{i}")]).chain(["1.3.6.1.2.1.1.5.0".into(), "1.3.6.1.2.1.99.9.9".into()]).collect();
    let v = s.get(&oids).await.unwrap();
    assert_eq!(v.len(), 50);
    assert!(matches!(v[0], Val::Unsigned(_)));
    assert!(matches!(v[1], Val::Counter64(_)));
    assert_eq!(v[48].pretty(), "dev1.lab");
    assert_eq!(v[49], Val::Missing);
    let w = s.walk("1.3.6.1.2.1.2.2.1.2").await.unwrap();
    assert_eq!(w.len(), 24);
    assert_eq!(w[0].0, "1");
    assert_eq!(w[23].1.pretty(), "GigabitEthernet0/24");
    let ips = s.walk("1.3.6.1.2.1.4.20.1.2").await.unwrap();
    assert_eq!(ips[0].0, "10.1.1.1");
}

#[tokio::test]
async fn v1_get_with_missing_oid_and_getnext_walk() {
    let Some(mut s) = up(&ep(20161, SnmpVersion::V1, "dev2")).await else { return };
    let v = s.get(&["1.3.6.1.2.1.2.2.1.10.3".into(), "1.3.6.1.2.1.77.1".into(), "1.3.6.1.2.1.1.5.0".into()]).await.unwrap();
    assert!(matches!(v[0], Val::Unsigned(_)), "{v:?}");
    assert_eq!(v[1], Val::Missing);
    assert_eq!(v[2].pretty(), "dev2.lab");
    let w = s.walk("1.3.6.1.2.1.31.1.1.1.1").await.unwrap();
    assert_eq!(w.len(), 24);
}

#[tokio::test]
async fn wrong_community_times_out() {
    let e = Endpoint { timeout: Duration::from_millis(300), retries: 1, ..ep(20171, SnmpVersion::V2c, "wrong") };
    let mut s = Session::open(&e).await.unwrap();
    let t = std::time::Instant::now();
    assert!(s.get(&["1.3.6.1.2.1.1.5.0".into()]).await.is_err());
    assert!(t.elapsed() < Duration::from_secs(2));
}

async fn v3(user: &str, auth: &str, privp: Option<&str>) -> Option<String> {
    let mut o = SnmpOptions::new();
    o.insert("username".into(), user.into());
    o.insert("authpassword".into(), "authpass123".into());
    o.insert("authprotocol".into(), auth.into());
    if let Some(p) = privp {
        o.insert("privpassword".into(), "privpass123".into());
        o.insert("privprotocol".into(), p.into());
    }
    let mut e = ep(20171, SnmpVersion::V3, "");
    e.v3 = Some(V3Params::from_options(&o).unwrap());
    let probe = up(&ep(20171, SnmpVersion::V2c, "public")).await?;
    drop(probe);
    let mut s = Session::open(&e).await.unwrap();
    let v = s.get(&["1.3.6.1.2.1.1.5.0".into(), "1.3.6.1.2.1.2.2.1.10.1".into()]).await.unwrap();
    assert!(matches!(v[1], Val::Unsigned(_)));
    let w = s.walk("1.3.6.1.2.1.2.2.1.2").await.unwrap();
    assert!(!w.is_empty());
    Some(v[0].pretty())
}

#[tokio::test]
async fn v3_auth_priv_sha_aes() {
    if let Some(n) = v3("v3priv", "sha", Some("aes")).await {
        assert_eq!(n, "linkscope-testagent");
    }
}

#[tokio::test]
async fn v3_auth_no_priv_md5() {
    if let Some(n) = v3("v3auth", "md5", None).await {
        assert_eq!(n, "linkscope-testagent");
    }
}

#[tokio::test]
async fn v3_sha256_aes() {
    if let Some(n) = v3("v3sha256", "sha256", Some("aes")).await {
        assert_eq!(n, "linkscope-testagent");
    }
}

#[tokio::test]
async fn v3_wrong_password_is_reported() {
    let Some(_) = up(&ep(20171, SnmpVersion::V2c, "public")).await else { return };
    let mut o = SnmpOptions::new();
    o.insert("username".into(), "v3priv".into());
    o.insert("authpassword".into(), "wrongpass1".into());
    o.insert("authprotocol".into(), "sha".into());
    o.insert("privpassword".into(), "privpass123".into());
    o.insert("privprotocol".into(), "aes".into());
    let mut e = ep(20171, SnmpVersion::V3, "");
    e.timeout = Duration::from_millis(500);
    e.v3 = Some(V3Params::from_options(&o).unwrap());
    let mut s = Session::open(&e).await.unwrap();
    assert!(s.get(&["1.3.6.1.2.1.1.5.0".into()]).await.is_err());
}
