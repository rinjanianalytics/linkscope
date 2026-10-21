// SPDX-License-Identifier: GPL-2.0-or-later
//! `linkscope-rate`: a faithful port of MRTG's `rateup` rate engine.
//!
//! MRTG keeps one `<name>.log` file per target. Line 1 holds the time of the
//! last update and the raw counter strings seen then (`time in out`). Every
//! further line is `time avg_in avg_out max_in max_out`, newest first,
//! consolidated into four resolutions:
//!
//! | section | step   | lines |
//! |---------|--------|-------|
//! | day     | 300 s  | 600   |
//! | week    | 1800 s | 600   |
//! | month   | 7200 s | 600   |
//! | year    | 1 day  | 732   |
//!
//! plus up to one partial line before each section that realigns the time to
//! the next step boundary. `rateup` is a fresh process on every poll: it
//! reads the log ([`RateLog::load`]), turns the new counters into a rate and
//! rewrites the whole file ([`RateLog::update`], [`RateLog::render`],
//! [`RateLog::save`]), and optionally prints the max / average / current
//! figures that MRTG puts on its HTML pages ([`RateLog::summarize`]).
//!
//! This crate reproduces rateup byte for byte, including its quirks:
//!
//! * Counters are subtracted as decimal strings, so counters wider than 64
//!   bits work; the difference is a `double`.
//! * A negative difference greater than `-2^32` is treated as a 32-bit wrap
//!   (`+ 2^32`); anything more negative becomes 0 (a 64-bit reset).
//! * Rates are rounded with `+ .5` and truncated to `long long`. Values that
//!   do not fit (or NaN) become `i64::MIN`, as on x86-64.
//! * Gaps of 0 s or more than 3600 s (and the first update after creating a
//!   fresh log) do not compute a rate: the previous rate is repeated (or 0
//!   with `unknown_as_zero`).
//! * An input of exactly `-1` means unknown. If the current *or previous*
//!   raw value is `-1` the previous rate is repeated (or 0). This applies to
//!   gauges too.
//! * Rates below 0 or above `abs_max` are replaced in the same way.
//! * Each run re-reads the integers it wrote. A daemon that keeps a
//!   [`RateLog`] in memory gets exactly the same results as one process per
//!   poll, because the in-memory history is the re-read (truncated) one.
//! * [`RateLog::summarize`] right after [`RateLog::update`] uses the
//!   un-truncated averages and the previous `histvalid`, exactly like a
//!   rateup invocation that both updates and draws graphs.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cnum;
mod diff;
mod history;
mod load;
mod save;

use std::fmt;
use std::path::Path;

use cnum::{c_ll, push_i64};
use diff::diff;
use history::{Agg, Hist, HistBuf, next};
use load::{Loader, ReadCode};

pub(crate) const DAY_COUNT: usize = 600;
pub(crate) const DAY_SAMPLE: i64 = 5 * 60;
pub(crate) const WEEK_COUNT: usize = 600;
pub(crate) const WEEK_SAMPLE: i64 = 30 * 60;
pub(crate) const MONTH_COUNT: usize = 600;
pub(crate) const MONTH_SAMPLE: i64 = 2 * 60 * 60;
pub(crate) const YEAR_COUNT: usize = 2 * 366;
pub(crate) const YEAR_SAMPLE: i64 = 24 * 60 * 60;
pub(crate) const MAX_HISTORY: usize = DAY_COUNT + WEEK_COUNT + MONTH_COUNT + YEAR_COUNT + 10;
/// History lines read from a log at most. A log rateup writes has fewer than [`MAX_HISTORY`];
/// rateup grows its array for longer ones, and this port did too, without bound: 4 MB of blank
/// lines (each repeats the line before) became about 170 MB of history. Sixteen times a full log
/// leaves every log rateup can write, and far longer ones, read as before.
pub(crate) const MAX_READ_HISTORY: usize = 16 * MAX_HISTORY;

/// Longest `in`/`out` string accepted by [`RateLog::update`]. rateup re-reads
/// line 1 with `%199s`, and its `diff()` overflows its 200-byte buffer for
/// operands longer than 197 bytes (which [`RateLog::update`] reports as
/// [`InputError::TooLongForDiff`]).
pub const MAX_RAW_LEN: usize = 199;

/// Longest base name rateup accepts ("Too long basename").
pub const MAX_BASE_LEN: usize = 120;

