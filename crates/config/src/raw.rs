// SPDX-License-Identifier: GPL-2.0-or-later
//! Line-level reader for `mrtg.cfg`, a port of MRTG's `readcfg`.
//!
//! The reader reproduces MRTG's grammar exactly: global `Key: value` lines,
//! per-target `Key[name]: value` lines, the `_` (default), `^` (prefix) and
//! `$` (suffix) pseudo-targets, whitespace continuation lines, comments and
//! `Include:` directives. Each finished key is validated by
//! [`crate::rules::quickcheck`] at the same point MRTG validates it, so error
//! messages appear for the same lines.
//!
//! Deliberate differences from MRTG (security and robustness fixes):
//! * glob includes are read in sorted order (MRTG read them in reverse);
//! * include depth is limited and include cycles are rejected;
//! * every file read can be subjected to a [`crate::trust::TrustPolicy`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use crate::rules;
use crate::trust::{TrustPolicy, check_file_trust};
use crate::ConfigError;

/// Maximum nesting depth of `Include:` directives.
pub const MAX_INCLUDE_DEPTH: usize = 16;
/// Files one configuration may read in all, Includes counted. Depth alone left the same file
/// includable again and again at every level: 16 levels of N each read N^16 files.
pub const MAX_CONFIG_FILES: usize = 1000;
/// Bytes one configuration may read in all.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024 * 1024;

/// What is left to read of [`MAX_CONFIG_FILES`] and [`MAX_CONFIG_BYTES`].
struct Budget {
    files: usize,
    bytes: u64,
}

/// The result of reading a configuration file, before semantic checks.
///
/// Keys are lowercased exactly like MRTG does. `per_target` mirrors MRTG's
/// `%rcfg` hash: `per_target[key][target] = value`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RawConfig {
    /// Path of the top-level configuration file.
    pub path: PathBuf,
    /// Global keywords (`%cfg`).
    pub global: IndexMap<String, String>,
    /// Target names in order of first appearance (`@routers`).
    pub targets: Vec<String>,
    /// Per-target keywords (`%rcfg`).
    pub per_target: IndexMap<String, IndexMap<String, String>>,
    /// Every file that was read, top-level first.
    pub files: Vec<PathBuf>,
}

impl RawConfig {
    /// Look up `Key[target]`.
    pub fn get(&self, key: &str, target: &str) -> Option<&str> {
        self.per_target.get(key).and_then(|m| m.get(target)).map(String::as_str)
    }

    fn set(&mut self, key: &str, target: &str, value: String) {
        self.per_target
            .entry(key.to_string())
            .or_default()
            .insert(target.to_string(), value);
    }

    fn append(&mut self, key: &str, target: &str, value: &str) {
        self.per_target
            .entry(key.to_string())
            .or_default()
            .entry(target.to_string())
            .or_default()
            .push_str(value);
    }
}

/// Options controlling [`read_config`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOptions {
    /// How to treat configuration files that other accounts could modify.
    pub trust: TrustPolicy,
}

struct Source {
    lines: std::vec::IntoIter<String>,
    file: PathBuf,
    line_no: usize,
}

struct Patterns {
    include: Regex,
    target_line: Regex,
    global_line: Regex,
    continuation: Regex,
}

impl Patterns {
    fn new() -> Self {
        Patterns {
            include: Regex::new(r"(?i)^include:[ \t\n\r\x0c]*(.*?[^ \t\n\r\x0c])[ \t\n\r\x0c]*$").unwrap(),
            target_line: Regex::new(r"^([A-Za-z0-9*]+)\[([^ \t\n\r\x0c]+)\][ \t\n\r\x0c]*:[ \t\n\r\x0c]*(.*[^ \t\n\r\x0c]?)[ \t\n\r\x0c]*$").unwrap(),
            global_line: Regex::new(r"^([^ \t\n\r\x0c]+):[ \t\n\r\x0c]*(.*[^ \t\n\r\x0c])[ \t\n\r\x0c]*$").unwrap(),
            continuation: Regex::new(r"^[ \t\n\r\x0c]+(.*[^ \t\n\r\x0c])[ \t\n\r\x0c]*$").unwrap(),
        }
    }
}

fn load_lines(path: &Path, opts: &ReadOptions, budget: &mut Budget) -> Result<Vec<String>, ConfigError> {
    use std::os::unix::fs::OpenOptionsExt;
    if budget.files == 0 {
        return Err(ConfigError::at(path, 0, format!("more than {MAX_CONFIG_FILES} configuration files read in all (Include)")));
    }
    budget.files -= 1;
    // Not blocking, and regular files only: a FIFO or a device named by an Include hung the
    // reader or streamed forever.
    let f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path).map_err(|e| ConfigError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    check_file_trust(path, &f, opts.trust)?;
    if !f.metadata().is_ok_and(|m| m.is_file()) {
        return Err(ConfigError::at(path, 0, "not a regular file"));
    }
    let mut data = Vec::new();
    f.take(budget.bytes.saturating_add(1)).read_to_end(&mut data).map_err(|e| ConfigError::Io { path: path.to_path_buf(), source: e })?;
    if data.len() as u64 > budget.bytes {
        return Err(ConfigError::at(path, 0, format!("more than {} MB of configuration read in all (Include)", MAX_CONFIG_BYTES >> 20)));
    }
    budget.bytes -= data.len() as u64;
    let mut out = Vec::new();
    for line in BufReader::new(&data[..]).split(b'\n') {
        let line = line.map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        // MRTG reads bytes; we accept invalid UTF-8 lossily rather than failing.
        out.push(String::from_utf8_lossy(&line).into_owned());
    }
    Ok(out)
}

