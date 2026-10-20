// SPDX-License-Identifier: GPL-2.0-or-later
//! Differential tests against the original MRTG Perl code.
//!
//! Each case is run through MRTG's own `readcfg`/`cfgcheck` (via
//! tests/perl/dump.pl and /root/mrtg/src/lib/mrtg2) and through this crate,
//! and the resulting structures are compared. Skips cleanly when Perl or
//! the MRTG sources are unavailable.

use std::path::{Path, PathBuf};
use std::process::Command;

use linkscope_config::{CheckOptions, ReadOptions, Source, TrustPolicy, check, read_config};
use serde_json::{Value, json};

const MRTG_LIB: &str = "/root/mrtg/src/lib/mrtg2/MRTG_lib.pm";

fn perl_available() -> bool {
    Path::new(MRTG_LIB).exists() && Command::new("perl").arg("-e1").status().is_ok_and(|s| s.success())
}

fn perl_dump(cfg: &Path) -> Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perl/dump.pl");
    let out = Command::new("perl").arg(script).arg(cfg).arg("check").output().expect("run perl");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("bad perl output for {}: {e}\n{}\n{}", cfg.display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    })
}

fn read_opts() -> ReadOptions {
    ReadOptions { trust: TrustPolicy::Off }
}

/// Compare one config file; returns a description of differences.
fn compare(cfg: &Path) -> Vec<String> {
    let mut diffs = Vec::new();
    let perl = perl_dump(cfg);
    let rust = read_config(cfg, &read_opts());
    match (&perl.get("error"), &rust) {
        (Some(e), Ok(_)) => {
            diffs.push(format!("perl readcfg failed ({}) but rust succeeded", e.as_str().unwrap_or("").trim()));
            return diffs;
        }
        (None, Err(e)) => {
            diffs.push(format!("rust read failed ({e}) but perl succeeded"));
            return diffs;
        }
        (Some(_), Err(_)) => return diffs,
        (None, Ok(_)) => {}
    }
    let raw = rust.unwrap();
    let rraw = json!({
        "global": serde_json::to_value(&raw.global).unwrap(),
        "targets": serde_json::to_value(&raw.targets).unwrap(),
        "per_target": serde_json::to_value(&raw.per_target).unwrap(),
    });
    if rraw != perl["raw"] {
        diffs.push(format!("raw differs:\n perl: {}\n rust: {}", perl["raw"], rraw));
        return diffs;
    }
    let checked = check(raw, &CheckOptions { no_create: true });
    let perl_err = perl.get("check_error").and_then(|v| v.as_str()).map(str::to_string);
    match (&perl_err, &checked) {
        (Some(e), Ok(_)) => diffs.push(format!("perl cfgcheck failed ({}) but rust succeeded", e.trim())),
        (None, Err(e)) => diffs.push(format!("rust check failed ({e}) but perl succeeded")),
        _ => {}
    }
    let Ok(cfg) = checked else { return diffs };
    if perl_err.is_some() {
        return diffs;
    }
    // Sources.
    let psrc = perl["sources"].as_array().cloned().unwrap_or_default();
    if psrc.len() != cfg.sources.len() {
        diffs.push(format!("source count perl {} rust {}", psrc.len(), cfg.sources.len()));
        return diffs;
    }
    for (i, (p, r)) in psrc.iter().zip(&cfg.sources).enumerate() {
        let rj = match r {
            Source::Exec(e) => json!({"Methode": "EXEC", "Command": e.command}),
            Source::Snmp(s) => json!({
                "Methode": "SNMP",
                "Community": s.community,
                "Host": s.host,
                "SnmpOpt": s.snmp_info,
                "Conversion": s.conversion.clone().unwrap_or_default(),
                "OID": [s.oid[0], s.oid[1]],
                "IfSel": [s.sel[0].method(), s.sel[1].method()],
                "Key": [s.sel[0].key(), s.sel[1].key()],
                "ipv4only": if s.ipv4only { 1 } else { 0 },
            }),
        };
        let mut p = p.clone();
        // Perl leaves OID undef for simple selectors in complex targets.
        if let Some(arr) = p.get_mut("OID").and_then(|v| v.as_array_mut()) {
            for v in arr.iter_mut() {
                if v.is_null() {
                    *v = json!("");
                }
            }
        }
        if p != rj {
            diffs.push(format!("source {i} differs:\n perl: {p}\n rust: {rj}"));
        }
    }
    for t in &cfg.targets {
        let pe = perl["expr"][&t.name].as_str().unwrap_or("");
        if pe != t.rewritten {
            diffs.push(format!("[{}] rewritten differs:\n perl: {pe:?}\n rust: {:?}", t.name, t.rewritten));
        }
        let pu = perl["unique"][&t.name].as_i64().unwrap_or(-99);
        let ru = t.unique_source.map(|u| u as i64).unwrap_or(-1);
        if pu != ru {
            diffs.push(format!("[{}] unique differs: perl {pu} rust {ru}", t.name));
        }
    }
    diffs
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    let body = body.replace("@WORK@", &dir.join("work").to_string_lossy());
    std::fs::write(&p, body).unwrap();
    p
}

