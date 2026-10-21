// SPDX-License-Identifier: GPL-2.0-or-later
//! LinkScope's alert keywords: what they accept, what they need beside them,
//! and that no error message repeats a secret.

use linkscope_config::{CheckOptions, ConfigError, ReadOptions, TrustPolicy};

fn load(body: &str) -> Result<linkscope_config::Config, String> {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let path = dir.path().join("m.cfg");
    std::fs::write(&path, format!("WorkDir: {}\n{body}Target[a]: 1:pub-lic@10.0.0.1\nTitle[a]: A\nMaxBytes[a]: 100\n", work.display())).unwrap();
    linkscope_config::load(&path, &ReadOptions { trust: TrustPolicy::Off }, &CheckOptions::default()).map_err(|e: ConfigError| e.to_string())
}

#[test]
fn a_full_alert_setup_loads() {
    let c = load(
        "ThreshMailServer: localhost:587\nThreshMailSender: lab@example.com\nAlertMailUser: lab\nAlertMailPassword: pw-SECRET\n\
         AlertWebhook: https://hooks.example.com/t/tok-SECRET\nAlertWebhookSecret: 0123456789abcdef\nThreshFor[a]: 3\nThreshMaxI[a]: 30%\n",
    )
    .unwrap();
    assert_eq!(c.global.thresh_mail_server.as_deref(), Some("localhost:587"));
    assert_eq!(c.global.alert_mail_user.as_deref(), Some("lab"));
    assert_eq!(c.targets[0].thresholds.for_polls, Some(3));
    // Never serialized: the URL's token, the key, the password.
    let json = serde_json::to_string(&c).unwrap();
    for s in ["tok-SECRET", "0123456789abcdef", "pw-SECRET"] {
        assert!(!json.contains(s), "{s} serialized");
    }
}

#[test]
fn keywords_come_in_pairs() {
    let e = load("AlertWebhook: http://127.0.0.1:9000/hook\n").unwrap_err();
    assert!(e.contains("If \"AlertWebhook\" is defined, then \"AlertWebhookSecret\" must be defined too."), "{e}");
    let e = load("AlertWebhookSecret: 0123456789abcdef\n").unwrap_err();
    assert!(e.contains("then \"AlertWebhook\" must be defined too"), "{e}");
    let e = load("ThreshMailServer: localhost\nThreshMailSender: a@b.c\nAlertMailUser: lab\n").unwrap_err();
    assert!(e.contains("then \"AlertMailPassword\" must be defined too"), "{e}");
    let e = load("AlertMailUser: lab\nAlertMailPassword: pw\n").unwrap_err();
    assert!(e.contains("needs \"ThreshMailServer:\""), "{e}");
}

#[test]
fn bad_values_are_refused_without_repeating_them() {
    let e = load("AlertWebhook: ftp://x/tok-SECRET\nAlertWebhookSecret: 0123456789abcdef\n").unwrap_err();
    assert!(e.contains("must be an http:// or https:// URL") && !e.contains("tok-SECRET"), "{e}");
    let e = load("AlertWebhook: http://x/\nAlertWebhookSecret: short-SECRET\n").unwrap_err();
    assert!(e.contains("at least 16 characters") && !e.contains("short-SECRET"), "{e}");
    let e = load("ThreshFor[a]: 0\n").unwrap_err();
    assert!(e.contains("CFG Error in \"threshfor[]\": 0 must be a Number bigger than 0"), "{e}");
    let e = load("ThreshMailServer: not a host!:587\nThreshMailSender: a@b.c\n").unwrap_err();
    assert!(e.contains("Unknown mailserver hostname"), "{e}");
    // Judged by its form: no DNS lookup blocks --check or a reload (a name that does not resolve
    // shows when mail is sent).
    let started = std::time::Instant::now();
    assert!(load("ThreshMailServer: no-such-host.invalid:587\nThreshMailSender: a@b.c\n").is_ok());
    assert!(started.elapsed().as_secs() < 2);
}
