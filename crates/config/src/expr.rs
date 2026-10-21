// SPDX-License-Identifier: GPL-2.0-or-later
//! Safe evaluator for MRTG Target expressions.
//!
//! MRTG turns a `Target[]` value into Perl source and `eval`s it, with the
//! data sources as `Math::BigFloat` objects. LinkScope compiles the same
//! rewritten string (see [`crate::target`]) into this AST, accepting only
//! the arithmetic that Target expressions use in practice: numbers, source
//! references, parentheses, unary `+`/`-`, `**`, `*`, `/`, `%`, `+`, `-`, and
//! the functions `int()` and `abs()`. Precedence and associativity follow
//! Perl. Arithmetic is arbitrary precision (like `Math::BigFloat`), so 64-bit
//! counters and sums of them are exact. Anything else is rejected when the
//! config is loaded, instead of failing (or running) at poll time.

use std::str::FromStr;

use bigdecimal::num_bigint::BigInt;
use bigdecimal::{BigDecimal, RoundingMode, Zero};
use serde::Serialize;

/// Maximum nesting depth accepted when parsing (guards the stack).
pub const MAX_DEPTH: usize = 200;
/// Most tokens in a Target expression (a real one has a few dozen).
pub const MAX_TOKENS: usize = 1000;

/// Traffic direction, MRTG's `$mode` (`_IN_` / `_OUT_`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Dir {
    In,
    Out,
}

/// A compiled Target expression.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Expr {
    Num(#[serde(serialize_with = "ser_dec")] BigDecimal),
    /// Reference to entry `N` of the config's source table.
    Source(usize),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Int(Box<Expr>),
    Abs(Box<Expr>),
}

fn ser_dec<S: serde::Serializer>(d: &BigDecimal, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&d.to_plain_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(BigDecimal),
    Src(usize),
    Op(&'static str),
    LParen,
    RParen,
    Func(&'static str),
}

/// Why a value could not be computed (MRTG logs these and records "unknown").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A referenced source has no value this round.
    Undefined(usize),
    DivisionByZero,
    /// Result is not a finite number or is over 190 characters long.
    NotANumber,
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Undefined(i) => write!(f, "source {i} has no value"),
            EvalError::DivisionByZero => write!(f, "division by zero"),
            EvalError::NotANumber => write!(f, "did not evaluate to a number"),
        }
    }
}

impl Expr {
    /// Compile MRTG's rewritten Target string.
    pub fn compile(rewritten: &str) -> Result<Expr, String> {
        let toks = lex(rewritten)?;
        if toks.is_empty() {
            return Err("empty Target expression".into());
        }
        // Nesting is limited by MAX_DEPTH, but a chain (`1+1+1+...`) builds a tree as deep as it
        // is long, and evaluating or dropping it recurses: the token count bounds every shape.
        if toks.len() > MAX_TOKENS {
            return Err(format!(
                "Target expression is longer than {MAX_TOKENS} parts"
            ));
        }
        let mut p = Parser {
            toks,
            pos: 0,
            depth: 0,
        };
        let e = p.expr(0)?;
        if p.pos != p.toks.len() {
            return Err(format!(
                "unexpected {:?} in Target expression",
                p.toks[p.pos]
            ));
        }
        Ok(e)
    }

    /// Source indices referenced by this expression.
    pub fn sources(&self, out: &mut Vec<usize>) {
        match self {
            Expr::Num(_) => {}
            Expr::Source(i) => out.push(*i),
            Expr::Neg(e) | Expr::Int(e) | Expr::Abs(e) => e.sources(out),
            Expr::Bin(_, a, b) => {
                a.sources(out);
                b.sources(out);
            }
        }
    }

    /// Evaluate with `get(source_index)` supplying the value for the
    /// current direction.
    pub fn eval(&self, get: &dyn Fn(usize) -> Option<BigDecimal>) -> Result<BigDecimal, EvalError> {
        Ok(match self {
            Expr::Num(n) => n.clone(),
            Expr::Source(i) => get(*i).ok_or(EvalError::Undefined(*i))?,
            Expr::Neg(e) => -e.eval(get)?,
            Expr::Int(e) => e.eval(get)?.with_scale_round(0, RoundingMode::Down),
            Expr::Abs(e) => e.eval(get)?.abs(),
            Expr::Bin(op, a, b) => {
                let x = a.eval(get)?;
                let y = b.eval(get)?;
                match op {
                    Op::Add => x + y,
                    Op::Sub => x - y,
                    Op::Mul => x * y,
                    Op::Div => {
                        if y.is_zero() {
                            return Err(EvalError::DivisionByZero);
                        }
                        x / y
                    }
                    Op::Mod => {
                        if y.is_zero() {
                            return Err(EvalError::DivisionByZero);
                        }
                        // Math::BigFloat bmod: result has the sign of the divisor.
                        let q = (&x / &y).with_scale_round(0, RoundingMode::Floor);
                        x - y * q
                    }
                    Op::Pow => pow(&x, &y)?,
                }
            }
        })
    }

