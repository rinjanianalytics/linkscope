// SPDX-License-Identifier: GPL-2.0-or-later
//! Randomised differential testing against the original C `rateup`.
//!
//! Every step runs the same update through:
//! 1. the C binary on its own directory,
//! 2. a long-lived in-memory [`RateLog`] (daemon style), and
//! 3. a load / update / save cycle on a separate directory (cron style),
//!
//! and requires the `.log` (and `.old`) bytes to be identical. Periodically
//! the C binary also draws all four graphs and its printed figures are
//! compared with [`RateLog::summarize`].
//!
//! Scale with `MRTG_RATE_SEEDS` and `MRTG_RATE_STEPS`. Tests skip (pass with
//! a notice) when the reference binary is missing.

mod common;

use std::path::Path;

use common::*;
use linkscope_rate::{Mode, Period, RateLog, SummaryOpts, UpdateOutcome};

const BASE: &str = "t";
const LAB_LOG: &str = "/home/claude/build/lab/if0.log";

#[derive(Clone, Copy, Debug)]
enum Style {
    U32,
    U64,
    Wide,
    Weird,
    Gauge,
}

/// Raw-value generator for one direction.
struct Gen {
    style: Style,
    v: u128,
    rate: u128,
}

impl Gen {
    fn new(rng: &mut Rng, style: Style) -> Self {
        let rate = rng.pick(&[10u128, 1000, 1_000_000, 125_000_000, 10_000_000_000]);
        let v = match style {
            Style::U32 => rng.below128(1 << 32),
            Style::U64 => {
                if rng.chance(50) {
                    (1u128 << 64) - rng.below128(1 << 40) - 1
                } else {
                    rng.below128(1 << 64)
                }
            }
            Style::Wide => 10u128.pow(24) + rng.below128(10u128.pow(24)),
            Style::Weird | Style::Gauge => rng.below128(1_000_000),
        };
        Gen { style, v, rate }
    }

    fn next(&mut self, rng: &mut Rng, gap: i64) -> String {
        if rng.chance(4) {
            return "-1".into();
        }
        let span = self.rate * u128::from(gap.clamp(1, 7200) as u64) + 1;
        let inc = rng.below128(span);
        match self.style {
            Style::U32 | Style::U64 | Style::Wide => {
                let modulus: Option<u128> = match self.style {
                    Style::U32 => Some(1 << 32),
                    Style::U64 => Some(1 << 64),
                    _ => None,
                };
                match rng.below(100) {
                    0..=2 => self.v = self.v.saturating_sub(rng.below128(5000)),
                    3 => self.v = rng.below128(1000),
                    _ => self.v += inc,
                }
                if let Some(m) = modulus {
                    self.v %= m;
                }
                self.v.to_string()
            }
            Style::Weird => {
                self.v += inc;
                match rng.below(8) {
                    0 => format!("-{}", self.v),
                    1 => format!("000{}", self.v),
                    2 => format!("{}.{}", self.v, rng.below(100)),
                    3 => format!("{}.5", self.v / 3),
                    4 => format!("+{}", self.v),
                    _ => self.v.to_string(),
                }
            }
            Style::Gauge => match rng.below(20) {
                0 => format!("{}{:012}", rng.below(1 << 40), rng.below(1_000_000_000_000)),
                1 => format!("-{}", rng.below(1000)),
                2 => "0".into(),
                _ => rng.below128(self.rate * 300 + 1).to_string(),
            },
        }
    }
}

fn pick_gap(rng: &mut Rng) -> i64 {
    match rng.below(100) {
        0..=54 => 300,
        55..=58 => 0,
        59..=61 => 1,
        62..=64 => 299,
        65..=67 => 301,
        68..=70 => 3599,
        71..=72 => 3600,
        73..=75 => 3601,
        76..=81 => rng.below(900) as i64 + 1,
        82..=84 => 86400 * (1 + rng.below(3) as i64),
        85 => 86400 * rng.below(800) as i64 + rng.below(86400) as i64,
        86..=90 => -(rng.below(4000) as i64 + 1),
        _ => rng.below(7200) as i64,
    }
}

fn random_opts(rng: &mut Rng, dorel: bool, flags: (bool, bool, bool)) -> SummaryOpts {
    SummaryOpts {
        maxx: rng.pick(&[1usize, 50, 400, 500, 750]),
        bits: rng.chance(50),
        peak: rng.chance(50),
        withzeroes: flags.0,
        no_i: flags.1,
        no_o: flags.2,
        dorelpercent: dorel,
    }
}

