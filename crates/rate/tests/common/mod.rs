// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared helpers for the differential tests against the C rateup binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use linkscope_rate::{Mode, Period, Summary, SummaryOpts};

/// Location of the reference binary (override with `MRTG_RATEUP`).
pub fn rateup_path() -> PathBuf {
    std::env::var_os("MRTG_RATEUP")
        .map_or_else(|| PathBuf::from("/home/claude/build/rateup"), PathBuf::from)
}

/// Returns the binary path, or prints a notice and returns `None` so the
/// calling test can return early (skip) when the binary is missing.
pub fn rateup() -> Option<PathBuf> {
    let p = rateup_path();
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: reference rateup binary not found at {}", p.display());
        None
    }
}

/// Number from an environment variable, with a default.
pub fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Small deterministic PRNG (SplitMix64); no external crates needed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
    pub fn below128(&mut self, n: u128) -> u128 {
        if n == 0 {
            0
        } else {
            ((u128::from(self.next_u64()) << 64) | u128::from(self.next_u64())) % n
        }
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    pub fn pick<T: Copy>(&mut self, s: &[T]) -> T {
        s[self.below(s.len() as u64) as usize]
    }
}

/// A temporary directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "linkscope-rate-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("create temp dir");
        TempDir(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn read_opt(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok()
}

/// One rateup update request.
#[derive(Clone, Debug)]
pub struct Upd {
    pub now: i64,
    pub input: String,
    pub output: String,
    pub abs_max: i64,
    pub mode: Mode,
    pub zero: bool,
}

/// Graph request for one period.
#[derive(Clone, Copy, Debug)]
pub struct Img {
    pub period: Period,
    pub opts: SummaryOpts,
}

/// Runs the C binary: optional update then image records. Returns
/// (exit status success, parsed stdout numbers per image).
pub fn run_c(
    bin: &Path,
    dir: &Path,
    base: &str,
    now: i64,
    upd: Option<&Upd>,
    flags: &[&str],
    imgs: &[Img],
) -> (bool, Vec<Vec<i64>>) {
    let mut cmd = Command::new(bin);
    cmd.arg(dir).arg(base).arg(now.to_string());
    let dorel = imgs.first().is_some_and(|i| i.opts.dorelpercent);
    if let Some(u) = upd {
        cmd.arg(if u.zero { "-z" } else { "-Z" });
        let mut letter = u.mode.letter().to_string();
        if dorel {
            letter.push('p');
        }
        cmd.arg(letter)
            .arg(&u.input)
            .arg(&u.output)
            .arg(u.abs_max.to_string());
    }
    cmd.args(flags);
    for (k, im) in imgs.iter().enumerate() {
        let o = im.opts;
        cmd.arg(if o.peak { "p" } else { "i" })
            .arg(dir.join(format!("img{k}.png")))
            .arg("1000000")
            .arg("1000000")
            .arg(o.maxx.to_string())
            .arg("100")
            .arg("1")
            .arg("1")
            .arg("0")
            .arg(im.period.step().to_string())
            .arg(if o.bits { "1" } else { "0" })
            .arg("4")
            .arg("1")
            .arg("")
            .arg("0");
    }
    let out = cmd.output().expect("spawn rateup");
    let text = String::from_utf8_lossy(&out.stdout);
    let nums: Vec<i64> = text
        .lines()
        .map(|l| l.trim().parse().expect("rateup printed a non-number"))
        .collect();
    let mut per = Vec::new();
    let mut it = nums.into_iter();
    for im in imgs {
        let k = if im.opts.dorelpercent { 11 } else { 8 };
        per.push(it.by_ref().take(k).collect());
    }
    (out.status.success(), per)
}

/// rateup's printed lines for a [`Summary`].
pub fn summary_lines(s: &Summary) -> Vec<i64> {
    let mut v = vec![s.max_in, s.max_out];
    v.extend(s.max_pct);
    v.extend([s.avg_in, s.avg_out]);
    v.extend(s.avg_pct);
    v.extend([s.cur_in, s.cur_out]);
    v.extend(s.cur_pct);
    v.extend([s.avg_max_in, s.avg_max_out]);
    v
}

/// The command-line flags for summary options that are global in rateup.
pub fn flags_for(o: &SummaryOpts) -> Vec<&'static str> {
    let mut f = Vec::new();
    if o.withzeroes {
        f.push("-0");
    }
    if o.no_i {
        f.push("-i");
    }
    if o.no_o {
        f.push("-o");
    }
    f
}

/// Shows the first differing line of two logs.
pub fn first_diff(a: &[u8], b: &[u8]) -> String {
    let la: Vec<&[u8]> = a.split(|&c| c == b'\n').collect();
    let lb: Vec<&[u8]> = b.split(|&c| c == b'\n').collect();
    for i in 0..la.len().max(lb.len()) {
        let x = la.get(i).copied().unwrap_or(b"<eof>");
        let y = lb.get(i).copied().unwrap_or(b"<eof>");
        if x != y {
            return format!(
                "line {}: rust={:?} c={:?}",
                i + 1,
                String::from_utf8_lossy(x),
                String::from_utf8_lossy(y)
            );
        }
    }
    "no line differs".into()
}