/// Read `path` with MRTG's `readcfg` semantics.
pub fn read_config(path: &Path, opts: &ReadOptions) -> Result<RawConfig, ConfigError> {
    let pat = Patterns::new();
    let mut raw = RawConfig {
        path: path.to_path_buf(),
        ..Default::default()
    };
    let top_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

    let mut budget = Budget { files: MAX_CONFIG_FILES, bytes: MAX_CONFIG_BYTES };
    let mut stack: Vec<Source> = vec![Source {
        lines: load_lines(path, opts, &mut budget)?.into_iter(),
        file: path.to_path_buf(),
        line_no: 0,
    }];
    raw.files.push(path.to_path_buf());
    let mut open_files: HashSet<PathBuf> = HashSet::new();
    open_files.insert(canonical(path));

    // MRTG state.
    let mut first: Option<String> = None;
    // Perl's `$second`: Some("") after a global line, Some(name) after a target line.
    let mut second: Option<String> = None;
    let mut pre: HashMap<String, String> = HashMap::new();
    let mut post: HashMap<String, String> = HashMap::new();
    let mut deflt: IndexMap<String, String> = IndexMap::new();
    let mut defaulted: HashSet<(String, String)> = HashSet::new();
    let mut seen: HashMap<(String, String), (PathBuf, usize)> = HashMap::new();
    let mut known_targets: HashSet<String> = HashSet::new();
    let mut last_file = path.to_path_buf();
    let mut last_line = 0usize;

    while let Some(top) = stack.last_mut() {
        let Some(mut line) = top.lines.next() else {
            let finished = stack.pop().expect("non-empty");
            open_files.remove(&canonical(&finished.file));
            continue;
        };
        top.line_no += 1;
        let file = top.file.clone();
        let line_no = top.line_no;
        last_file = file.clone();
        last_line = line_no;

        if let Some(c) = pat.include.captures(&line) {
            let spec = c[1].to_string();
            let mut files: Vec<PathBuf> = Vec::new();
            // A relative name is relative to the top configuration's directory, MRTG's own
            // fallback. MRTG tried the working directory first, so where the daemon was started
            // decided which file was read; that is no longer the case.
            let spec_path = if Path::new(&spec).is_absolute() { PathBuf::from(&spec) } else { top_dir.join(&spec) };
            if spec.contains('*') {
                files = glob_sorted(&spec_path.to_string_lossy());
            } else {
                files.push(spec_path);
            }
            if stack.len() + 1 > MAX_INCLUDE_DEPTH {
                return Err(ConfigError::at(&file, line_no, format!(
                    "Include nesting deeper than {MAX_INCLUDE_DEPTH} levels"
                )));
            }
            // Push in reverse so the first file is read first.
            for f in files.into_iter().rev() {
                let key = canonical(&f);
                if open_files.contains(&key) {
                    return Err(ConfigError::at(&file, line_no, format!(
                        "Include cycle: {} includes itself", f.display()
                    )));
                }
                let lines = load_lines(&f, opts, &mut budget).map_err(|e| match e {
                    ConfigError::Io { path, .. } => ConfigError::at(&file, line_no, format!(
                        "unable to open include file: {}", path.display()
                    )),
                    other => other,
                })?;
                open_files.insert(key);
                raw.files.push(f.clone());
                stack.push(Source { lines: lines.into_iter(), file: f, line_no: 0 });
            }
            continue;
        }

        line = line.replace('\t', " ");
        if line.ends_with('\r') {
            line.pop();
        }
        while line.ends_with(' ') {
            line.pop();
        }
        if is_comment_or_blank(&line) {
            continue;
        }
        // `get`: byte 7 may fall inside a character (`Title[é]: x` panicked here).
        if line.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("supress")) {
            line = format!("suppress{}", &line[7..]);
        }

        // Continuation line.
        if first.is_some()
            && let Some(c) = pat.continuation.captures(&line) {
                let text = c[1].to_string();
                let f = first.as_deref().unwrap();
                match second.as_deref() {
                    Some("^") => push_nl(pre.entry(f.to_string()).or_default(), &text),
                    Some("$") => push_nl(post.entry(f.to_string()).or_default(), &text),
                    Some("_") => push_nl(deflt.entry(f.to_string()).or_default(), &text),
                    // MRTG quirk, reproduced on purpose: after a *global* key `$second`
                    // is '' so the continuation lands in `$rcfg{key}{''}` and is
                    // effectively ignored. `check` warns about it.
                    Some(t) => raw.append(f, t, &format!(" {text}")),
                    None => {}
                }
                continue;
            }

        finish_key(&mut raw, &first, &second, &post, &deflt, &mut defaulted, &file, line_no)?;

        if let Some(c) = pat.target_line.captures(&line) {
            let mut key = c[1].to_lowercase();
            let target = c[2].to_lowercase();
            let value = c[3].to_string();
            if key == "colors" {
                key = "colours".into();
            }
            first = Some(key.clone());
            second = Some(target.clone());
            match target.as_str() {
                "^" => {
                    if value.is_empty() { pre.remove(&key); } else { pre.insert(key, value); }
                    continue;
                }
                "$" => {
                    if value.is_empty() { post.remove(&key); } else { post.insert(key, value); }
                    continue;
                }
                "_" => {
                    if value.is_empty() { deflt.shift_remove(&key); } else { deflt.insert(key, value); }
                    continue;
                }
                _ => {}
            }
            if known_targets.insert(target.clone()) {
                raw.targets.push(target.clone());
            }
            for (k, v) in &deflt {
                if raw.get(k, &target).is_none() {
                    raw.set(k, &target, v.clone());
                    defaulted.insert((k.clone(), target.clone()));
                }
            }
            for (k, v) in &pre {
                if raw.get(k, &target).is_none() {
                    defaulted.remove(&(k.clone(), target.clone()));
                    let nsc = raw.global.get("nospacechar").cloned();
                    raw.set(k, &target, apply_nospace(v, nsc.as_deref(), true));
                }
            }
            let id = (key.clone(), target.clone());
            if let Some((pf, pl)) = seen.get(&id) {
                return Err(ConfigError::at(&file, line_no, format!(
                    "duplicate definition for {key}[{target}]. First definition is on line {pl} of {}",
                    pf.display()
                )));
            }
            seen.insert(id.clone(), (file.clone(), line_no));
            if defaulted.remove(&id) {
                raw.set(&key, &target, String::new());
            }
            raw.append(&key, &target, &value);
            continue;
        }
        if let Some(c) = pat.global_line.captures(&line) {
            let key = c[1].to_lowercase();
            raw.global.insert(key.clone(), c[2].to_string());
            first = Some(key);
            second = Some(String::new());
            continue;
        }
        return Err(ConfigError::at(&file, line_no, format!("Line ({line}) does not make sense")));
    }

    finish_key(&mut raw, &first, &second, &post, &deflt, &mut defaulted, &last_file, last_line)?;
    Ok(raw)
}

