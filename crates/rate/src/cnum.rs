// SPDX-License-Identifier: GPL-2.0-or-later
//! Emulation of the C library and x86-64 numeric behaviour that rateup relies on.
//!
//! rateup is compiled C, so its output depends on a few details that Rust does
//! differently by default: float to integer casts, `scanf` integer parsing,
//! `atof`, and `%lld` formatting. Each helper here reproduces the glibc /
//! x86-64 result exactly.

/// `(long long) x` as executed by x86-64 (`cvttsd2si`).
///
/// C leaves out-of-range conversions undefined; on x86-64 the hardware returns
/// the "integer indefinite" value `i64::MIN` for NaN and for anything outside
/// `[-2^63, 2^63)`. Rust's `as` saturates instead, so it must not be used for
/// values that rateup converts.
#[inline]
pub(crate) fn c_ll(x: f64) -> i64 {
    // 2^63 is exactly representable; anything >= it (or NaN) is out of range.
    if x.is_nan() || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x) {
        i64::MIN
    } else {
        x as i64
    }
}

/// C `isspace` in the "C" locale.
#[inline]
pub(crate) fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Result of a `scanf` integer conversion attempt.
pub(crate) enum ScanInt {
    /// Converted value and the position just after the last digit.
    Ok(i64, usize),
    /// Matching failure; position where the stream is left.
    Fail(usize),
    /// End of input reached while skipping leading white space.
    Eof,
}

/// glibc `scanf("%lld")` / `"%ld"` starting at `pos`: skips white space,
/// accepts an optional sign and decimal digits, and saturates to
/// `i64::MIN`/`i64::MAX` on overflow (glibc delegates to `strtoll`).
pub(crate) fn scan_int(buf: &[u8], mut pos: usize) -> ScanInt {
    while pos < buf.len() && is_c_space(buf[pos]) {
        pos += 1;
    }
    if pos >= buf.len() {
        return ScanInt::Eof;
    }
    let mut neg = false;
    if buf[pos] == b'+' || buf[pos] == b'-' {
        neg = buf[pos] == b'-';
        pos += 1;
    }
    let start = pos;
    let mut mag: u64 = 0;
    let mut overflow = false;
    while pos < buf.len() && buf[pos].is_ascii_digit() {
        let d = u64::from(buf[pos] - b'0');
        match mag.checked_mul(10).and_then(|m| m.checked_add(d)) {
            Some(m) => mag = m,
            None => overflow = true,
        }
        pos += 1;
    }
    if pos == start {
        return ScanInt::Fail(pos);
    }
    let v = if neg {
        if overflow || mag > 1u64 << 63 {
            i64::MIN
        } else {
            0i64.wrapping_sub_unsigned(mag)
        }
    } else if overflow || mag > i64::MAX as u64 {
        i64::MAX
    } else {
        mag as i64
    };
    ScanInt::Ok(v, pos)
}

/// glibc `atof`/`strtod` in the "C" locale over a byte string that ends at
/// the first NUL (or the slice end).
///
/// Supports leading white space, a sign, decimal mantissas with optional
/// fraction and exponent, `inf`/`infinity`/`nan[(...)]` and hexadecimal
/// floats. Decimal conversion is correctly rounded (as glibc is).
pub(crate) fn c_atof(s: &[u8]) -> f64 {
    let s = match s.iter().position(|&b| b == 0) {
        Some(n) => &s[..n],
        None => s,
    };
    let mut p = 0;
    while p < s.len() && is_c_space(s[p]) {
        p += 1;
    }
    let mut neg = false;
    if p < s.len() && (s[p] == b'+' || s[p] == b'-') {
        neg = s[p] == b'-';
        p += 1;
    }
    let rest = &s[p..];
    let lower = |i: usize| rest.get(i).map(|b| b.to_ascii_lowercase());
    let v = if lower(0) == Some(b'i') && lower(1) == Some(b'n') && lower(2) == Some(b'f') {
        f64::INFINITY
    } else if lower(0) == Some(b'n') && lower(1) == Some(b'a') && lower(2) == Some(b'n') {
        f64::NAN
    } else if rest.first() == Some(&b'0')
        && lower(1) == Some(b'x')
        && (rest.get(2).is_some_and(u8::is_ascii_hexdigit)
            || (rest.get(2) == Some(&b'.') && rest.get(3).is_some_and(u8::is_ascii_hexdigit)))
    {
        hex_float(&rest[2..])
    } else {
        dec_float(rest)
    };
    if neg { -v } else { v }
}

