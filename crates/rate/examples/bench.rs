// SPDX-License-Identifier: GPL-2.0-or-later
//! Rough timing of load, update and render on a full-size log.
//!
//! `cargo run --release -p linkscope-rate --example bench`

use std::hint::black_box;
use std::time::Instant;

use linkscope_rate::{Mode, Period, RateLog, SummaryOpts};

fn main() {
    let n = 5000u32;
    let mut log = RateLog::fresh(1_700_000_000);
    let mut t = 1_700_000_000i64;
    let mut c = 0u64;
    // Warm up to a fully populated history.
    for _ in 0..800 {
        t += 300;
        c += 123_456_789;
        log.update(
            t,
            &c.to_string(),
            &c.to_string(),
            i64::MAX,
            Mode::Counter,
            false,
        );
    }
    let bytes = log.render_bytes();
    println!(
        "log: {} lines, {} bytes",
        log.history_len() + 1,
        bytes.len()
    );

    let start = Instant::now();
    for _ in 0..n {
        black_box(RateLog::from_bytes(Some(black_box(&bytes)), None, t).unwrap());
    }
    report("load (parse)", start, n);

    let start = Instant::now();
    for _ in 0..n {
        t += 300;
        c += 987_654_321;
        let s = c.to_string();
        log.update(t, &s, &s, i64::MAX, Mode::Counter, false);
    }
    report("update", start, n);

    let start = Instant::now();
    for _ in 0..n {
        black_box(log.render());
    }
    report("render", start, n);

    let start = Instant::now();
    for _ in 0..n {
        t += 300;
        c += 987_654_321;
        let s = c.to_string();
        log.update(t, &s, &s, i64::MAX, Mode::Counter, false);
        black_box(log.render_bytes());
    }
    report("update + render", start, n);

    let opts = SummaryOpts::default();
    let start = Instant::now();
    for _ in 0..n {
        for p in Period::ALL {
            black_box(log.summarize(t, p, &opts));
        }
    }
    report("summarize x4 periods", start, n);

    let dir = std::env::temp_dir().join(format!("linkscope-rate-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    log.save(&dir, "b").unwrap();
    let m = 500;
    let start = Instant::now();
    for _ in 0..m {
        t += 300;
        let mut l = RateLog::load(&dir, "b", t).unwrap();
        l.update(t, "1", "2", i64::MAX, Mode::Counter, false);
        l.save(&dir, "b").unwrap();
    }
    report("load + update + save (fsync, disk)", start, m);
    let _ = std::fs::remove_dir_all(&dir);
}

fn report(what: &str, start: Instant, n: u32) {
    let per = start.elapsed() / n;
    println!("{what:<40} {per:>10.2?} per op");
}