    /// Evaluate and post-process exactly like MRTG's `getcurrent`: reject
    /// non-numbers and values over 190 characters, round half to even to an
    /// integer, drop a leading `+`. Returns the decimal string handed to the
    /// rate engine.
    pub fn eval_value(
        &self,
        get: &dyn Fn(usize) -> Option<BigDecimal>,
    ) -> Result<String, EvalError> {
        let v = self.eval(get)?;
        // Judged before it is printed: `1e999999999` prints as a billion digits.
        if !within_width(&v) {
            return Err(EvalError::NotANumber);
        }
        let plain = v.to_plain_string();
        if plain.len() > 190 {
            return Err(EvalError::NotANumber);
        }
        let r = v.with_scale_round(0, RoundingMode::HalfEven);
        let (digits, _) = r.as_bigint_and_exponent();
        Ok(digits.to_string())
    }
}

/// The widest value MRTG keeps: `getcurrent` rejects one that prints longer than 190 characters.
pub const MAX_WIDTH: i64 = 190;

/// Whether `v` prints in at most [`MAX_WIDTH`] digits on either side of the point, judged from
/// its digits and exponent without printing it. A value from a device or a command can be
/// written with an exponent (`1e999999999` passes MRTG's number check), and printing or aligning
/// it would take gigabytes; anything this wide MRTG rejects anyway.
pub fn within_width(v: &BigDecimal) -> bool {
    let (_, scale) = v.as_bigint_and_exponent();
    let digits = i64::try_from(v.digits()).unwrap_or(i64::MAX);
    scale.unsigned_abs() <= MAX_WIDTH as u64 && digits.saturating_sub(scale) <= MAX_WIDTH
}

/// A value read from a device or a command, if it is a decimal within [`MAX_WIDTH`].
pub fn bounded_decimal(s: &str) -> Option<BigDecimal> {
    BigDecimal::from_str(s).ok().filter(within_width)
}

fn pow(x: &BigDecimal, y: &BigDecimal) -> Result<BigDecimal, EvalError> {
    if y.is_integer() {
        let (e, _) = y.with_scale(0).as_bigint_and_exponent();
        if e >= BigInt::from(0) && e <= BigInt::from(64) {
            let n: u32 = e.try_into().map_err(|_| EvalError::NotANumber)?;
            let mut acc = BigDecimal::from(1);
            for _ in 0..n {
                acc *= x;
            }
            return Ok(acc);
        }
    }
    let xf: f64 = x.to_string().parse().map_err(|_| EvalError::NotANumber)?;
    let yf: f64 = y.to_string().parse().map_err(|_| EvalError::NotANumber)?;
    let r = xf.powf(yf);
    if !r.is_finite() {
        return Err(if xf == 0.0 {
            EvalError::DivisionByZero
        } else {
            EvalError::NotANumber
        });
    }
    BigDecimal::from_str(&format!("{r}")).map_err(|_| EvalError::NotANumber)
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    const PH: &str = "$target->[";
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            b'*' if b.get(i + 1) == Some(&b'*') => {
                out.push(Tok::Op("**"));
                i += 2;
            }
            b'+' | b'-' | b'*' | b'/' | b'%' => {
                out.push(Tok::Op(match c {
                    b'+' => "+",
                    b'-' => "-",
                    b'*' => "*",
                    b'/' => "/",
                    _ => "%",
                }));
                i += 1;
            }
            b'$' if s[i..].starts_with(PH) => {
                let rest = &s[i + PH.len()..];
                let end = rest.find("]{$mode}").ok_or("malformed source reference")?;
                let idx: usize = rest[..end]
                    .parse()
                    .map_err(|_| "malformed source reference")?;
                out.push(Tok::Src(idx));
                i += PH.len() + end + "]{$mode}".len();
            }
            b'0'..=b'9' | b'.' => {
                let (n, len) = lex_number(&s[i..])?;
                out.push(Tok::Num(n));
                i += len;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &s[start..i];
                let f = match word {
                    "int" => "int",
                    "abs" => "abs",
                    _ => return Err(unsupported(s, start)),
                };
                out.push(Tok::Func(f));
            }
            _ => return Err(unsupported(s, i)),
        }
    }
    Ok(out)
}