#[derive(Clone, Copy, Debug)]
enum Start {
    Fresh,
    Lab,
    LineOnly,
    ThreeColumn,
    CorruptLogGoodOld,
}

/// Writes the starting files into `dir`; returns the initial time.
fn setup(start: Start, dir: &Path, rng: &mut Rng) -> i64 {
    let w = |name: &str, data: &[u8]| std::fs::write(dir.join(name), data).unwrap();
    match start {
        Start::Fresh => 1_000_000_000 + rng.below(900_000_000) as i64,
        Start::Lab => {
            w("t.log", &std::fs::read(LAB_LOG).unwrap());
            1_760_006_200 + rng.below(4000) as i64
        }
        Start::LineOnly => {
            let t = 1_500_000_000 + rng.below(1_000_000) as i64;
            w("t.log", format!("{t} 1000 2000\n").as_bytes());
            t + 300
        }
        Start::ThreeColumn => {
            // Old MRTG 2.x format: some lines have only three numbers, and
            // some have max < avg (rateup raises max to avg).
            let t = 1_600_000_000 + rng.below(1_000_000) as i64;
            let mut s = format!("{t} 5000 6000\n");
            let mut tt = t;
            for i in 0..900 {
                let a = rng.below(100_000);
                let b = rng.below(100_000);
                if i % 3 == 0 {
                    s += &format!("{tt} {a} {b}\n");
                } else {
                    s += &format!("{tt} {a} {b} {} {}\n", a / 2, b + 5);
                }
                tt -= if i < 300 { 300 } else { 1800 };
            }
            w("t.log", s.as_bytes());
            t + rng.below(600) as i64
        }
        Start::CorruptLogGoodOld => {
            let mut lab = std::fs::read(LAB_LOG).unwrap();
            w("t.old", &lab);
            lab.truncate(lab.len() / 2);
            lab[0] = b'#';
            w("t.log", &lab);
            1_760_006_200 + rng.below(4000) as i64
        }
    }
}

