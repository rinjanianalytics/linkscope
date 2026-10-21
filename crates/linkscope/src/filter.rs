// SPDX-License-Identifier: GPL-2.0-or-later
//! Safe evaluator for cfgmaker's `--if-filter`.
//!
//! MRTG's cfgmaker `eval`s the filter as Perl. LinkScope accepts the Perl
//! subset filters are written in, and nothing else:
//! * variables `$name` (cfgmaker's interface variables, e.g. `$default`,
//!   `$if_type`, `$if_is_ethernet`, `$if_snmp_descr`, `$if_speed`);
//! * numbers and quoted strings;
//! * `!`, `not`, `&&`, `and`, `||`, `or`, parentheses;
//! * numeric `== != < <= > >=`, string `eq ne lt gt le ge`;
//! * regex match `=~ /re/i` and `!~ /re/`.
//!
//! Truthiness follows Perl: `""`, `"0"` and `0` are false.

use std::collections::HashMap;

use regex::{Regex, RegexBuilder};

/// A filter variable's value.
#[derive(Debug, Clone, PartialEq)]
pub enum FVal {
    Num(f64),
    Str(String),
}

impl FVal {
    fn truthy(&self) -> bool {
        match self {
            FVal::Num(n) => *n != 0.0,
            FVal::Str(s) => !(s.is_empty() || s == "0"),
        }
    }
    fn num(&self) -> f64 {
        match self {
            FVal::Num(n) => *n,
            FVal::Str(s) => linkscope_config::rules::perl_num(s),
        }
    }
    fn text(&self) -> String {
        match self {
            FVal::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
            FVal::Num(n) => format!("{n}"),
            FVal::Str(s) => s.clone(),
        }
    }
    fn bool(b: bool) -> FVal {
        if b { FVal::Num(1.0) } else { FVal::Str(String::new()) }
    }
}

#[derive(Debug, Clone)]
enum Node {
    Var(String),
    Lit(FVal),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Cmp(String, Box<Node>, Box<Node>),
    Match(bool, Box<Node>, Regex),
}

/// A compiled filter.
#[derive(Debug, Clone)]
pub struct Filter {
    root: Node,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Var(String),
    Num(f64),
    Str(String),
    Re(String, String),
    Op(String),
    LP,
    RP,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '$' {
            let st = i + 1;
            i += 1;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            if i == st {
                return Err("'$' must be followed by a variable name".into());
            }
            out.push(Tok::Var(c[st..i].iter().collect()));
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == '_') {
                i += 1;
            }
            let t: String = c[st..i].iter().filter(|&&x| x != '_').collect();
            out.push(Tok::Num(t.parse().map_err(|_| format!("bad number {t}"))?));
        } else if ch == '\'' || ch == '"' {
            let q = ch;
            i += 1;
            let mut v = String::new();
            loop {
                match c.get(i) {
                    None => return Err("unterminated string".into()),
                    Some('\\') => {
                        if let Some(n) = c.get(i + 1) {
                            v.push(*n);
                        }
                        i += 2;
                    }
                    Some(x) if *x == q => {
                        i += 1;
                        break;
                    }
                    Some('$') | Some('@') if q == '"' => return Err("interpolation in strings is not supported; use single quotes".into()),
                    Some(x) => {
                        v.push(*x);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(v));
        } else if ch == '/' && matches!(out.last(), Some(Tok::Op(o)) if o == "=~" || o == "!~") {
            i += 1;
            let mut re = String::new();
            loop {
                match c.get(i) {
                    None => return Err("unterminated regex".into()),
                    Some('\\') => {
                        re.push('\\');
                        if let Some(n) = c.get(i + 1) {
                            re.push(*n);
                        }
                        i += 2;
                    }
                    Some('/') => {
                        i += 1;
                        break;
                    }
                    Some(x) => {
                        re.push(*x);
                        i += 1;
                    }
                }
            }
            let st = i;
            while i < c.len() && c[i].is_ascii_alphabetic() {
                i += 1;
            }
            out.push(Tok::Re(re, c[st..i].iter().collect()));
        } else if ch == '(' {
            out.push(Tok::LP);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::RP);
            i += 1;
        } else if ch.is_ascii_alphabetic() {
            let st = i;
            while i < c.len() && c[i].is_ascii_alphabetic() {
                i += 1;
            }
            let w: String = c[st..i].iter().collect();
            match w.as_str() {
                "and" | "or" | "not" | "eq" | "ne" | "lt" | "gt" | "le" | "ge" => out.push(Tok::Op(w)),
                _ => return Err(format!("'{w}' is not supported in --if-filter (only variables, comparisons and && || ! are)")),
            }
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            let op = ["&&", "||", "==", "!=", "<=", ">=", "=~", "!~"].iter().find(|o| two == **o);
            if let Some(o) = op {
                out.push(Tok::Op(o.to_string()));
                i += 2;
            } else if matches!(ch, '!' | '<' | '>') {
                out.push(Tok::Op(ch.to_string()));
                i += 1;
            } else {
                return Err(format!("unexpected '{ch}' in --if-filter"));
            }
        }
    }
    Ok(out)
}

struct P {
    t: Vec<Tok>,
    i: usize,
    depth: usize,
}

