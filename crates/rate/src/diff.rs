// SPDX-License-Identifier: GPL-2.0-or-later
//! Port of rateup's `diff()`: decimal string subtraction.
//!
//! rateup never parses counters as integers. It subtracts the previous
//! counter string from the current one digit by digit, so counters of any
//! width (64-bit, or the 25-digit values some agents report) work. The
//! result is converted with `atof`, so it is a `double`.
//!
//! The routine operates on raw bytes, not on validated numbers: it skips
//! leading non-digits, then subtracts every remaining byte from the right
//! using C `char` arithmetic, so inputs like `12.5` produce the same odd
//! (but deterministic) results that rateup produces.

use crate::InputError;
use crate::cnum::c_atof;

/// Size of rateup's `res[MAXL]` buffer.
const MAXL: usize = 200;

/// `diff(a, b)` = `a - b` as rateup computes it.
///
/// Fails where the C code has undefined behaviour: when either operand has
/// no digit at all (the C loop `while (!isdigit(*a)) a++` runs off the end
/// of the string) or when the longer operand needs more than the 200-byte
/// result buffer (more than 197 bytes after the skipped prefix).
pub(crate) fn diff(a: &[u8], b: &[u8]) -> Result<f64, InputError> {
    let (mut a, mut b) = (a, b);
    // Two negative numbers: -x - -y == y - x, so swap and drop the signs.
    if a.first() == Some(&b'-') && b.first() == Some(&b'-') {
        let (na, nb) = (&b[1..], &a[1..]);
        a = na;
        b = nb;
    }
    a = &a[a
        .iter()
        .position(u8::is_ascii_digit)
        .ok_or(InputError::NoDigits)?..];
    b = &b[b
        .iter()
        .position(u8::is_ascii_digit)
        .ok_or(InputError::NoDigits)?..];
    // The strings are NUL terminated in C; stop at an embedded NUL.
    a = until_nul(a);
    b = until_nul(b);
    let m = a.len().max(b.len());
    if m + 2 >= MAXL {
        return Err(InputError::TooLongForDiff);
    }
    // res[0..=m+1] is filled with spaces, res[m+2] is the terminator.
    let mut res = [b' '; MAXL];
    res[m + 2] = 0;
    let mut c: i32 = 0;
    let mut r = m + 1;
    for x in 0..m {
        let sa = digit_at(a, x);
        let sb = digit_at(b, x);
        // C: *r1 = save_a - save_b - c + '0'; stored into a signed char.
        let mut v = (sa - sb - c + i32::from(b'0')) as i8;
        if i32::from(v) < i32::from(b'0') {
            v = (i32::from(v) + 10) as i8;
            c = 1;
        } else if i32::from(v) > i32::from(b'9') {
            v = (i32::from(v) - 10) as i8;
            c = 1;
        } else {
            c = 0;
        }
        res[r] = v as u8;
        r -= 1;
    }
    if c != 0 {
        // Negative result: ten's complement back, scanning from the right
        // while the bytes are digits.
        let mut r = m + 1;
        let mut x = 0;
        while res[r].is_ascii_digit() && x < m {
            let mut v = ((i32::from(b'9') - i32::from(res[r]) + c) + i32::from(b'0')) as i8;
            if i32::from(v) > i32::from(b'9') {
                v = (i32::from(v) - 10) as i8;
                c = 1;
            } else {
                c = 0;
            }
            res[r] = v as u8;
            x += 1;
            r -= 1;
        }
        Ok(-c_atof(&res))
    } else {
        Ok(c_atof(&res))
    }
}

/// Byte `x` positions from the right as a C `char` (signed), or `'0'` once
/// the operand is exhausted.
#[inline]
fn digit_at(s: &[u8], x: usize) -> i32 {
    if x < s.len() {
        i32::from(s[s.len() - 1 - x] as i8)
    } else {
        i32::from(b'0')
    }
}

#[inline]
fn until_nul(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::diff;
    use crate::InputError;

    #[test]
    fn basic() {
        assert_eq!(diff(b"1000", b"400"), Ok(600.0));
        assert_eq!(diff(b"400", b"1000"), Ok(-600.0));
        assert_eq!(diff(b"5", b"0"), Ok(5.0));
        assert_eq!(diff(b"-5", b"-8"), Ok(3.0));
        assert_eq!(
            diff(b"1000000000000000000000000", b"999999999999999999999000"),
            Ok(1000.0)
        );
        assert_eq!(diff(b"0", b"4294967295"), Ok(-4294967295.0));
    }

    #[test]
    fn undefined_cases_are_none() {
        assert_eq!(diff(b"abc", b"1"), Err(InputError::NoDigits));
        assert_eq!(diff(b"1", b""), Err(InputError::NoDigits));
        assert_eq!(diff(b"-", b"-5"), Err(InputError::NoDigits));
        let long = vec![b'1'; 198];
        assert_eq!(diff(&long, b"1"), Err(InputError::TooLongForDiff));
        let ok = vec![b'1'; 197];
        assert!(diff(&ok, b"1").is_ok());
    }
}
