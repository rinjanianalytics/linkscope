// SPDX-License-Identifier: GPL-2.0-or-later
//! What a configuration may make the reader do through `Include:`: no more than a bounded number
//! of files and bytes in all, no FIFO or device, and a relative name read from the top
//! configuration's directory (2026-10-21).

use std::path::Path;

use linkscope_config::ReadOptions;
use linkscope_config::raw::read_config;
use linkscope_config::trust::TrustPolicy;

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

const OPTS: ReadOptions = ReadOptions { trust: TrustPolicy::Off };

#[test]
fn includes_that_fan_out_stop_at_the_budget() {
    // Five levels, each including the next 20 times: 20^5 files without a total budget.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "l4.cfg", "# leaf\n");
    for level in (0..4).rev() {
        let body = (0..20).map(|_| format!("Include: l{}.cfg\n", level + 1)).collect::<String>();
        write(dir.path(), &format!("l{level}.cfg"), &body);
    }
    let started = std::time::Instant::now();
    let err = read_config(&dir.path().join("l0.cfg"), &OPTS).unwrap_err().to_string();
    assert!(err.contains("1000 configuration files"), "{err}");
    assert!(started.elapsed().as_secs() < 10, "took {:?}", started.elapsed());
}

#[test]
fn an_include_that_is_not_a_regular_file_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("mkfifo").arg(dir.path().join("pipe.cfg")).status().unwrap().success());
    let top = write(dir.path(), "top.cfg", "Include: pipe.cfg\n");
    let err = read_config(&top, &OPTS).unwrap_err().to_string();
    assert!(err.contains("not a regular file"), "{err}");
    let top = write(dir.path(), "dev.cfg", "Include: /dev/zero\n");
    assert!(read_config(&top, &OPTS).is_err());
}

#[test]
fn a_relative_include_is_read_from_the_top_configurations_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("parts")).unwrap();
    write(dir.path(), "parts/a.cfg", "Target[a]: 1:c@h\n");
    write(dir.path(), "parts/b.cfg", "Target[b]: 2:c@h\n");
    let top = write(dir.path(), "top.cfg", "Include: parts/a.cfg\nInclude: parts/b*.cfg\n");
    let raw = read_config(&top, &OPTS).unwrap();
    assert_eq!(raw.files.len(), 3, "{:?}", raw.files);
    assert!(raw.files.iter().all(|f| f.starts_with(dir.path())), "{:?}", raw.files);
}