fn dec_float(s: &[u8]) -> f64 {
    let mut p = 0;
    let mut digits = 0;
    while p < s.len() && s[p].is_ascii_digit() {
        p += 1;
        digits += 1;
    }
    if p < s.len() && s[p] == b'.' {
        p += 1;
        while p < s.len() && s[p].is_ascii_digit() {
            p += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0.0;
    }
    let mant_end = p;
    let mut end = mant_end;
    if p < s.len() && (s[p] == b'e' || s[p] == b'E') {
        let mut q = p + 1;
        if q < s.len() && (s[q] == b'+' || s[q] == b'-') {
            q += 1;
        }
        let exp_digits = q;
        while q < s.len() && s[q].is_ascii_digit() {
            q += 1;
        }
        if q > exp_digits {
            end = q;
        }
    }
    let mut text = String::with_capacity(end + 1);
    text.push('0');
    // Only ASCII digits, '.', 'e'/'E' and an exponent sign reach here; the
    // leading '0' turns ".5" into "0.5".
    text.extend(s[..end].iter().map(|&b| b as char));
    text.parse::<f64>().unwrap_or(0.0)
}

fn hex_float(s: &[u8]) -> f64 {
    let mut mant: u128 = 0;
    let mut exp: i64 = 0;
    let mut p = 0;
    let mut seen_point = false;
    let mut sticky = false;
    while p < s.len() {
        let b = s[p];
        if b == b'.' && !seen_point {
            seen_point = true;
        } else if let Some(d) = (b as char).to_digit(16) {
            if mant >> 120 == 0 {
                mant = (mant << 4) | u128::from(d);
                if seen_point {
                    exp -= 4;
                }
            } else {
                sticky |= d != 0;
                if !seen_point {
                    exp += 4;
                }
            }
        } else {
            break;
        }
        p += 1;
    }
    if p < s.len() && (s[p] == b'p' || s[p] == b'P')
        && let ScanInt::Ok(e, _) = scan_int_nows(&s[p + 1..]) {
            exp = exp.saturating_add(e);
        }
    if sticky {
        mant |= 1;
    }
    (mant as f64) * 2f64.powi(exp.clamp(-2000, 2000) as i32)
}

/// Like [`scan_int`] but without skipping white space (for `p` exponents).
fn scan_int_nows(s: &[u8]) -> ScanInt {
    match s.first() {
        Some(b) if b.is_ascii_digit() || *b == b'+' || *b == b'-' => scan_int(s, 0),
        _ => ScanInt::Eof,
    }
}

/// Appends the `%lld` decimal form of `v` to `out` without going through
/// `core::fmt` (render writes ~12,000 numbers per log).
#[inline]
pub(crate) fn push_i64(out: &mut Vec<u8>, v: i64) {
    const PAIRS: &[u8; 200] = b"0001020304050607080910111213141516171819\
        2021222324252627282930313233343536373839\
        4041424344454647484950515253545556575859\
        6061626364656667686970717273747576777879\
        8081828384858687888990919293949596979899";
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut m = v.unsigned_abs();
    while m >= 100 {
        let d = (m % 100) as usize * 2;
        m /= 100;
        i -= 2;
        buf[i..i + 2].copy_from_slice(&PAIRS[d..d + 2]);
    }
    if m >= 10 {
        let d = m as usize * 2;
        i -= 2;
        buf[i..i + 2].copy_from_slice(&PAIRS[d..d + 2]);
    } else {
        i -= 1;
        buf[i] = b'0' + m as u8;
    }
    if v < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&buf[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cvttsd2si() {
        assert_eq!(c_ll(f64::NAN), i64::MIN);
        assert_eq!(c_ll(1e30), i64::MIN);
        assert_eq!(c_ll(-1e30), i64::MIN);
        assert_eq!(c_ll(9_223_372_036_854_775_808.0), i64::MIN);
        assert_eq!(c_ll(-9_223_372_036_854_775_808.0), i64::MIN);
        assert_eq!(c_ll(2.9), 2);
        assert_eq!(c_ll(-2.9), -2);
    }

    #[test]
    fn scanf_saturates() {
        assert!(matches!(scan_int(b"99999999999999999999", 0), ScanInt::Ok(i64::MAX, 20)));
        assert!(matches!(scan_int(b" -99999999999999999999", 0), ScanInt::Ok(i64::MIN, 22)));
        assert!(matches!(scan_int(b"-9223372036854775808", 0), ScanInt::Ok(i64::MIN, _)));
        assert!(matches!(scan_int(b"+5", 0), ScanInt::Ok(5, 2)));
        assert!(matches!(scan_int(b"- 5", 0), ScanInt::Fail(1)));
        assert!(matches!(scan_int(b"   ", 0), ScanInt::Eof));
    }

    #[test]
    fn atof_forms() {
        assert_eq!(c_atof(b"   123"), 123.0);
        assert_eq!(c_atof(b"  12.5e1x"), 125.0);
        assert_eq!(c_atof(b"1e"), 1.0);
        assert_eq!(c_atof(b"1.e2"), 100.0);
        assert_eq!(c_atof(b"-.5"), -0.5);
        assert_eq!(c_atof(b"abc"), 0.0);
        assert_eq!(c_atof(b"0x10"), 16.0);
        assert_eq!(c_atof(b"0x1p4"), 16.0);
        assert!(c_atof(b"INFinity").is_infinite());
        assert!(c_atof(b"nan").is_nan());
        assert_eq!(c_atof(b"12\x0034"), 12.0);
        assert_eq!(c_atof(b"1234567890123456789012345"), 1.2345678901234568e24);
    }

    #[test]
    fn itoa() {
        for v in [0, 1, -1, 9, 10, 42, 99, 100, 101, 999, -1000, i64::MAX, i64::MIN, 1_000_000, 12_345] {
            let mut o = Vec::new();
            push_i64(&mut o, v);
            assert_eq!(o, v.to_string().into_bytes());
        }
    }
}