/// How raw values are turned into rates (the rateup update letter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `u`: a monotonically increasing counter; rate = delta / seconds.
    Counter,
    /// `a`: the value is already a delta since the last poll (counter reset
    /// on read); rate = value / seconds.
    Absolute,
    /// `g`: the value is the rate itself (`gauge`).
    Gauge,
    /// `h`: counter delta scaled to units per hour (`perhour`).
    PerHour,
    /// `m`: counter delta scaled to units per minute (`perminute`).
    PerMinute,
}

impl Mode {
    /// The rateup command letter for this mode.
    pub fn letter(self) -> char {
        match self {
            Mode::Counter => 'u',
            Mode::Absolute => 'a',
            Mode::Gauge => 'g',
            Mode::PerHour => 'h',
            Mode::PerMinute => 'm',
        }
    }

    /// Parses a rateup command letter (`u a g h m`).
    pub fn from_letter(c: char) -> Option<Mode> {
        Some(match c {
            'u' => Mode::Counter,
            'a' => Mode::Absolute,
            'g' => Mode::Gauge,
            'h' => Mode::PerHour,
            'm' => Mode::PerMinute,
            _ => return None,
        })
    }

    /// Modes where the raw value is a running counter that gets diffed
    /// against the previous one (rateup `absupdate` 0, 3, 4).
    fn is_counter(self) -> bool {
        matches!(self, Mode::Counter | Mode::PerHour | Mode::PerMinute)
    }
}

/// Graph period: which resolution [`RateLog::summarize`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    /// Daily graph, 300 s per pixel.
    Day,
    /// Weekly graph, 1800 s per pixel.
    Week,
    /// Monthly graph, 7200 s per pixel.
    Month,
    /// Yearly graph, 86400 s per pixel.
    Year,
}

impl Period {
    /// Seconds per graph column (the `step` rateup receives).
    pub fn step(self) -> i64 {
        match self {
            Period::Day => DAY_SAMPLE,
            Period::Week => WEEK_SAMPLE,
            Period::Month => MONTH_SAMPLE,
            Period::Year => YEAR_SAMPLE,
        }
    }

    /// All four periods in MRTG's order.
    pub const ALL: [Period; 4] = [Period::Day, Period::Week, Period::Month, Period::Year];
}

/// Errors from [`RateLog::load`].
#[derive(Debug)]
pub enum RateError {
    /// Both `<base>.log` and `<base>.old` were unusable and the backup had
    /// unsorted (or corrupt) history lines. rateup exits with status 1.
    Corrupt {
        /// The base name of the log.
        base: String,
    },
    /// The base name is longer than [`MAX_BASE_LEN`] bytes.
    BaseTooLong,
}

impl fmt::Display for RateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RateError::Corrupt { base } => write!(
                f,
                "log file for {base} was corrupt or not in sorted order, and so was the backup"
            ),
            RateError::BaseTooLong => write!(f, "base name longer than {MAX_BASE_LEN} bytes"),
        }
    }
}

impl std::error::Error for RateError {}

/// Why [`RateLog::update`] refused an input. These are inputs for which the
/// C rateup has undefined behaviour or would write a log it cannot re-read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    /// Empty `in` or `out` string.
    Empty,
    /// Longer than [`MAX_RAW_LEN`] bytes.
    TooLong,
    /// Contains white space or NUL (would corrupt line 1 of the log).
    BadChar,
    /// The value had to be diffed but it (or the previous raw value) has no
    /// digit at all: rateup's `diff()` would scan past the string.
    NoDigits,
    /// The value had to be diffed but is longer than rateup's `diff()`
    /// buffer allows (more than 197 bytes).
    TooLongForDiff,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InputError::Empty => "empty value",
            InputError::TooLong => "value longer than 199 bytes",
            InputError::BadChar => "value contains white space or NUL",
            InputError::NoDigits => "value has no digits",
            InputError::TooLongForDiff => "value too long to diff",
        })
    }
}

impl std::error::Error for InputError {}

/// Result of [`RateLog::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The log was rewritten; [`RateLog::render`] gives the new file.
    Updated,
    /// `now` is older than the log's last update ("Let's not do the time
    /// warp, again"). The log is unchanged and should not be saved.
    TimeWarp,
    /// The input was refused; the log is unchanged.
    Rejected(InputError),
}

