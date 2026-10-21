// SPDX-License-Identifier: GPL-2.0-or-later
//! Ports of `readhist()` and `readfile()`: parsing a `.log` file the way
//! rateup's `fscanf`/`fgets`/`sscanf` sequence does.

use crate::cnum::{ScanInt, is_c_space, scan_int};
use crate::history::{Hist, HistBuf};
use crate::{DAY_COUNT, DAY_SAMPLE, MONTH_COUNT, MONTH_SAMPLE, WEEK_COUNT, WEEK_SAMPLE, YEAR_COUNT, YEAR_SAMPLE};

/// Width of the `%199s` conversions for the last in/out strings.
const LAST_WIDTH: usize = 199;
/// `fgets(buf, 256, fi)` reads at most 255 bytes per call.
const FGETS_MAX: usize = 255;

/// `readhist()` return codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReadCode {
    Ok,
    /// Missing file or unreadable first line (rateup code 1).
    Unreadable,
    /// History lines out of order (rateup code 2).
    Unsorted,
}

/// All the state `readfile()` touches, including C variables whose values
/// survive from one `readhist()` call to the next.
pub(crate) struct Loader {
    pub last_time: i64,
    pub last_in: Vec<u8>,
    pub last_out: Vec<u8>,
    pub hist: HistBuf,
    /// readhist's `long lasttime`, left unchanged when `%ld` fails. In C it is
    /// an uninitialised local; we model it as 0 carried across calls.
    lasttime: i64,
    /// readhist's `long long rd[5]`, which keeps the previous line's values
    /// when a line has fewer than five numbers. Uninitialised in C; 0 here.
    rd: [i64; 5],
}

impl Loader {
    pub fn new() -> Self {
        Loader {
            last_time: 0,
            last_in: Vec::new(),
            last_out: Vec::new(),
            hist: HistBuf { entries: Vec::with_capacity(crate::MAX_HISTORY + 1), valid: 0 },
            lasttime: 0,
            rd: [0; 5],
        }
    }

    /// `readhist(file)`; `data` is `None` when `fopen` failed.
    pub fn readhist(&mut self, data: Option<&[u8]>) -> ReadCode {
        let Some(data) = data else {
            return ReadCode::Unreadable;
        };
        let mut code = ReadCode::Ok;
        let mut pos = 0;
        if self.scan_first_line(data, &mut pos) != 3 {
            code = ReadCode::Unreadable;
        }
        self.last_time = self.lasttime;
        let mut cur = self.last_time;
        let mut x = 0usize;
        self.hist.valid = 0;
        while pos < data.len() && x < crate::MAX_READ_HISTORY {
            // fgets: up to and including '\n', at most 255 bytes.
            let rest = &data[pos..];
            let lim = rest.len().min(FGETS_MAX);
            let len = rest[..lim].iter().position(|&b| b == b'\n').map_or(lim, |i| i + 1);
            let line = &rest[..len];
            pos += len;
            // sscanf stops at an embedded NUL.
            let line = match line.iter().position(|&b| b == 0) {
                Some(n) => &line[..n],
                None => line,
            };
            if scan5(line, &mut self.rd) < 5 {
                self.rd[3] = self.rd[1];
                self.rd[4] = self.rd[2];
            }
            let rd = self.rd;
            let mut e = Hist {
                time: rd[0],
                inr: rd[1] as f64,
                outr: rd[2] as f64,
                inmax: rd[3] as f64,
                outmax: rd[4] as f64,
            };
            if e.inmax < e.inr {
                e.inmax = e.inr;
            }
            if e.outmax < e.outr {
                e.outmax = e.outr;
            }
            self.hist.set(x, e);
            if e.time > cur {
                code = ReadCode::Unsorted;
                break;
            }
            cur = e.time;
            x += 1;
        }
        self.hist.valid = x;
        code
    }

    /// `fscanf(fi, "%ld %199s %199s\n", ...)`; returns the conversion count
    /// (-1 for EOF before the first conversion).
    fn scan_first_line(&mut self, data: &[u8], pos: &mut usize) -> i32 {
        match scan_int(data, *pos) {
            ScanInt::Eof => return -1,
            ScanInt::Fail(p) => {
                *pos = p;
                return 0;
            }
            ScanInt::Ok(v, p) => {
                self.lasttime = v;
                *pos = p;
            }
        }
        let Some(tok) = scan_str(data, pos) else { return 1 };
        self.last_in = tok;
        let Some(tok) = scan_str(data, pos) else { return 2 };
        self.last_out = tok;
        skip_ws(data, pos);
        3
    }

    /// The "file does not exist, it must be created" branch of `readfile()`.
    pub fn create_fresh(&mut self, now: i64) {
        let mut t = now.wrapping_sub(DAY_SAMPLE);
        self.hist.valid = DAY_COUNT + WEEK_COUNT + MONTH_COUNT + YEAR_COUNT - 1;
        self.last_time = t;
        // last.in[0] = 'x': marks "no previous counter" for update().
        match self.last_in.first_mut() {
            Some(b) => *b = b'x',
            None => self.last_in.push(b'x'),
        }
        let mut idx = 0;
        for (count, sample) in [
            (DAY_COUNT, DAY_SAMPLE),
            (WEEK_COUNT, WEEK_SAMPLE),
            (MONTH_COUNT, MONTH_SAMPLE),
            (YEAR_COUNT, YEAR_SAMPLE),
        ] {
            t = t / sample * sample;
            for _ in 0..count {
                self.hist.set(idx, Hist { time: t, ..Hist::default() });
                idx += 1;
                t = t.wrapping_sub(sample);
            }
        }
    }
}

fn skip_ws(data: &[u8], pos: &mut usize) {
    while *pos < data.len() && is_c_space(data[*pos]) {
        *pos += 1;
    }
}

/// `%199s`: skip white space, then read up to 199 non-space bytes. The
/// stored C string ends at the first NUL byte.
fn scan_str(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    skip_ws(data, pos);
    if *pos >= data.len() {
        return None;
    }
    let start = *pos;
    while *pos < data.len() && *pos - start < LAST_WIDTH && !is_c_space(data[*pos]) {
        *pos += 1;
    }
    let tok = &data[start..*pos];
    let tok = match tok.iter().position(|&b| b == 0) {
        Some(n) => &tok[..n],
        None => tok,
    };
    Some(tok.to_vec())
}

/// `sscanf(buf, "%lld %lld %lld %lld %lld", ...)`: returns the number of
/// conversions (or -1 for an empty / blank line) and stores into `rd`.
fn scan5(line: &[u8], rd: &mut [i64; 5]) -> i32 {
    let mut pos = 0;
    for (i, slot) in rd.iter_mut().enumerate() {
        match scan_int(line, pos) {
            ScanInt::Ok(v, p) => {
                *slot = v;
                pos = p;
            }
            ScanInt::Eof if i == 0 => return -1,
            _ => return i as i32,
        }
    }
    5
}
