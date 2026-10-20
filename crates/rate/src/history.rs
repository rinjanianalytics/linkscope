// SPDX-License-Identifier: GPL-2.0-or-later
//! rateup's in-memory history array and the `NEXT` consolidation macro.

use crate::cnum::c_ll;

/// One history slot (`struct HISTORY` without the unused `percent`).
///
/// Values are `f64` because rateup stores them as `double`; integers read
/// from the log are converted with C's `long long` to `double` rounding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Hist {
    pub time: i64,
    pub inr: f64,
    pub outr: f64,
    pub inmax: f64,
    pub outmax: f64,
}

/// rateup's `history` array plus `histvalid`.
///
/// `entries` may hold more than `valid` slots: rateup reads one slot past
/// `histvalid` (and the slot after that) inside `NEXT`, and those slots can
/// hold stale data from an earlier failed read. Slots past the end of
/// `entries` read as zero, like rateup's `calloc`ed array.
#[derive(Clone, Debug, Default)]
pub(crate) struct HistBuf {
    pub entries: Vec<Hist>,
    pub valid: usize,
}

impl HistBuf {
    #[inline]
    pub fn at(&self, i: usize) -> Hist {
        self.entries.get(i).copied().unwrap_or_default()
    }

    /// Writes slot `i`, growing with zero slots as needed.
    pub fn set(&mut self, i: usize, h: Hist) {
        if i >= self.entries.len() {
            self.entries.resize(i + 1, Hist::default());
        }
        self.entries[i] = h;
    }
}

/// Output of one `NEXT(steptime)` expansion.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Agg {
    pub inr: f64,
    pub outr: f64,
    pub inmax: i64,
    pub outmax: i64,
}

/// C `max(a, b)` on a `long long` and a `double`, cast back to `long long`.
#[inline]
fn max_ll(a: i64, b: f64) -> i64 {
    let a = a as f64;
    c_ll(if a > b { a } else { b })
}

/// The `NEXT(steptime)` macro: consolidate the history that covers
/// `(now - steptime, now]`, starting the search at slot `*n`.
///
/// * If slot `n` sits exactly at `now` and slot `n+1` exactly at
///   `now - steptime`, the slot is copied verbatim (no averaging).
/// * If `now` is newer than slot `n`, rateup prints "trying to read ahead of
///   the available data" and the result is all zeros.
/// * Otherwise the overlapping slots are time-weighted: rates are averaged
///   by overlap (`inr / avc`), maxima are the maximum of the slot maxima.
///   When the covered time `avc` is 0 the average is `0/0`, which turns into
///   `i64::MIN` once rateup casts it to `long long`.
pub(crate) fn next(h: &HistBuf, n: &mut usize, now: i64, steptime: i64) -> Agg {
    let mut inmax: i64 = 0;
    let mut outmax: i64 = 0;
    let mut avc: i64 = 0;
    let mut inr = 0.0;
    let mut outr = 0.0;
    let nextnow = now.wrapping_sub(steptime);
    let valid = h.valid;
    let hn = h.at(*n);
    if now == hn.time && *n < valid && nextnow == h.at(*n + 1).time {
        inr = hn.inr;
        outr = hn.outr;
        inmax = c_ll(hn.inmax);
        outmax = c_ll(hn.outmax);
        *n += 1;
    } else if now > hn.time {
        // "Rateup is trying to read ahead of the available data"
    } else {
        while now <= h.at(*n + 1).time && *n < valid {
            *n += 1;
        }
        loop {
            let cur = h.at(*n);
            let nxt = h.at(*n + 1);
            let interval = if now >= cur.time {
                if nextnow <= nxt.time {
                    cur.time.wrapping_sub(nxt.time) as f64
                } else {
                    cur.time.wrapping_sub(nextnow) as f64
                }
            } else if nextnow > nxt.time {
                steptime as f64
            } else {
                now.wrapping_sub(nxt.time) as f64
            };
            inr += cur.inr * interval;
            outr += cur.outr * interval;
            avc = c_ll(avc as f64 + interval);
            inmax = max_ll(inmax, cur.inmax);
            outmax = max_ll(outmax, cur.outmax);
            if nextnow <= nxt.time {
                *n += 1;
            } else {
                break;
            }
            if !(*n < valid && nextnow < h.at(*n).time) {
                break;
            }
        }
        // avc != steptime would print "StepTime does not match Avc".
        inr /= avc as f64;
        outr /= avc as f64;
    }
    Agg { inr, outr, inmax, outmax }
}