/// One rendered history line: `time in out inmax outmax` as printed.
#[derive(Clone, Copy, Debug)]
struct Row {
    time: i64,
    vals: [i64; 4],
}

/// A target's rate log: line 1 (last update) plus the consolidated history.
#[derive(Clone, Debug)]
pub struct RateLog {
    last_time: i64,
    last_in: Vec<u8>,
    last_out: Vec<u8>,
    /// History exactly as a fresh rateup process would read it back.
    hist: HistBuf,
    /// Lines written by the last update (`None` right after a load).
    rows: Option<Vec<Row>>,
    /// rateup's in-process history right after `update()` (un-truncated
    /// averages, old `histvalid`), used by `summarize`.
    view: Option<HistBuf>,
    /// Scratch buffer for the new history of an update.
    scratch: Vec<Hist>,
}

/// Options for [`RateLog::summarize`] (rateup's graph options).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryOpts {
    /// Number of graph columns (`XSize`).
    pub maxx: usize,
    /// Multiply everything by 8 (`Options[]: bits`).
    pub bits: bool,
    /// Peak graph (`WithPeak[]`): maxima come from the per-column maxima
    /// instead of the averages.
    pub peak: bool,
    /// Count all-zero columns as samples (`Options[]: withzeroes`).
    pub withzeroes: bool,
    /// Ignore the first variable for scaling (`Options[]: noi`).
    pub no_i: bool,
    /// Ignore the second variable for scaling (`Options[]: noo`).
    pub no_o: bool,
    /// Also report in/out percentages (`Options[]: dorelpercent`).
    pub dorelpercent: bool,
}

impl Default for SummaryOpts {
    fn default() -> Self {
        SummaryOpts {
            maxx: 400,
            bits: false,
            peak: false,
            withzeroes: false,
            no_i: false,
            no_o: false,
            dorelpercent: false,
        }
    }
}

/// One graph column: the consolidated values for `(time - step, time]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Column end time (newest edge).
    pub time: i64,
    /// Average input rate (times 8 with `bits`).
    pub r#in: f64,
    /// Average output rate (times 8 with `bits`).
    pub out: f64,
    /// Maximum input rate (times 8 with `bits`).
    pub in_max: f64,
    /// Maximum output rate (times 8 with `bits`).
    pub out_max: f64,
}

/// The figures rateup prints after drawing a graph, which MRTG shows as
/// "Max", "Average" and "Current" on the HTML page.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Maximum input rate (of averages, or of maxima for a peak graph).
    pub max_in: i64,
    /// Maximum output rate.
    pub max_out: i64,
    /// Maximum of `100 * in / out` (only with `dorelpercent`).
    pub max_pct: Option<i64>,
    /// Average input rate over the sampled columns.
    pub avg_in: i64,
    /// Average output rate over the sampled columns.
    pub avg_out: i64,
    /// `100 * avg_in / avg_out` (only with `dorelpercent`).
    pub avg_pct: Option<i64>,
    /// Newest column's input rate.
    pub cur_in: i64,
    /// Newest column's output rate.
    pub cur_out: i64,
    /// Newest column's `100 * in / out` (only with `dorelpercent`).
    pub cur_pct: Option<i64>,
    /// Average of the per-column input maxima.
    pub avg_max_in: i64,
    /// Average of the per-column output maxima.
    pub avg_max_out: i64,
    /// The `maxx` columns, newest first.
    pub points: Vec<Point>,
}

impl RateLog {
    /// Loads `<dir>/<base>.log` with rateup's `readfile()` semantics.
    ///
    /// If the log is missing or its first line is unreadable, or its history
    /// is unsorted, `<base>.old` is tried. If that fails too, a fresh history
    /// is created: 2532 zero slots aligned on the day/week/month/year steps
    /// ending one day-step before `now`, with the "no previous counter"
    /// marker so the first update repeats the (zero) rate.
    ///
    /// Returns an error only where rateup exits: the backup exists but is
    /// unsorted, or the base name is too long.
    pub fn load(dir: &Path, base: &str, now: i64) -> Result<RateLog, RateError> {
        if base.len() > MAX_BASE_LEN {
            return Err(RateError::BaseTooLong);
        }
        let log = save::read_file(&dir.join(format!("{base}.log")));
        let mut ld = Loader::new();
        if ld.readhist(log.as_deref()) == ReadCode::Ok {
            return Ok(Self::from_loader(ld));
        }
        drop(log);
        let old = save::read_file(&dir.join(format!("{base}.old")));
        Self::finish_fallback(ld, old.as_deref(), now, base)
    }