const CORPUS: &[&str] = &[
    // Defaults, prefix, suffix, continuation, comments.
    r#"WorkDir: @WORK@
# a comment
Options[_]: growright, bits
Title[^]: Traffic for
PageTop[$]: </div>
Target[r1]: 2:public@router1
Title[r1]: Uplink
MaxBytes[r1]: 125000000
PageTop[r1]: <div><h1>Uplink</h1>
   <b>second line</b>
Target[r2]: -#Gi0/1:pub\@x@sw1.example.net:::::2
Options[r2]: gauge
Title[r2]: Gi0/1
MaxBytes[r2]: 1250000
Colors[r2]: A#00cc00,B#0000ff,C#006600,D#ff00ff
Supress[r2]: y
"#,
    // NoSpaceChar and default overridden.
    r#"WorkDir: @WORK@
NoSpaceChar: ~
Title[^]: Pre~
Title[$]: ~post
YLegend[_]: Bits per second
Target[a]: 1:c@h
Title[a]: Mid
YLegend[a]: own
MaxBytes[a]: 100
Target[b]: 3:c@h + 4:c@h
Title[b]: Sum
MaxBytes[b]: 100
"#,
    // Complex targets, ipv6, selectors, exec, arithmetic, HC and NoHC.
    r#"WorkDir: @WORK@
EnableIPv6: yes
Target[c1]: ifInErrors.3&ifOutErrors.3:pub@[2001:db8::1]:161
Title[c1]: errors
MaxBytes[c1]: 1000
Target[c2]: \Port\ 1\:2:pub@host:161:2:3:1.5:2 * 8
Title[c2]: descr
MaxBytes[c2]: 1000
NoHC[c2]: yes
Target[c3]: !0-1b-2c-3-4-5:pub@host:::::3
SnmpOptions[c3]: username=>'u', authpassword=>'aaaaaaaa'
Title[c3]: mac
MaxBytes[c3]: 1000
IPv4Only[c3]: yes
Target[c4]: `/usr/bin/printf '1\n2\n'` + /10.0.0.1:pub@host
Title[c4]: exec
MaxBytes[c4]: 1000
Target[c5]: ( %6:pub@host:::::2 ) / 2 + 1.3.6.1.2.1.2.2.1.10.7&1.3.6.1.2.1.2.2.1.16.7:pub@host
Title[c5]: mixed
MaxBytes1[c5]: 1000
MaxBytes2[c5]: 2000
RouterUptime[c5]: pub@host
"#,
    // Global continuation (MRTG ignores it), setenv, options, kmg.
    r#"WorkDir: @WORK@
Refresh: 600
   900
Target[s]: 5:x@y
Title[s]: s
MaxBytes[s]: 10
SetEnv[s]: A="1" B="two words"
kMG[s]: k, M ,G
Options[s]: dorelpercent, unknaszero
"#,
    // Errors both sides must report.
    r#"WorkDir: @WORK@
Target[e]: 1:x@y
MaxBytes[e]: 10
"#,
    r#"WorkDir: @WORK@
Target[e]: 1:x@y
Title[e]: t
MaxBytes[e]: 10
Target[e]: 2:x@y
"#,
    r#"WorkDir: @WORK@
Bogus: 1
"#,
    r#"WorkDir: @WORK@
Target[x]: 1:x@y
Title[x]: t
MaxBytes[x]: 10
XSize[x]: 900
"#,
];

