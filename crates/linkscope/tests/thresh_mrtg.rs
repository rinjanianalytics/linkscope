// SPDX-License-Identifier: GPL-2.0-or-later
//! MRTG's threshold behavior, pinned: for targets without LinkScope's alert
//! keywords, the programs run, the mails sent and the `ThreshDir` state files
//! must stay exactly what MRTG's `threshcheck` produces.
//!
//! The expected transcript is `tests/data/thresh_mrtg.txt`, recorded from the
//! port of MRTG 2.17.10 before LinkScope's alerts were added. To record it
//! again: `LINKSCOPE_RECORD=1 cargo test -p linkscope --test thresh_mrtg`.

use std::fmt::Write as _;
use std::path::Path;

use linkscope::thresh;

fn load(dir: &Path, body: &str) -> linkscope_config::Config {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let path = dir.join("m.cfg");
    std::fs::write(&path, format!("WorkDir: {}\n{body}", work.display())).unwrap();
    linkscope_config::load(
        &path,
        &linkscope_config::ReadOptions {
            trust: linkscope_config::TrustPolicy::Off,
        },
        &linkscope_config::CheckOptions::default(),
    )
    .unwrap()
}

fn state_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

const TARGETS: &str = "\
Target[r1]: 1:pub-lic@10.0.0.1
MaxBytes[r1]: 125000
Title[r1]: Router one <b>ether2</b>
PageTop[r1]: <h1>R1</h1><table><tr><td>System:</td><td>r1</td></tr></table>
SetEnv[r1]: SITE=\"lab\"
ThreshMaxI[r1]: 30%
ThreshMinO[r1]: 100
ThreshDesc[r1]: uplink to the lab
ThreshProgI[r1]: /bin/true
ThreshProgOKI[r1]: /bin/echo
ThreshProgO[r1]: /bin/false
ThreshMailAddress[r1]: noc@example.com
";

/// Rates (in, out) for successive polls: broken in, both, cleared, broken again.
const POLLS: &[(f64, f64)] = &[
    (52_118.0, 400.0),
    (52_118.0, 64.0),
    (60_000.0, 0.0),
    (10_000.0, 500.0),
    (40_000.5, 99.9),
];

fn run(title: &str, cfg: &linkscope_config::Config, thresh_dir: Option<&Path>, out: &mut String) {
    let _ = writeln!(out, "== {title}");
    let t = &cfg.targets[0];
    for (i, (r_in, r_out)) in POLLS.iter().enumerate() {
        let actions = thresh::check(cfg, t, Some(*r_in), Some(*r_out));
        let _ = writeln!(out, "-- poll {i}: in {r_in}, out {r_out}");
        for a in actions {
            let _ = writeln!(out, "{a:#?}");
        }
        if let Some(d) = thresh_dir {
            let _ = writeln!(out, "state files: {:?}", state_files(d));
        }
    }
}

#[test]
fn thresholds_behave_as_mrtg() {
    let tmp = tempfile::tempdir().unwrap();
    let mut out = String::new();

    // No ThreshDir: every broken poll acts, nothing on recovery.
    let plain = load(
        &tmp.path().join("a"),
        &format!("ThreshMailServer: localhost\nThreshMailSender: mrtg@example.com\n{TARGETS}"),
    );
    run("without ThreshDir", &plain, None, &mut out);

    // ThreshHyst set explicitly, still without ThreshDir.
    let hyst = load(
        &tmp.path().join("b"),
        &format!(
            "ThreshMailServer: localhost\nThreshMailSender: mrtg@example.com\nThreshHyst: 0.5\n{TARGETS}"
        ),
    );
    run("ThreshHyst 0.5", &hyst, None, &mut out);

    // ThreshDir: once per transition, with state files.
    let sd = tmp.path().join("state");
    std::fs::create_dir_all(&sd).unwrap();
    let dir = load(
        &tmp.path().join("c"),
        &format!(
            "ThreshDir: {}\nThreshMailServer: localhost\nThreshMailSender: mrtg@example.com\n{TARGETS}",
            sd.display()
        ),
    );
    run("with ThreshDir", &dir, Some(&sd), &mut out);

    let expected = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thresh_mrtg.txt");
    if std::env::var_os("LINKSCOPE_RECORD").is_some() {
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, &out).unwrap();
    }
    let want = std::fs::read_to_string(&expected)
        .expect("record the transcript first (see the module docs)");
    assert_eq!(out, want, "MRTG threshold behavior changed");
}
