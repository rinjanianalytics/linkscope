// SPDX-License-Identifier: GPL-2.0-or-later
//! The gNMI client against the mock device (`linkscope-mockagent`): every way a device answers
//! that LinkScope must read, and every failure it must name.

use std::io::Write;
use std::time::Duration;

use linkscope::gnmi::{DeviceKey, GnmiPool, poll_device};
use linkscope::poll::SourceResult;
use linkscope_config::{GnmiOptions, GnmiSource};
use linkscope_gnmi::Encoding;
use linkscope_mockagent::gnmi::{Device, Interface, Model, Origins, serve};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};

const PASSWORD: &str = "s3cret-gnmi-SECRET";

/// A CA and a server certificate for 127.0.0.1 signed by it, as PEM: (ca, cert, key).
fn pki() -> (String, String, String) {
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca, KeyPair::generate().unwrap()).unwrap();
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
        .unwrap()
        .signed_by(&key, &ca)
        .unwrap();
    (ca.pem(), cert.pem(), key.serialize_pem())
}

struct Lab {
    dir: tempfile::TempDir,
    port: u16,
    ca: String,
}

impl Lab {
    /// Start `dev` on a free port, with TLS unless `tls` is false.
    async fn start(dev: Device, tls: bool) -> Lab {
        let (ca, cert, key) = pki();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let identity = tls.then(|| (cert.into_bytes(), key.into_bytes()));
        tokio::spawn(serve(dev, listener, identity));
        let dir = tempfile::tempdir().unwrap();
        let lab = Lab { dir, port, ca };
        lab.file("ca.pem", &lab.ca.clone());
        lab.file("pass", &format!("{PASSWORD}\n"));
        lab
    }

    fn file(&self, name: &str, text: &str) -> String {
        let p = self.dir.path().join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        p.display().to_string()
    }

    fn options(&self, extra: &str) -> GnmiOptions {
        let d = self.dir.path().display();
        GnmiOptions::parse(&format!(
            "user=linkscope password-file={d}/pass ca={d}/ca.pem {extra}"
        ))
        .unwrap()
    }

    async fn read(&self, options: GnmiOptions, ifs: &[&str]) -> Vec<SourceResult> {
        read_with(&GnmiPool::default(), self.port, options, ifs).await
    }
}

async fn read_with(
    pool: &GnmiPool,
    port: u16,
    options: GnmiOptions,
    ifs: &[&str],
) -> Vec<SourceResult> {
    let members: Vec<(usize, GnmiSource)> = ifs
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                i,
                GnmiSource {
                    interface: n.to_string(),
                    host: "127.0.0.1".into(),
                    port,
                    options: options.clone(),
                },
            )
        })
        .collect();
    let key = DeviceKey::of(&members[0].1);
    let mut out = poll_device(pool, key, members, Duration::from_secs(5)).await;
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

fn device(model: Model) -> Device {
    let mut d = Device::new(
        model,
        "mock-router",
        vec![
            Interface::new("ethernet-1/1", 11, 18_446_744_073_709_551_000, 42),
            Interface::new("ethernet-1/2", 12, 1_000, 2_000),
        ],
    );
    d.login = Some(("linkscope".into(), PASSWORD.into()));
    d
}

/// The mock was up 90,061 s when it started; a second may pass before it is read.
fn assert_uptime(r: &SourceResult) {
    let u = r.uptime.as_deref().unwrap_or("");
    assert!(["1 day, 1:01:01", "1 day, 1:01:02"].contains(&u), "{u}");
}

fn ok(r: &SourceResult) -> (&str, &str) {
    assert_eq!(r.error, None);
    (r.input.as_deref().unwrap(), r.output.as_deref().unwrap())
}

#[tokio::test]
async fn openconfig_over_tls_with_a_login() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let r = lab
        .read(lab.options(""), &["ethernet-1/1", "ethernet-1/2"])
        .await;
    // 64-bit counters arrive whole (JSON_IETF sends them as strings).
    assert_eq!(ok(&r[0]), ("18446744073709551000", "42"));
    assert_eq!(ok(&r[1]), ("1000", "2000"));
    assert_eq!(r[0].if_index, [Some("11".into()), Some("11".into())]);
    assert_eq!(r[1].name.as_deref(), Some("mock-router"));
    assert_uptime(&r[1]);
}

#[tokio::test]
async fn srl_without_get_and_with_containers_in_proto() {
    // A device that only streams, answers leaves with their parent, and speaks PROTO only.
    let mut d = device(Model::Srl);
    d.get = false;
    d.containers = true;
    d.encodings = vec![Encoding::Proto];
    let lab = Lab::start(d, true).await;
    let r = lab.read(lab.options("model=srl"), &["ethernet-1/2"]).await;
    assert_eq!(ok(&r[0]), ("1000", "2000"));
    assert_eq!(r[0].name.as_deref(), Some("mock-router"));
    assert_uptime(&r[0]);
}