/// Apply the pending `$` suffix and validate the key that just ended.
#[allow(clippy::too_many_arguments)]
fn finish_key(
    raw: &mut RawConfig,
    first: &Option<String>,
    second: &Option<String>,
    post: &HashMap<String, String>,
    deflt: &IndexMap<String, String>,
    defaulted: &mut HashSet<(String, String)>,
    file: &Path,
    line: usize,
) -> Result<(), ConfigError> {
    let Some(f) = first.as_deref() else { return Ok(()) };
    if let Some(t) = second.as_deref()
        && !matches!(t, "^" | "$" | "_")
            && let Some(p) = post.get(f) {
                let id = (f.to_string(), t.to_string());
                if defaulted.remove(&id) {
                    raw.set(f, t, p.clone());
                } else {
                    let nsc = raw.global.get("nospacechar").cloned();
                    let s = apply_nospace(p, nsc.as_deref(), false);
                    raw.append(f, t, &s);
                }
            }
    match second.as_deref() {
        Some("_") => {
            if let Some(v) = deflt.get(f) {
                rules::quickcheck(f, true, v, file, line)?;
            }
        }
        Some("^") | Some("$") | None => {}
        Some("") => {
            let v = raw.global.get(f).cloned().unwrap_or_default();
            rules::quickcheck(f, false, &v, file, line)?;
        }
        Some(t) => {
            let v = raw.get(f, t).unwrap_or("").to_string();
            rules::quickcheck(f, true, &v, file, line)?;
        }
    }
    Ok(())
}

/// MRTG's NoSpaceChar handling. For prefixes (`is_prefix`) the default is to
/// append a space; for suffixes the default is to prepend one.
fn apply_nospace(v: &str, nospacechar: Option<&str>, is_prefix: bool) -> String {
    if let Some(n) = nospacechar.filter(|n| !n.is_empty())
        && let Some(stripped) = v.strip_suffix(n) {
            // MRTG uses `m/(.*)\Q$nsc\E$/` and `.` does not match a newline,
            // so for a multi-line prefix/suffix only the last line survives.
            let last = stripped.rsplit('\n').next().unwrap_or(stripped);
            return last.to_string();
        }
    if is_prefix { format!("{v} ") } else { format!(" {v}") }
}

fn push_nl(s: &mut String, text: &str) {
    s.push('\n');
    s.push_str(text);
}

fn is_comment_or_blank(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    t.is_empty() || t.starts_with('#')
}

fn glob_sorted(pattern: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = glob::glob(pattern)
        .map(|g| g.filter_map(Result::ok).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