    /// Like [`RateLog::load`] but from in-memory file contents; `None` means
    /// the file could not be opened. Useful for tests and fuzzing.
    pub fn from_bytes(
        log: Option<&[u8]>,
        old: Option<&[u8]>,
        now: i64,
    ) -> Result<RateLog, RateError> {
        let mut ld = Loader::new();
        if ld.readhist(log) == ReadCode::Ok {
            return Ok(Self::from_loader(ld));
        }
        Self::finish_fallback(ld, old, now, "")
    }

    /// A brand-new log, as rateup creates when neither file exists.
    pub fn fresh(now: i64) -> RateLog {
        let mut ld = Loader::new();
        ld.create_fresh(now);
        Self::from_loader(ld)
    }

    fn finish_fallback(
        mut ld: Loader,
        old: Option<&[u8]>,
        now: i64,
        base: &str,
    ) -> Result<RateLog, RateError> {
        match ld.readhist(old) {
            ReadCode::Ok => {}
            ReadCode::Unsorted => {
                return Err(RateError::Corrupt {
                    base: base.to_owned(),
                });
            }
            ReadCode::Unreadable => ld.create_fresh(now),
        }
        Ok(Self::from_loader(ld))
    }

    fn from_loader(ld: Loader) -> RateLog {
        RateLog {
            last_time: ld.last_time,
            last_in: ld.last_in,
            last_out: ld.last_out,
            hist: ld.hist,
            rows: None,
            view: None,
            scratch: Vec::new(),
        }
    }

    /// Time of the last update (line 1 of the log).
    pub fn last_time(&self) -> i64 {
        self.last_time
    }

    /// Raw `in`/`out` strings of the last update (line 1 of the log). Bytes
    /// that are not UTF-8 (only possible in a hand-edited log) yield `""`;
    /// see [`RateLog::last_raw_bytes`].
    pub fn last_raw(&self) -> (&str, &str) {
        (
            std::str::from_utf8(&self.last_in).unwrap_or(""),
            std::str::from_utf8(&self.last_out).unwrap_or(""),
        )
    }

    /// Raw `in`/`out` bytes of the last update.
    pub fn last_raw_bytes(&self) -> (&[u8], &[u8]) {
        (&self.last_in, &self.last_out)
    }

    /// Forget the last update's raw values, as if there were no previous counter (rateup's `x`
    /// marker): the next update repeats the last rate instead of taking a difference. For a log
    /// whose line 1 holds a value too long to diff (only a damaged or hand-made log can), which
    /// would otherwise refuse every update and never be rewritten.
    pub fn forget_last_values(&mut self) {
        self.last_in.clear();
        self.last_in.push(b'x');
        self.last_out.clear();
        self.last_out.push(b'x');
    }

    /// The newest history entry: `(time, in_rate, out_rate)`, i.e. line 2
    /// of the log. `None` for an empty history.
    pub fn latest_rate(&self) -> Option<(i64, f64, f64)> {
        (self.hist.valid > 0).then(|| {
            let h = self.hist.at(0);
            (h.time, h.inr, h.outr)
        })
    }

    /// Number of history lines (rateup's `histvalid` as a re-read would see).
    pub fn history_len(&self) -> usize {
        self.hist.valid
    }