#[tokio::test]
async fn every_encoding_reads_the_same() {
    for enc in [Encoding::JsonIetf, Encoding::Json, Encoding::Proto] {
        let mut d = device(Model::OpenConfig);
        d.encodings = vec![enc];
        let lab = Lab::start(d, true).await;
        let r = lab.read(lab.options(""), &["ethernet-1/1"]).await;
        assert_eq!(ok(&r[0]), ("18446744073709551000", "42"), "{enc:?}");
    }
}

#[tokio::test]
async fn a_wrong_password_is_named_and_not_shown() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    lab.file("pass", "wrong-password-SECRET\n");
    let r = lab
        .read(lab.options(""), &["ethernet-1/1", "ethernet-1/2"])
        .await;
    for x in &r {
        let e = x.error.as_deref().unwrap();
        assert!(e.contains("login refused"), "{e}");
        assert!(!e.contains("SECRET"), "{e}");
        assert_eq!(x.input, None);
    }
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let (other_ca, _, _) = pki();
    lab.file("ca.pem", &other_ca);
    let r = lab.read(lab.options(""), &["ethernet-1/1"]).await;
    let e = r[0].error.as_deref().unwrap();
    assert!(
        e.contains("TLS") && e.to_lowercase().contains("certificate"),
        "{e}"
    );
    assert_eq!(r[0].input, None);

    // tls=insecure reads it anyway (encrypted, not authenticated).
    let d = lab.dir.path().display();
    let o = GnmiOptions::parse(&format!(
        "user=linkscope password-file={d}/pass tls=insecure"
    ))
    .unwrap();
    let r = lab.read(o, &["ethernet-1/1"]).await;
    assert_eq!(ok(&r[0]), ("18446744073709551000", "42"));
}

#[tokio::test]
async fn a_name_not_on_the_certificate_is_refused() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let r = lab
        .read(
            lab.options("tls-name=router.example.net"),
            &["ethernet-1/1"],
        )
        .await;
    let e = r[0].error.as_deref().unwrap();
    assert!(e.contains("TLS"), "{e}");
}

#[tokio::test]
async fn an_unknown_interface_fails_alone() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let r = lab
        .read(
            lab.options(""),
            &["ethernet-1/1", "ethernet-9/9", "ethernet-1/2"],
        )
        .await;
    assert_eq!(ok(&r[0]), ("18446744073709551000", "42"));
    assert_eq!(ok(&r[2]), ("1000", "2000"));
    let e = r[1].error.as_deref().unwrap();
    assert!(
        e.contains("ethernet-9/9") && e.contains("does not answer"),
        "{e}"
    );
}

#[tokio::test]
async fn the_wrong_model_is_named() {
    let lab = Lab::start(device(Model::Srl), true).await;
    let r = lab.read(lab.options(""), &["ethernet-1/1"]).await;
    let e = r[0].error.as_deref().unwrap();
    assert!(e.contains("openconfig"), "{e}");
}

#[tokio::test]
async fn plain_grpc_with_tls_off() {
    let mut d = device(Model::OpenConfig);
    d.login = None;
    let lab = Lab::start(d, false).await;
    let r = lab
        .read(GnmiOptions::parse("tls=off").unwrap(), &["ethernet-1/2"])
        .await;
    assert_eq!(ok(&r[0]), ("1000", "2000"));
}

#[tokio::test]
async fn a_stopped_device_is_named_and_a_connection_is_reused() {
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let pool = GnmiPool::default();
    for _ in 0..3 {
        let r = read_with(&pool, lab.port, lab.options(""), &["ethernet-1/2"]).await;
        assert_eq!(ok(&r[0]), ("1000", "2000"));
    }
    // Nothing listens on this port.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let r = read_with(&pool, port, lab.options(""), &["ethernet-1/2"]).await;
    let e = r[0].error.as_deref().unwrap();
    assert!(e.to_lowercase().contains("refused"), "{e}");
}

