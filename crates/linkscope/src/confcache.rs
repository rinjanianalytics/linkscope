// SPDX-License-Identifier: GPL-2.0-or-later
//! MRTG's interface cache (the `.ok` file next to the config).
//!
//! Maps `(host, method, key)` to an ifIndex, where method is one of
//! `Descr`, `Name`, `Type`, `Ip`, `Eth`. The on-disk format is MRTG's:
//! `host<TAB>method<TAB>key<TAB>value` lines, so the file can be shared with
//! MRTG. Fixes compared with MRTG:
//! * every host is written back (MRTG dropped hosts that were not re-scanned
//!   in the same run whenever any host was re-scanned);
//! * the daemon persists the cache after each cycle (MRTG only wrote it on exit);
//! * the file is replaced atomically and never through a symlink.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;

/// Seconds a host that did not answer a table walk is skipped (MRTG: 300).
pub const DEAD_HOST_SECS: u64 = 300;

/// MRTG's `cleanhostkey`: drop the timeout/retries/backoff fields from
/// `community@host:port:timeout:retries:backoff:version` and replace `:`.
pub fn clean_host_key(host: &str) -> String {
    static FULL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(:\d*)(?:(:\d*)(?:(:\d*)(?:(:\d*)(?:(:\d*)))))$").unwrap());
    static PART: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(:\d*)(?:(:\d*)(?:(:\d*)(?:(:\d*)?)?)?)$").unwrap());
    let s = if FULL.is_match(host) {
        FULL.replace(host, "${1}${5}").into_owned()
    } else {
        PART.replace(host, "${1}").into_owned()
    };
    s.replace(':', "_")
}

/// Normalise a cached value or key like MRTG: control characters and spaces
/// become one space each, trailing spaces are removed.
fn tidy(v: &str) -> String {
    let s: String = v.chars().map(|c| if c <= ' ' { ' ' } else { c }).collect();
    s.trim_end_matches(' ').to_string()
}

/// Strip trailing control characters and spaces (`s/[\0- ]+$//`).
pub fn trim_ctl(v: &str) -> &str {
    v.trim_end_matches(|c: char| c <= ' ')
}

#[derive(Debug, Default)]
pub struct ConfCache {
    map: BTreeMap<String, BTreeMap<String, BTreeMap<String, Option<String>>>>,
    dead: HashMap<String, Instant>,
    dirty: bool,
}

impl ConfCache {
    /// Read an `.ok` file; a missing or unreadable file gives an empty cache. A file others can
    /// read is written again, owner only, at the next save.
    pub fn load(path: &Path) -> ConfCache {
        let mut c = ConfCache::default();
        let text = match crate::fsutil::read_state(path, crate::fsutil::MAX_STATE_BYTES) {
            Ok(Some((text, open))) => {
                c.dirty = open;
                text
            }
            Ok(None) => return c,
            Err(e) => {
                tracing::warn!(
                    "reading {}: {e}; starting with an empty cache",
                    path.display()
                );
                return c;
            }
        };
        for line in String::from_utf8_lossy(&text).lines() {
            if !line.contains('\t') {
                continue;
            }
            if line
                .split_whitespace()
                .next()
                .is_some_and(|w| w.ends_with(':'))
            {
                continue; // legacy lines
            }
            let mut f = line.split('\t');
            let (Some(h), Some(m), Some(k)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let v = f.next().unwrap_or("");
            c.map
                .entry(h.to_string())
                .or_default()
                .entry(m.to_string())
                .or_default()
                .insert(tidy(trim_ctl(k)), Some(v.to_string()));
        }
        c
    }

    /// Write the cache atomically (temp file plus rename, no symlink follow).
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.dirty && path.exists() {
            return Ok(());
        }
        let mut out = String::new();
        for (h, methods) in &self.map {
            for (m, keys) in methods {
                for (k, v) in keys {
                    if let Some(v) = v {
                        out.push_str(&format!("{h}\t{m}\t{k}\t{v}\n"));
                    }
                }
            }
        }
        crate::fsutil::write_atomic(path, out.as_bytes(), crate::fsutil::STATE_MODE)?;
        self.dirty = false;
        Ok(())
    }

    /// `readfromcache`.
    pub fn read(&self, host: &str, method: &str, key: &str) -> Option<&str> {
        self.map
            .get(&clean_host_key(host))?
            .get(method)?
            .get(key)?
            .as_deref()
    }

    /// `storeincache`: a conflicting second value turns the entry into `Dup`.
    pub fn store(&mut self, host: &str, method: &str, key: &str, value: Option<&str>) {
        self.dirty = true;
        let slot = self
            .map
            .entry(clean_host_key(host))
            .or_default()
            .entry(method.to_string())
            .or_default()
            .entry(key.to_string())
            .or_insert(None);
        match value {
            None => *slot = None,
            Some(v) => {
                let v = tidy(v);
                *slot = match slot.take() {
                    Some(old) if old != v => Some("Dup".into()),
                    _ => Some(v),
                };
            }
        }
    }

    /// Drop everything known about `host` before a re-scan.
    pub fn clear_host(&mut self, host: &str) {
        self.dirty = true;
        self.map.remove(&clean_host_key(host));
    }

    pub fn mark_dead(&mut self, host: &str) {
        self.dead.insert(clean_host_key(host), Instant::now());
    }

    pub fn is_dead(&self, host: &str) -> bool {
        self.dead
            .get(&clean_host_key(host))
            .is_some_and(|t| t.elapsed() < Duration::from_secs(DEAD_HOST_SECS))
    }
}

/// Render a binary MAC like MRTG: lowercase hex bytes joined by `-`.
pub fn mac_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_host_key_matches_mrtg() {
        assert_eq!(
            clean_host_key("public@router:161:2:5:1:2"),
            "public@router_161_2"
        );
        assert_eq!(clean_host_key("public@router:161:2"), "public@router_161");
        assert_eq!(clean_host_key("public@router"), "public@router");
        assert_eq!(clean_host_key("public@router:::::2"), "public@router__2");
    }

    #[test]
    fn dup_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ok");
        let mut c = ConfCache::default();
        c.store("p@h:161", "Descr", "eth0", Some("2"));
        c.store("p@h:161", "Descr", "eth0", Some("3"));
        c.store("p@h:161", "Name", "Gi0/1", Some("5 "));
        c.store("q@other", "Ip", "10.0.0.1", Some("7"));
        c.save(&p).unwrap();
        let d = ConfCache::load(&p);
        assert_eq!(d.read("p@h:161", "Descr", "eth0"), Some("Dup"));
        assert_eq!(d.read("p@h:161", "Name", "Gi0/1"), Some("5"));
        assert_eq!(d.read("q@other", "Ip", "10.0.0.1"), Some("7"));
    }
}