    /// rateup's `update()`: turn the raw values into rates and rebuild the
    /// consolidated history.
    ///
    /// `input`/`output` are the raw values exactly as MRTG passes them
    /// (decimal strings of any width, or `"-1"` for unknown). `abs_max` is
    /// `AbsMax[]` (or `MaxBytes[]`). `unknown_as_zero` is rateup's `-z`
    /// (`Options[]: unknaszero`); otherwise (`-Z`) unknown or out-of-range
    /// rates repeat the previous rate.
    ///
    /// After `Updated`, the in-memory history equals what re-reading the
    /// rendered log gives, so repeated calls behave like repeated rateup
    /// processes.
    pub fn update(
        &mut self,
        now: i64,
        input: &str,
        output: &str,
        abs_max: i64,
        mode: Mode,
        unknown_as_zero: bool,
    ) -> UpdateOutcome {
        // Every rateup run starts from the re-read history.
        if let Some(v) = self.view.take() {
            self.scratch = v.entries;
        }
        if let Err(e) = check_raw(input).and_then(|()| check_raw(output)) {
            return UpdateOutcome::Rejected(e);
        }
        if now < self.last_time {
            return UpdateOutcome::TimeWarp;
        }
        let (inrate, outrate) = match self.rates(
            now,
            input.as_bytes(),
            output.as_bytes(),
            abs_max,
            mode,
            unknown_as_zero,
        ) {
            Ok(r) => r,
            Err(e) => return UpdateOutcome::Rejected(e),
        };

        let h = &self.hist;
        let mut rows = self.rows.take().unwrap_or_default();
        rows.clear();
        rows.reserve(MAX_HISTORY);
        let mut lhist = std::mem::take(&mut self.scratch);
        lhist.clear();
        lhist.reserve(MAX_HISTORY);

        rows.push(Row {
            time: now,
            vals: [inrate, outrate, inrate, outrate],
        });
        lhist.push(Hist {
            time: now,
            inr: inrate as f64,
            outr: outrate as f64,
            inmax: inrate as f64,
            outmax: outrate as f64,
        });
        let mut emit = |t: i64, a: Agg| {
            rows.push(Row {
                time: t,
                vals: [c_ll(a.inr), c_ll(a.outr), a.inmax, a.outmax],
            });
            lhist.push(Hist {
                time: t,
                inr: a.inr,
                outr: a.outr,
                inmax: a.inmax as f64,
                outmax: a.outmax as f64,
            });
        };

        // "just in case we were dead for a long time, don't try to gather
        // data from non existing log entries"
        let mut t = h.at(0).time;
        let mut n = 0usize;
        for (i, (count, sample)) in [
            (DAY_COUNT, DAY_SAMPLE),
            (WEEK_COUNT, WEEK_SAMPLE),
            (MONTH_COUNT, MONTH_SAMPLE),
            (YEAR_COUNT, YEAR_SAMPLE),
        ]
        .into_iter()
        .enumerate()
        {
            let plannow = t / sample * sample;
            if plannow < t {
                emit(t, next(h, &mut n, t, t.wrapping_sub(plannow)));
                t = plannow;
            }
            // The day section starts at x = 1: line 2 is the new sample.
            let start = usize::from(i == 0);
            for _ in start..count {
                emit(t, next(h, &mut n, t, sample));
                t = t.wrapping_sub(sample);
            }
        }

        // rateup copies the new slots over its old array but keeps the old
        // histvalid; that array is what image() would read next.
        let mut view = std::mem::take(&mut self.hist);
        if view.entries.len() < lhist.len() {
            view.entries.resize(lhist.len(), Hist::default());
        }
        view.entries[..lhist.len()].copy_from_slice(&lhist);

        // What the next process reads back: integers, with inmax >= in.
        lhist.clear();
        lhist.extend(rows.iter().map(|r| {
            let [i, o, im, om] = r.vals.map(|v| v as f64);
            Hist {
                time: r.time,
                inr: i,
                outr: o,
                inmax: if im < i { i } else { im },
                outmax: if om < o { o } else { om },
            }
        }));
        let valid = lhist.len();
        self.hist = HistBuf {
            entries: lhist,
            valid,
        };
        self.rows = Some(rows);
        self.view = Some(view);
        self.last_time = now;
        self.last_in.clear();
        self.last_in.extend_from_slice(input.as_bytes());
        self.last_out.clear();
        self.last_out.extend_from_slice(output.as_bytes());
        UpdateOutcome::Updated
    }

    /// The rate computation part of `update()` (before any file is written).
    fn rates(
        &self,
        now: i64,
        input: &[u8],
        output: &[u8],
        abs_max: i64,
        mode: Mode,
        zero: bool,
    ) -> Result<(i64, i64), InputError> {
        let h0 = self.hist.at(0);
        let period = now.wrapping_sub(self.last_time) as f64;
        let (mut inrate, mut outrate);
        if period <= 0.0 || period > 3600.0 || self.last_in.first() == Some(&b'x') {
            // "if last update is strange": sync with zero or the last value.
            (inrate, outrate) = if zero {
                (0, 0)
            } else {
                (c_ll(h0.inr), c_ll(h0.outr))
            };
        } else {
            inrate = one_rate(input, &self.last_in, h0.inr, mode, period, zero)?;
            outrate = one_rate(output, &self.last_out, h0.outr, mode, period, zero)?;
        }
        if inrate < 0 || inrate > abs_max {
            inrate = if zero { 0 } else { c_ll(h0.inr) };
        }
        if outrate < 0 || outrate > abs_max {
            outrate = if zero { 0 } else { c_ll(h0.outr) };
        }
        Ok((inrate, outrate))
    }

