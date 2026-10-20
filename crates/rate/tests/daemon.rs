// SPDX-License-Identifier: GPL-2.0-or-later
//! A long-running daemon that keeps one `RateLog` in memory must produce the
//! same logs (and graph figures) as one rateup process per poll, which
//! re-reads the integers it wrote each time.

mod common;

use common::*;
use linkscope_rate::{Mode, Period, RateLog, SummaryOpts, UpdateOutcome};

const STEPS: usize = 600;

/// Deterministic but irregular poll sequence with fractional averages
/// (gaps that are not multiples of 300 make consolidation produce
/// non-integers, which is where truncation on re-read matters).
fn polls() -> Vec<(i64, String, String)> {
    let mut rng = Rng::new(42);
    let mut t = 1_700_000_123i64;
    let (mut a, mut b) = (4_000_000_000u64, 17u64);
    let mut v = Vec::with_capacity(STEPS);
    for i in 0..STEPS {
        t += match i % 7 {
            0 => 301,
            1 => 299,
            2 => 1 + rng.below(700) as i64,
            3 => 300,
            4 => 3601,
            _ => 157 + rng.below(300) as i64,
        };
        a = (a + rng.below(10_000_000)) % (1 << 32);
        b += rng.below(1_000_000_007);
        let ins = if i % 50 == 7 { "-1".to_string() } else { a.to_string() };
        v.push((t, ins, b.to_string()));
    }
    v
}

#[test]
fn daemon_matches_reload_and_c_600_steps() {
    let polls = polls();
    let rdir = TempDir::new("daemon-r");
    let mut daemon = RateLog::load(rdir.path(), "t", polls[0].0).unwrap();
    let opts = SummaryOpts { maxx: 400, peak: true, ..SummaryOpts::default() };
    for (i, (now, a, b)) in polls.iter().enumerate() {
        assert_eq!(daemon.update(*now, a, b, i64::MAX, Mode::Counter, false), UpdateOutcome::Updated);
        // Cron style: load what the previous step saved, update, save.
        let mut cron = RateLog::load(rdir.path(), "t", *now).unwrap();
        assert_eq!(cron.update(*now, a, b, i64::MAX, Mode::Counter, false), UpdateOutcome::Updated);
        cron.save(rdir.path(), "t").unwrap();
        let d = daemon.render();
        assert_eq!(d, cron.render(), "step {i}: daemon and save/load differ");
        assert_eq!(d.as_bytes(), std::fs::read(rdir.path().join("t.log")).unwrap(), "step {i}");
        assert_eq!(daemon.last_raw(), (a.as_str(), b.as_str()));
        for p in Period::ALL {
            assert_eq!(daemon.summarize(*now, p, &opts), cron.summarize(*now, p, &opts), "step {i} {p:?}");
        }
    }

    // The same 600 polls through the C binary give the same final log, and
    // every intermediate log matches too.
    let Some(bin) = rateup() else { return };
    let cdir = TempDir::new("daemon-c");
    let mut again = RateLog::load(cdir.path(), "t", polls[0].0).unwrap();
    for (i, (now, a, b)) in polls.iter().enumerate() {
        let u = Upd { now: *now, input: a.clone(), output: b.clone(), abs_max: i64::MAX, mode: Mode::Counter, zero: false };
        let imgs: Vec<Img> = if i % 100 == 99 {
            Period::ALL.iter().map(|&period| Img { period, opts }).collect()
        } else {
            Vec::new()
        };
        let (ok, printed) = run_c(&bin, cdir.path(), "t", *now, Some(&u), &[], &imgs);
        assert!(ok);
        again.update(*now, a, b, i64::MAX, Mode::Counter, false);
        let c = std::fs::read(cdir.path().join("t.log")).unwrap();
        assert!(again.render_bytes() == c, "step {i}: {}", first_diff(&again.render_bytes(), &c));
        for (k, im) in imgs.iter().enumerate() {
            assert_eq!(summary_lines(&again.summarize(*now, im.period, &im.opts)), printed[k], "step {i}");
        }
    }
    assert_eq!(again.render(), daemon.render());
}

#[test]
fn summarize_after_update_uses_untruncated_history() {
    // Right after an update rateup draws graphs from doubles that were not
    // yet truncated; after a reload it sees integers. Both are reproduced.
    let mut log = RateLog::fresh(1_700_000_000);
    let mut t = 1_700_000_000;
    let mut c = 0u64;
    for i in 0..60u64 {
        t += 211 + (i % 4) as i64 * 37;
        c += 1_000_003 * (i % 5 + 1);
        log.update(t, &c.to_string(), &c.to_string(), i64::MAX, Mode::Counter, false);
    }
    let reloaded = RateLog::from_bytes(Some(log.render().as_bytes()), None, t).unwrap();
    let o = SummaryOpts { maxx: 50, ..SummaryOpts::default() };
    let a = log.summarize(t, Period::Week, &o);
    let b = reloaded.summarize(t, Period::Week, &o);
    assert_ne!(a.points, b.points, "un-truncated averages should differ from truncated ones");
    assert!(a.points.iter().any(|p| p.r#in.fract() != 0.0));
    assert!(b.points.iter().all(|p| p.r#in.fract() == 0.0 || p.r#in.is_nan()) || b.points.len() == 50);
    // An update that is refused as a time warp behaves like a fresh run.
    let mut warped = log.clone();
    assert_eq!(warped.update(t - 5, "1", "1", i64::MAX, Mode::Counter, false), UpdateOutcome::TimeWarp);
    assert_eq!(warped.summarize(t, Period::Week, &o), b);
}