/// One full randomized scenario. Returns the number of compared steps.
fn scenario(bin: &Path, seed: u64, steps: u64) -> u64 {
    let mut rng = Rng::new(seed);
    let starts = [Start::Fresh, Start::Fresh, Start::Lab, Start::LineOnly, Start::ThreeColumn, Start::CorruptLogGoodOld];
    let mut start = starts[(seed % starts.len() as u64) as usize];
    if !Path::new(LAB_LOG).exists() && matches!(start, Start::Lab | Start::CorruptLogGoodOld) {
        start = Start::Fresh;
    }
    let cdir = TempDir::new("c");
    let rdir = TempDir::new("r");
    let mut t = setup(start, cdir.path(), &mut Rng::new(seed ^ 0xABCD));
    setup(start, rdir.path(), &mut Rng::new(seed ^ 0xABCD));
    let mode = rng.pick(&[Mode::Counter, Mode::Counter, Mode::Absolute, Mode::Gauge, Mode::PerHour, Mode::PerMinute]);
    let style = match mode {
        Mode::Gauge | Mode::Absolute if rng.chance(70) => Style::Gauge,
        _ => rng.pick(&[Style::U32, Style::U64, Style::Wide, Style::Weird]),
    };
    let zero = rng.chance(50);
    let abs_max = rng.pick(&[i64::MAX, 1_000_000_000_000, 125_000_000, 10_000, 1 << 32]);
    let mut gi = Gen::new(&mut rng, style);
    let mut go = Gen::new(&mut rng, style);
    let ctx = format!("seed={seed} start={start:?} mode={mode:?} style={style:?} zero={zero} abs_max={abs_max}");

    // The daemon loads once, at its first poll (fresh logs depend on it).
    let mut daemon: Option<RateLog> = None;
    // render() of a log that was only loaded is a normalised re-rendering,
    // not rateup output; compare the daemon once it has written something.
    let mut daemon_wrote = false;
    let mut compared = 0;
    for step in 0..steps {
        let gap = pick_gap(&mut rng);
        let now = t + gap;
        let u = Upd { now, input: gi.next(&mut rng, gap), output: go.next(&mut rng, gap), abs_max, mode, zero };
        let with_images = step % 41 == 40 || step + 1 == steps;
        let dorel = rng.chance(30);
        let gflags = (rng.chance(30), rng.chance(20), rng.chance(20));
        let imgs: Vec<Img> = if with_images {
            Period::ALL.iter().map(|&p| Img { period: p, opts: random_opts(&mut rng, dorel, gflags) }).collect()
        } else {
            Vec::new()
        };
        let flags = imgs.first().map(|i| flags_for(&i.opts)).unwrap_or_default();
        let (ok, printed) = run_c(bin, cdir.path(), BASE, now, Some(&u), &flags, &imgs);
        assert!(ok, "{ctx} step={step}: rateup failed");

        // Daemon style.
        let daemon = daemon.get_or_insert_with(|| RateLog::load(rdir.path(), BASE, now).expect("initial load"));
        let r1 = daemon.update(u.now, &u.input, &u.output, u.abs_max, u.mode, u.zero);
        assert!(!matches!(r1, UpdateOutcome::Rejected(_)), "{ctx} step={step}: rejected {u:?}");
        // Cron style.
        let mut cron = RateLog::load(rdir.path(), BASE, now).expect("reload");
        let r2 = cron.update(u.now, &u.input, &u.output, u.abs_max, u.mode, u.zero);
        assert_eq!(r1, r2, "{ctx} step={step}");
        if r2 == UpdateOutcome::Updated {
            cron.save(rdir.path(), BASE).expect("save");
            t = now;
        }

        let clog = read_opt(&cdir.path().join("t.log")).expect("C log exists");
        daemon_wrote |= r1 == UpdateOutcome::Updated;
        let dlog = if daemon_wrote { daemon.render_bytes() } else { clog.clone() };
        assert!(dlog == clog, "{ctx} step={step} {u:?} daemon differs: {}", first_diff(&dlog, &clog));
        let rlog = read_opt(&rdir.path().join("t.log")).expect("rust log exists");
        assert!(rlog == clog, "{ctx} step={step} {u:?} cron differs: {}", first_diff(&rlog, &clog));
        assert!(
            read_opt(&rdir.path().join("t.old")) == read_opt(&cdir.path().join("t.old")),
            "{ctx} step={step}: .old differs"
        );

        for (k, im) in imgs.iter().enumerate() {
            for (who, log) in [("daemon", &*daemon), ("cron", &cron)] {
                let s = log.summarize(now, im.period, &im.opts);
                assert_eq!(s.points.len(), im.opts.maxx);
                assert_eq!(
                    summary_lines(&s),
                    printed[k],
                    "{ctx} step={step} {who} summary {:?} {:?}",
                    im.period,
                    im.opts
                );
            }
        }
        // Graphs without an update: summarize on a freshly loaded log.
        if step % 53 == 52 {
            let gflags = (rng.chance(30), rng.chance(20), rng.chance(20));
            let imgs: Vec<Img> =
                Period::ALL.iter().map(|&p| Img { period: p, opts: random_opts(&mut rng, false, gflags) }).collect();
            let at = t + rng.below(600) as i64;
            let (ok, printed) = run_c(bin, cdir.path(), BASE, at, None, &flags_for(&imgs[0].opts), &imgs);
            assert!(ok);
            let fresh = RateLog::load(cdir.path(), BASE, at).unwrap();
            for (k, im) in imgs.iter().enumerate() {
                let s = fresh.summarize(at, im.period, &im.opts);
                assert_eq!(summary_lines(&s), printed[k], "{ctx} step={step} load-only summary {:?}", im.period);
            }
        }
        compared += 1;
    }
    compared
}

fn run_seeds(range: std::ops::Range<u64>) {
    let Some(bin) = rateup() else { return };
    let per = env_num("MRTG_RATE_SEEDS", 0);
    let steps = env_num("MRTG_RATE_STEPS", 300);
    // MRTG_RATE_SEEDS scales the number of seeds handled by each test.
    let range = if per > 0 { range.start * per..range.start * per + per } else { range };
    let mut total = 0;
    let n = range.end - range.start;
    for seed in range {
        total += scenario(&bin, seed, steps);
    }
    eprintln!("compared {total} steps over {n} seeds byte-for-byte");
}

#[test]
fn differential_seeds_a() {
    run_seeds(0..6);
}

#[test]
fn differential_seeds_b() {
    run_seeds(6..12);
}

#[test]
fn differential_seeds_c() {
    run_seeds(12..18);
}

#[test]
fn differential_seeds_d() {
    run_seeds(18..24);
}