    /// The exact bytes of the log file rateup writes.
    ///
    /// After [`RateLog::update`] this is byte-identical to rateup's output.
    /// For a log that was only loaded it is the normalised re-rendering of
    /// what was read (line 1 plus `histvalid` five-column lines).
    pub fn render(&self) -> String {
        let bytes = self.render_bytes();
        match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        }
    }

    /// Like [`RateLog::render`] but returns raw bytes (identical unless a
    /// hand-edited log had non-UTF-8 bytes in line 1).
    pub fn render_bytes(&self) -> Vec<u8> {
        let lines = self.rows.as_ref().map_or(self.hist.valid, Vec::len);
        let mut out =
            Vec::with_capacity(self.last_in.len() + self.last_out.len() + 24 + lines * 48);
        push_i64(&mut out, self.last_time);
        out.push(b' ');
        out.extend_from_slice(&self.last_in);
        out.push(b' ');
        out.extend_from_slice(&self.last_out);
        out.push(b'\n');
        let mut line = |time: i64, vals: [i64; 4]| {
            push_i64(&mut out, time);
            for v in vals {
                out.push(b' ');
                push_i64(&mut out, v);
            }
            out.push(b'\n');
        };
        match &self.rows {
            Some(rows) => rows.iter().for_each(|r| line(r.time, r.vals)),
            None => self.hist.entries[..self.hist.valid.min(self.hist.entries.len())]
                .iter()
                .for_each(|h| line(h.time, [h.inr, h.outr, h.inmax, h.outmax].map(c_ll))),
        }
        out
    }

    /// Atomically writes the log like rateup, but without following
    /// symlinks: the new content goes to `<base>.tmp_<pid>` (created with
    /// `O_CREAT|O_EXCL|O_NOFOLLOW`, mode 0644; fails if it already exists),
    /// is fsynced, then `<base>.log` is renamed to `<base>.old` and the
    /// temporary file to `<base>.log`.
    pub fn save(&self, dir: &Path, base: &str) -> std::io::Result<()> {
        save::save_atomic(dir, base, &self.render_bytes(), true)
    }

    /// Like [`RateLog::save`] (same atomic, symlink-safe replacement) but
    /// without fsyncing the file and directory. For callers that write many
    /// logs and then flush them all at once (for example with `syncfs`).
    /// A crash can lose the latest update but never leaves a partial log.
    pub fn save_unsynced(&self, dir: &Path, base: &str) -> std::io::Result<()> {
        save::save_atomic(dir, base, &self.render_bytes(), false)
    }

    /// rateup's `image()` statistics for one graph period at time `now`.
    ///
    /// Columns end at `now` rounded down to the period's step and go back
    /// `opts.maxx` steps. The returned numbers are exactly the lines rateup
    /// prints to stdout (and MRTG parses): integer `long long` values,
    /// divided back by 8 when `bits` is set. `points` holds the column
    /// values the graph is drawn from (already multiplied by 8 with `bits`).
    pub fn summarize(&self, now: i64, period: Period, opts: &SummaryOpts) -> Summary {
        let h = self.view.as_ref().unwrap_or(&self.hist);
        let step = period.step();
        let muli: f64 = if opts.bits { 8.0 } else { 1.0 };
        let muli_ll: i64 = if opts.bits { 8 } else { 1 };
        let mut now = now / step * step;
        let (mut avin, mut avout, mut avmxin, mut avmxout) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let (mut maxin, mut maxout, mut maxpercent): (i64, i64, i64) = (0, 0, 0);
        let (mut latestin, mut latestout, mut latestpercent) = (0.0f64, 0.0f64, 0.0f64);
        let mut valsamp: i64 = 0;
        let mut n = 0usize;
        let mut points = Vec::with_capacity(opts.maxx);
        for x in 0..opts.maxx {
            let a = next(h, &mut n, now, step);
            let inr = a.inr * muli;
            let outr = a.outr * muli;
            let inmax = c_ll(a.inmax as f64 * muli);
            let outmax = c_ll(a.outmax as f64 * muli);
            points.push(Point {
                time: now,
                r#in: inr,
                out: outr,
                in_max: inmax as f64,
                out_max: outmax as f64,
            });
            if inmax > 0 || outmax > 0 || inr > 0.0 || outr > 0.0 || opts.withzeroes {
                valsamp += 1;
            }
            if x == 0 {
                latestin = inr;
                latestout = outr;
                if outr != 0.0 {
                    latestpercent = inr * 100.0 / outr;
                }
            }
            avin += inr;
            avout += outr;
            avmxin += inmax as f64;
            avmxout += outmax as f64;
            if opts.peak {
                maxin = maxin.max(inmax);
                maxout = maxout.max(outmax);
            } else {
                maxin = cmax_ll(maxin, inr);
                maxout = cmax_ll(maxout, outr);
            }
            // (maxs, which only scales the graph, is not needed here.)
            if opts.dorelpercent && outr != 0.0 {
                let dummy = 100.0 * inr / outr;
                let mp = maxpercent as f64;
                maxpercent = c_ll(if dummy > mp { dummy } else { mp });
            }
            now = now.wrapping_sub(step);
        }
        let mut avpercent = 0.0;
        if opts.dorelpercent && avout != 0.0 && valsamp != 0 {
            avpercent = 100.0 * avin / avout;
        }
        if valsamp != 0 {
            let v = valsamp as f64;
            avin /= v;
            avout /= v;
            avmxin /= v;
            avmxout /= v;
        }
        let m = muli_ll as f64;
        let pct = |v: f64| opts.dorelpercent.then(|| c_ll(v + 0.5));
        Summary {
            max_in: c_ll(maxin.wrapping_div(muli_ll) as f64 + 0.5),
            max_out: c_ll(maxout.wrapping_div(muli_ll) as f64 + 0.5),
            max_pct: pct(maxpercent as f64),
            avg_in: c_ll(avin / m + 0.5),
            avg_out: c_ll(avout / m + 0.5),
            avg_pct: pct(avpercent),
            cur_in: c_ll(latestin / m + 0.5),
            cur_out: c_ll(latestout / m + 0.5),
            cur_pct: pct(latestpercent),
            avg_max_in: c_ll(avmxin / m + 0.5),
            avg_max_out: c_ll(avmxout / m + 0.5),
            points,
        }
    }
}