#[test]
fn corpus_matches_perl() {
    if !perl_available() {
        eprintln!("skipping: perl or MRTG sources not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let mut failures = Vec::new();
    for (i, body) in CORPUS.iter().enumerate() {
        let p = write(dir.path(), &format!("c{i}.cfg"), body);
        for d in compare(&p) {
            failures.push(format!("case {i}: {d}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn includes_match_perl() {
    if !perl_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    write(dir.path(), "inc1.cfg", "Target[i1]: 1:c@h\nTitle[i1]: one\nMaxBytes[i1]: 5\n");
    let p = write(dir.path(), "top.cfg", "WorkDir: @WORK@\nOptions[_]: bits\nInclude: inc1.cfg\nTarget[t]: 2:c@h\nTitle[t]: two\nMaxBytes[t]: 5\n");
    let d = compare(&p);
    assert!(d.is_empty(), "{}", d.join("\n"));
}

/// Tiny deterministic PRNG so the test needs no extra crates.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() as usize) % xs.len()]
    }
}

#[test]
fn random_configs_match_perl() {
    if !perl_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let keys = ["Title", "PageTop", "YLegend", "ShortLegend", "Legend1", "LegendI", "Options", "MaxBytes", "Directory", "kMG"];
    let targets = ["a", "b", "C", "d.e", "f-1", "_x"];
    let values = ["foo", "bar baz", "1000", "growright", "bits, gauge", "<b>x</b>", "a~", "~b", "M"];
    let tgt_exprs = ["1:pub@h", "2:pub@h:161", "#Gi0/1:p@h:::::2", "\\eth0:p@r", "3:p@h + 4:p@h", "ifInErrors.1&ifOutErrors.1:p@h", "-5:p@h"];
    let mut rng = Lcg(0x5eed);
    let mut failures = Vec::new();
    let cases = 1500;
    for case in 0..cases {
        let mut s = String::from("WorkDir: @WORK@\n");
        if rng.next() % 3 == 0 {
            s.push_str("NoSpaceChar: ~\n");
        }
        let lines = 5 + rng.next() % 25;
        let mut defined = std::collections::HashSet::new();
        for _ in 0..lines {
            match rng.next() % 10 {
                0 => s.push_str("# comment\n"),
                1 => s.push('\n'),
                2 => {
                    let k = rng.pick(&keys);
                    let w = rng.pick(&["_", "^", "$"]);
                    let v = if rng.next() % 4 == 0 { "" } else { rng.pick(&values) };
                    s.push_str(&format!("{k}[{w}]: {v}\n"));
                }
                3 => s.push_str(&format!("   {}\n", rng.pick(&values))),
                _ => {
                    let t = rng.pick(&targets);
                    let k = rng.pick(&keys);
                    if defined.insert((t.to_lowercase(), k.to_lowercase())) {
                        s.push_str(&format!("{k}[{t}]: {}\n", rng.pick(&values)));
                    }
                }
            }
        }
        for t in targets {
            if rng.next() % 2 == 0 && defined.insert((t.to_lowercase(), "target".into())) {
                s.push_str(&format!("Target[{t}]: {}\n", rng.pick(&tgt_exprs)));
            }
        }
        let p = write(dir.path(), &format!("r{case}.cfg"), &s);
        for d in compare(&p) {
            failures.push(format!("random case {case}:\n{s}\n=> {d}"));
        }
        if failures.len() > 3 {
            break;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn contrib_configs_match_perl() {
    if !perl_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let re = regex::Regex::new(r"(?im)^(WorkDir|LogDir|HtmlDir|ImageDir|IconDir)\s*:.*$").unwrap();
    let mut failures = Vec::new();
    let mut n = 0;
    for entry in glob::glob("/root/mrtg/src/contrib/**/*.cfg").unwrap().flatten() {
        let body = std::fs::read_to_string(&entry).unwrap_or_default();
        let body = re.replace_all(&body, |c: &regex::Captures| format!("{}: {}", &c[1], work.display()));
        let p = dir.path().join(format!("contrib{n}.cfg"));
        std::fs::write(&p, body.as_ref()).unwrap();
        n += 1;
        for d in compare(&p) {
            failures.push(format!("{}: {d}", entry.display()));
        }
    }
    eprintln!("compared {n} contrib configs");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