fn unsupported(s: &str, at: usize) -> String {
    let snippet: String = s[at..].chars().take(24).collect();
    format!(
        "unsupported syntax near '{}'. LinkScope evaluates Target expressions without Perl: \
         use data sources, numbers, + - * / % **, parentheses, int() and abs()",
        snippet.trim()
    )
}

/// Perl numeric literal: decimal with optional fraction/exponent and `_`
/// separators, `0x` hex, `0b` binary, and leading-zero octal.
fn lex_number(s: &str) -> Result<(BigDecimal, usize), String> {
    let b = s.as_bytes();
    let radix_lit = |prefix_len: usize,
                     radix: u32,
                     valid: fn(u8) -> bool|
     -> Result<(BigDecimal, usize), String> {
        let mut j = prefix_len;
        while j < b.len() && (valid(b[j]) || b[j] == b'_') {
            j += 1;
        }
        let digits: String = s[prefix_len..j].chars().filter(|&c| c != '_').collect();
        let n = BigInt::parse_bytes(digits.as_bytes(), radix).ok_or("bad number")?;
        Ok((BigDecimal::from(n), j))
    };
    if s.starts_with("0x") || s.starts_with("0X") {
        return radix_lit(2, 16, |c| c.is_ascii_hexdigit());
    }
    if s.starts_with("0b") || s.starts_with("0B") {
        return radix_lit(2, 2, |c| c == b'0' || c == b'1');
    }
    let mut j = 0;
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'_') {
        j += 1;
    }
    let int_part_end = j;
    let mut is_float = false;
    if j < b.len() && b[j] == b'.' && b.get(j + 1) != Some(&b'.') {
        is_float = true;
        j += 1;
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'_') {
            j += 1;
        }
    }
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        let mut k = j + 1;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        let es = k;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        if k > es {
            is_float = true;
            j = k;
        }
    }
    let text: String = s[..j].chars().filter(|&c| c != '_').collect();
    if text == "." || text.is_empty() {
        return Err("bad number".into());
    }
    if !is_float && int_part_end > 1 && b[0] == b'0' {
        return radix_lit(1, 8, |c| (b'0'..=b'7').contains(&c));
    }
    let n = BigDecimal::from_str(&text).map_err(|_| format!("bad number '{text}'"))?;
    if !within_width(&n) {
        return Err(format!("number '{text}' is wider than {MAX_WIDTH} digits"));
    }
    Ok((n, j))
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bin_prec(op: &str) -> Option<(u8, bool)> {
        // (precedence, right-associative)
        match op {
            "+" | "-" => Some((1, false)),
            "*" | "/" | "%" => Some((2, false)),
            "**" => Some((4, true)),
            _ => None,
        }
    }

    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("Target expression nested too deeply".into());
        }
        let mut lhs = self.unary()?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            let Some((prec, right)) = Self::bin_prec(op) else {
                break;
            };
            if prec < min {
                break;
            }
            self.pos += 1;
            let rhs = self.expr(if right { prec } else { prec + 1 })?;
            let o = match op {
                "+" => Op::Add,
                "-" => Op::Sub,
                "*" => Op::Mul,
                "/" => Op::Div,
                "%" => Op::Mod,
                _ => Op::Pow,
            };
            lhs = Expr::Bin(o, Box::new(lhs), Box::new(rhs));
        }
        self.depth -= 1;
        Ok(lhs)
    }

    /// Unary minus binds looser than `**` in Perl: `-2**2 == -4`.
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Op("-")) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary_operand()?)))
            }
            Some(Tok::Op("+")) => {
                self.pos += 1;
                self.unary_operand()
            }
            _ => self.power(),
        }
    }

    fn unary_operand(&mut self) -> Result<Expr, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("Target expression nested too deeply".into());
        }
        let r = self.unary();
        self.depth -= 1;
        r
    }

    fn power(&mut self) -> Result<Expr, String> {
        let base = self.primary()?;
        if let Some(Tok::Op("**")) = self.peek() {
            self.pos += 1;
            // Right-associative, and the exponent may carry a unary sign.
            let exp = self.unary_operand()?;
            return Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Src(i)) => {
                self.pos += 1;
                Ok(Expr::Source(i))
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let e = self.expr(0)?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("missing ')' in Target expression".into());
                }
                self.pos += 1;
                Ok(e)
            }
            Some(Tok::Func(f)) => {
                self.pos += 1;
                if self.peek() != Some(&Tok::LParen) {
                    return Err(format!("{f} needs parentheses, e.g. {f}(...)"));
                }
                let inner = self.primary()?;
                Ok(if f == "int" {
                    Expr::Int(Box::new(inner))
                } else {
                    Expr::Abs(Box::new(inner))
                })
            }
            Some(t) => Err(format!("unexpected {t:?} in Target expression")),
            None => Err("Target expression ends unexpectedly".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_judged_by_its_width_without_printing_it() {
        // MRTG's number check lets an exponent through (its `.` is unescaped); the width decides.
        for ok in [
            "0",
            "123",
            "1e5",
            "-1e5",
            "18446744073709551615",
            "0.5",
            &"9".repeat(190),
        ] {
            assert!(bounded_decimal(ok).is_some(), "{ok}");
        }
        for wide in [
            "1e999999999",
            "-1e999999999",
            "1e-999999999",
            "1e191",
            &"9".repeat(191),
            "1e99999999999999",
        ] {
            assert!(bounded_decimal(wide).is_none(), "{wide}");
        }
        // A chain as long as it likes is refused before it is built (it overflowed the stack).
        let chain = vec!["1"; 2000].join("+");
        assert!(Expr::compile(&chain).is_err());
        assert!(Expr::compile(&vec!["1"; 300].join("+")).is_ok());
        // A literal that wide is refused when the config is read; a wide result is no number.
        assert!(Expr::compile("1e999999999").is_err());
        assert_eq!(
            ev(" $target->[0]{$mode} * $target->[0]{$mode} ", &["1e150"]),
            Err(EvalError::NotANumber)
        );
        assert_eq!(ev(" $target->[0]{$mode} * 8", &["1e5"]).unwrap(), "800000");
    }

    fn ev(s: &str, vals: &[&str]) -> Result<String, EvalError> {
        let e = Expr::compile(s).unwrap();
        let v: Vec<BigDecimal> = vals
            .iter()
            .map(|x| BigDecimal::from_str(x).unwrap())
            .collect();
        e.eval_value(&|i| v.get(i).cloned())
    }

    #[test]
    fn perl_precedence() {
        assert_eq!(ev("-2**2", &[]).unwrap(), "-4");
        assert_eq!(ev("2**3**2", &[]).unwrap(), "512");
        assert_eq!(ev("1+2*3", &[]).unwrap(), "7");
        assert_eq!(ev("(1+2)*3", &[]).unwrap(), "9");
        assert_eq!(ev("10-3-2", &[]).unwrap(), "5");
        assert_eq!(ev("-7 % 3", &[]).unwrap(), "2");
        assert_eq!(ev("7 % -3", &[]).unwrap(), "-2");
    }

    #[test]
    fn sources_and_big_counters() {
        let s = " $target->[0]{$mode}  +  $target->[1]{$mode} ";
        assert_eq!(
            ev(s, &["18446744073709551615", "18446744073709551615"]).unwrap(),
            "36893488147419103230"
        );
        assert_eq!(
            ev(" $target->[0]{$mode} * 8", &["123456789012345678901"]).unwrap(),
            "987654312098765431208"
        );
    }

    #[test]
    fn rounding_half_even_like_bigfloat() {
        assert_eq!(ev("5/2", &[]).unwrap(), "2");
        assert_eq!(ev("7/2", &[]).unwrap(), "4");
        assert_eq!(ev("10/3", &[]).unwrap(), "3");
        assert_eq!(ev("0x1F + 010 + 1_000", &[]).unwrap(), "1039");
    }

    #[test]
    fn undefined_and_div_zero() {
        assert_eq!(
            ev(" $target->[3]{$mode} ", &["1"]),
            Err(EvalError::Undefined(3))
        );
        assert_eq!(ev("1/0", &[]), Err(EvalError::DivisionByZero));
    }

    #[test]
    fn rejects_perl_code() {
        for bad in [
            "system('id')",
            "`id`",
            "$x",
            "1; unlink 'x'",
            "1 x 3",
            "1 . 2",
            "$target->[0]{$mode} or die",
            "qx(id)",
        ] {
            assert!(Expr::compile(bad).is_err(), "{bad} should be rejected");
        }
        let deep = "(".repeat(5000) + "1" + &")".repeat(5000);
        assert!(Expr::compile(&deep).is_err());
    }
}