#[tokio::test]
async fn discovery_writes_a_config_that_polls() {
    // An SR Linux device with OpenConfig off: OpenConfig is tried, refused, then SR Linux's model.
    let mut d = device(Model::Srl);
    d.interfaces[1].up = false;
    let lab = Lab::start(d, true).await;
    let dir = lab.dir.path().display().to_string();
    let options_text = format!("user=linkscope password-file={dir}/pass ca={dir}/ca.pem");
    let options = GnmiOptions::parse(&options_text).unwrap();
    let key = DeviceKey {
        host: "127.0.0.1".into(),
        port: lab.port,
        options: options.clone(),
    };
    let found = linkscope::gnmi::discover(&key, false, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(found.model, linkscope_config::GnmiModel::Srl);
    assert_eq!(found.hostname.as_deref(), Some("mock-router"));
    assert_eq!(found.interfaces.len(), 2);
    assert_eq!(found.interfaces[0].speed_bps, Some(10_000_000_000));
    assert_eq!(found.interfaces[0].if_index.as_deref(), Some("11"));

    let text = linkscope::gnmi_cfgmaker::render("127.0.0.1", lab.port, &options, &found, false);
    // The down interface is written commented out.
    assert!(text.contains("# Not polled: it is down."), "{text}");

    // The generated targets load as a config and poll.
    let work = lab.dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let cfg_path = lab.dir.path().join("m.cfg");
    std::fs::set_permissions(
        lab.dir.path().join("pass"),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    std::fs::write(&cfg_path, format!("WorkDir: {}\n{text}", work.display())).unwrap();
    let cfg = linkscope_config::load(
        &cfg_path,
        &linkscope_config::ReadOptions {
            trust: linkscope_config::TrustPolicy::Off,
        },
        &linkscope_config::CheckOptions::default(),
    )
    .unwrap();
    assert_eq!(cfg.targets.len(), 1);
    assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
    let linkscope_config::Source::Gnmi(g) = &cfg.sources[0] else {
        panic!("not a gNMI source");
    };
    let r = read_with(
        &GnmiPool::default(),
        g.port,
        g.options.clone(),
        &["ethernet-1/1"],
    )
    .await;
    assert_eq!(ok(&r[0]), ("18446744073709551000", "42"));
}

#[tokio::test]
async fn discovery_names_a_device_that_answers_no_model() {
    // An OpenConfig device, but the options insist on SR Linux's model.
    let lab = Lab::start(device(Model::OpenConfig), true).await;
    let key = DeviceKey {
        host: "127.0.0.1".into(),
        port: lab.port,
        options: lab.options("model=srl"),
    };
    let e = linkscope::gnmi::discover(&key, true, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        e.contains("none of LinkScope's interface models") && e.contains("srl"),
        "{e}"
    );
}

#[tokio::test]
async fn a_gateway_target_is_named_in_each_request() {
    let mut d = device(Model::OpenConfig);
    d.target = Some("edge-router-7".into());
    let lab = Lab::start(d, true).await;
    let r = lab
        .read(lab.options("target=edge-router-7"), &["ethernet-1/2"])
        .await;
    assert_eq!(ok(&r[0]), ("1000", "2000"));
    assert_eq!(r[0].name.as_deref(), Some("mock-router"));
    let r = lab.read(lab.options(""), &["ethernet-1/2"]).await;
    assert!(r[0].error.is_some());
}

#[tokio::test]
async fn openconfig_origin_is_sent_or_dropped_as_the_device_wants() {
    // SR Linux: OpenConfig paths only with origin `openconfig`.
    let mut d = device(Model::OpenConfig);
    d.origins = Origins::OpenConfigNeedsIt;
    let lab = Lab::start(d, true).await;
    let r = lab.read(lab.options(""), &["ethernet-1/2"]).await;
    assert_eq!(ok(&r[0]), ("1000", "2000"));
    assert_eq!(r[0].name.as_deref(), Some("mock-router"));

    // A device that refuses any origin is asked again without one.
    let mut d = device(Model::OpenConfig);
    d.origins = Origins::Refused;
    let lab = Lab::start(d, true).await;
    let pool = GnmiPool::default();
    for _ in 0..2 {
        let r = read_with(&pool, lab.port, lab.options(""), &["ethernet-1/2"]).await;
        assert_eq!(ok(&r[0]), ("1000", "2000"));
        assert_eq!(r[0].if_index, [Some("12".into()), Some("12".into())]);
    }
}

#[tokio::test]
async fn an_unknown_interface_answered_with_nothing_fails_alone() {
    // SR Linux answers an interface it does not have with an empty notification.
    let mut d = device(Model::Srl);
    d.missing_is_empty = true;
    let lab = Lab::start(d, true).await;
    let r = lab
        .read(lab.options("model=srl"), &["ethernet-1/1", "ethernet-9/9"])
        .await;
    assert_eq!(ok(&r[0]), ("18446744073709551000", "42"));
    let e = r[1].error.as_deref().unwrap();
    assert!(
        e.contains("ethernet-9/9") && e.contains("no srl counter"),
        "{e}"
    );
}
