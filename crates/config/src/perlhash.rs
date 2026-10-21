// SPDX-License-Identifier: GPL-2.0-or-later
//! Data-only parser for MRTG's `SnmpOptions` values.
//!
//! MRTG evaluates `SnmpOptions` as Perl code (`eval('{'.$v.'}')`), so any
//! Perl in that field runs at config load. LinkScope accepts only what the
//! keyword is documented to hold: a list of `key => value` pairs whose keys
//! are barewords or quoted strings and whose values are numbers or quoted
//! strings. Double-quoted strings may not interpolate (`$` or `@`).

use indexmap::IndexMap;

/// Parsed `SnmpOptions`, keys lowercased with any leading `-` removed.
pub type SnmpOptions = IndexMap<String, String>;

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Sep,
}

/// Parse a Perl hash-literal body such as `retries => 2, username => 'mrtg'`.
pub fn parse(src: &str) -> Result<SnmpOptions, String> {
    let toks = lex(src)?;
    let mut items: Vec<String> = Vec::new();
    let mut expect_item = true;
    for t in toks {
        match t {
            Tok::Sep => {
                if expect_item && !items.is_empty() {
                    // Perl tolerates repeated commas; keep it simple and allow them.
                }
                expect_item = true;
            }
            Tok::Word(w) | Tok::Str(w) => {
                if !expect_item {
                    return Err("missing ',' or '=>' between items".into());
                }
                items.push(w);
                expect_item = false;
            }
        }
    }
    if !items.len().is_multiple_of(2) {
        return Err("odd number of elements in option list".into());
    }
    let mut out = SnmpOptions::new();
    for pair in items.chunks(2) {
        let k = pair[0].trim_start_matches('-').to_lowercase();
        out.insert(k, pair[1].clone());
    }
    Ok(out)
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ',' {
            out.push(Tok::Sep);
            i += 1;
        } else if c == '=' && chars.get(i + 1) == Some(&'>') {
            out.push(Tok::Sep);
            i += 2;
        } else if c == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("unterminated string".into()),
                    Some('\\') if matches!(chars.get(i + 1), Some('\\') | Some('\'')) => {
                        s.push(chars[i + 1]);
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("unterminated string".into()),
                    Some('\\') => {
                        let n = chars.get(i + 1).copied().ok_or("unterminated string")?;
                        s.push(match n {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        i += 2;
                    }
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some('$') | Some('@') => {
                        return Err("variable interpolation is not allowed; use single quotes".into());
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_alphanumeric() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '.')) {
                i += 1;
            }
            let w: String = chars[start..i].iter().collect();
            let bare = w.trim_start_matches(['-', '+']);
            let is_ident = bare.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            let is_num = bare.parse::<f64>().is_ok();
            if !is_ident && !is_num {
                return Err(format!("'{w}' is neither a word nor a number"));
            }
            out.push(Tok::Word(w));
        } else {
            return Err(format!("unexpected character '{c}' (only key => value pairs are allowed)"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cfgmaker_v3_options() {
        let o = parse("username=>'mrtg',authpassword=>'s3cr3t',authprotocol=>'sha',privpassword=>'p',privprotocol=>'aes'").unwrap();
        assert_eq!(o["username"], "mrtg");
        assert_eq!(o["privprotocol"], "aes");
    }

    #[test]
    fn parses_numbers_and_dash_keys() {
        let o = parse("-retries => 3, timeout=>2.5, use_getbulk => 0,").unwrap();
        assert_eq!(o["retries"], "3");
        assert_eq!(o["timeout"], "2.5");
    }

    #[test]
    fn rejects_code() {
        assert!(parse("a => `id`").is_err());
        assert!(parse("a => system('id')").is_err());
        assert!(parse("a => \"$ENV{HOME}\"").is_err());
        assert!(parse("a").is_err());
    }
}