/// `(long long) max(a, b)` with `a: long long`, `b: double`.
#[inline]
fn cmax_ll(a: i64, b: f64) -> i64 {
    let af = a as f64;
    c_ll(if af > b { af } else { b })
}

/// Rejects raw values that rateup could not write and re-read safely.
fn check_raw(s: &str) -> Result<(), InputError> {
    if s.is_empty() {
        Err(InputError::Empty)
    } else if s.len() > MAX_RAW_LEN {
        Err(InputError::TooLong)
    } else if s.bytes().any(|b| b == 0 || cnum::is_c_space(b)) {
        Err(InputError::BadChar)
    } else {
        Ok(())
    }
}

/// One direction of rateup's rate calculation for a normal period.
fn one_rate(
    cur: &[u8],
    last: &[u8],
    h0: f64,
    mode: Mode,
    period: f64,
    zero: bool,
) -> Result<i64, InputError> {
    if cur == b"-1" || last == b"-1" {
        return Ok(if zero { 0 } else { c_ll(h0) });
    }
    let r = if mode.is_counter() {
        let mut r = diff(cur, last)?;
        if r < 0.0 {
            // A wrapped 32-bit counter, or a reset (64-bit counters "do not
            // wrap usually").
            if r > -4_294_967_296.0 {
                r += 4_294_967_296.0;
            } else {
                r = 0.0;
            }
        }
        r
    } else {
        diff(cur, b"0")?
    };
    Ok(match mode {
        Mode::Gauge => c_ll(r + 0.5),
        Mode::PerHour => c_ll(r * (3600.0 / period) + 0.5),
        Mode::PerMinute => c_ll(r * (60.0 / period) + 0.5),
        Mode::Counter | Mode::Absolute => c_ll(r / period + 0.5),
    })
}