impl P {
    fn peek_op(&self) -> Option<&str> {
        match self.t.get(self.i) {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }
    fn or(&mut self) -> Result<Node, String> {
        let mut l = self.and()?;
        while matches!(self.peek_op(), Some("||") | Some("or")) {
            self.i += 1;
            l = Node::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Node, String> {
        let mut l = self.not()?;
        while matches!(self.peek_op(), Some("&&") | Some("and")) {
            self.i += 1;
            l = Node::And(Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<Node, String> {
        self.depth += 1;
        if self.depth > 100 {
            return Err("--if-filter nested too deeply".into());
        }
        let r = if matches!(self.peek_op(), Some("!") | Some("not")) {
            self.i += 1;
            Ok(Node::Not(Box::new(self.not()?)))
        } else {
            self.cmp()
        };
        self.depth -= 1;
        r
    }
    fn cmp(&mut self) -> Result<Node, String> {
        let l = self.atom()?;
        let Some(op) = self.peek_op().map(str::to_string) else { return Ok(l) };
        match op.as_str() {
            "==" | "!=" | "<" | "<=" | ">" | ">=" | "eq" | "ne" | "lt" | "gt" | "le" | "ge" => {
                self.i += 1;
                let r = self.atom()?;
                Ok(Node::Cmp(op, Box::new(l), Box::new(r)))
            }
            "=~" | "!~" => {
                self.i += 1;
                let Some(Tok::Re(re, flags)) = self.t.get(self.i).cloned() else {
                    return Err("=~ needs a /regex/".into());
                };
                self.i += 1;
                let rx = RegexBuilder::new(&re)
                    .case_insensitive(flags.contains('i'))
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| format!("bad regex /{re}/: {e}"))?;
                Ok(Node::Match(op == "!~", Box::new(l), rx))
            }
            _ => Ok(l),
        }
    }
    fn atom(&mut self) -> Result<Node, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Var(v)) => {
                self.i += 1;
                Ok(Node::Var(v))
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Node::Lit(FVal::Num(n)))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Node::Lit(FVal::Str(s)))
            }
            Some(Tok::LP) => {
                self.i += 1;
                let e = self.or()?;
                if self.t.get(self.i) != Some(&Tok::RP) {
                    return Err("missing ')' in --if-filter".into());
                }
                self.i += 1;
                Ok(e)
            }
            other => Err(format!("unexpected {other:?} in --if-filter")),
        }
    }
}

impl Filter {
    pub fn compile(src: &str) -> Result<Filter, String> {
        let t = lex(src)?;
        // Depth counts parentheses and `not`; a chain (`a && a && ...`) is bounded by length.
        if t.len() > 1000 {
            return Err("--if-filter is longer than 1000 parts".into());
        }
        let mut p = P { t, i: 0, depth: 0 };
        let root = p.or()?;
        if p.i != p.t.len() {
            return Err(format!("unexpected {:?} in --if-filter", p.t[p.i]));
        }
        Ok(Filter { root, source: src.to_string() })
    }

    /// Evaluate; unknown variables are an error (a typo should not silently
    /// select nothing).
    pub fn eval(&self, vars: &HashMap<&str, FVal>) -> Result<bool, String> {
        Ok(ev(&self.root, vars)?.truthy())
    }
}

fn ev(n: &Node, vars: &HashMap<&str, FVal>) -> Result<FVal, String> {
    Ok(match n {
        Node::Var(v) => vars.get(v.as_str()).cloned().ok_or_else(|| format!("unknown variable ${v} in --if-filter"))?,
        Node::Lit(l) => l.clone(),
        Node::Not(e) => FVal::bool(!ev(e, vars)?.truthy()),
        Node::And(a, b) => {
            let x = ev(a, vars)?;
            if x.truthy() { ev(b, vars)? } else { x }
        }
        Node::Or(a, b) => {
            let x = ev(a, vars)?;
            if x.truthy() { x } else { ev(b, vars)? }
        }
        Node::Cmp(op, a, b) => {
            let (x, y) = (ev(a, vars)?, ev(b, vars)?);
            FVal::bool(match op.as_str() {
                "==" => x.num() == y.num(),
                "!=" => x.num() != y.num(),
                "<" => x.num() < y.num(),
                "<=" => x.num() <= y.num(),
                ">" => x.num() > y.num(),
                ">=" => x.num() >= y.num(),
                "eq" => x.text() == y.text(),
                "ne" => x.text() != y.text(),
                "lt" => x.text() < y.text(),
                "gt" => x.text() > y.text(),
                "le" => x.text() <= y.text(),
                _ => x.text() >= y.text(),
            })
        }
        Node::Match(neg, e, re) => FVal::bool(re.is_match(&ev(e, vars)?.text()) != *neg),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> HashMap<&'static str, FVal> {
        HashMap::from([
            ("default", FVal::Num(1.0)),
            ("if_type", FVal::Num(6.0)),
            ("if_oper", FVal::Str(String::new())),
            ("if_snmp_descr", FVal::Str("GigabitEthernet0/1".into())),
            ("if_speed", FVal::Num(125000000.0)),
        ])
    }

    #[test]
    fn typical_filters() {
        let v = vars();
        let t = |s: &str| Filter::compile(s).unwrap().eval(&v).unwrap();
        assert!(t("$default"));
        assert!(t("$if_type == 6 && $default"));
        assert!(!t("$if_oper"));
        assert!(t("not $if_oper and $if_snmp_descr =~ /gigabit/i"));
        assert!(t("$if_snmp_descr !~ /^Vlan/"));
        assert!(t("($if_speed > 10000000) || $if_type eq '24'"));
    }

    #[test]
    fn rejects_perl_code_and_typos() {
        for bad in ["system('id')", "`id`", "$default; unlink 'x'", "qx(id)", "$x =~ s/a/b/", "\"$ENV{HOME}\""] {
            assert!(Filter::compile(bad).is_err(), "{bad}");
        }
        assert!(Filter::compile("$defualt").unwrap().eval(&vars()).is_err());
    }
}
