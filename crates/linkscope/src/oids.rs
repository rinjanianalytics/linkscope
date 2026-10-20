// SPDX-License-Identifier: GPL-2.0-or-later
//! OID name resolution, compatible with MRTG's `SNMP_util::toOID`.
//!
//! Accepts dotted numeric OIDs (an optional leading dot is ignored), a
//! known name (`ifHCInOctets`), or a name followed by numeric
//! components (`ifInOctets.3`). Names come from MRTG's built-in table and
//! from MIB files listed in `LoadMIBs:`.

use std::collections::HashMap;
use std::path::Path;

use regex::Regex;

use crate::oid_table::OIDS;

/// Name to numeric-OID resolver.
#[derive(Debug, Clone)]
pub struct OidResolver {
    names: HashMap<String, String>,
    lower: HashMap<String, String>,
}

impl Default for OidResolver {
    fn default() -> Self {
        let mut r = OidResolver { names: HashMap::new(), lower: HashMap::new() };
        for (n, o) in OIDS {
            r.insert(n, o);
        }
        r
    }
}

impl OidResolver {
    fn insert(&mut self, name: &str, oid: &str) {
        self.names.insert(name.to_string(), oid.to_string());
        self.lower.entry(name.to_lowercase()).or_insert_with(|| oid.to_string());
    }

    /// Resolve `spec` to a numeric OID string without a leading dot.
    pub fn resolve(&self, spec: &str) -> Option<String> {
        let spec = spec.trim().trim_start_matches('.');
        if spec.is_empty() {
            return None;
        }
        let (head, rest) = match spec.split_once('.') {
            Some((h, r)) => (h, Some(r)),
            None => (spec, None),
        };
        let base = if head.bytes().all(|b| b.is_ascii_digit()) {
            head.to_string()
        } else {
            self.names.get(head).or_else(|| self.lower.get(&head.to_lowercase()))?.clone()
        };
        let full = match rest {
            Some(r) => format!("{base}.{r}"),
            None => base,
        };
        full.split('.').all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit())).then_some(full)
    }

    /// Parse `OBJECT IDENTIFIER` / `OBJECT-TYPE` assignments from a MIB file
    /// (the subset MRTG's `snmpMIB_to_OID` understands). Returns how many
    /// names were added.
    pub fn load_mib(&mut self, path: &Path) -> std::io::Result<usize> {
        let text = std::fs::read_to_string(path)?;
        // Strip ASN.1 comments ("--" to end of line).
        let text: String = text.lines().map(|l| l.split("--").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        let re = Regex::new(
            r"(?s)([A-Za-z][-A-Za-z0-9]*)\s+(?:OBJECT\s+IDENTIFIER|OBJECT-TYPE|OBJECT-IDENTITY|MODULE-IDENTITY|NOTIFICATION-TYPE|OBJECT-GROUP)\b.*?::=\s*\{\s*([A-Za-z][-A-Za-z0-9]*)\s+([0-9]+)\s*\}",
        )
        .expect("mib regex");
        let mut pending: Vec<(String, String, String)> = re
            .captures_iter(&text)
            .map(|c| (c[1].to_string(), c[2].to_string(), c[3].to_string()))
            .collect();
        let roots = [("iso", "1"), ("org", "1.3"), ("dod", "1.3.6"), ("internet", "1.3.6.1"), ("mib-2", "1.3.6.1.2.1"), ("enterprises", "1.3.6.1.4.1"), ("private", "1.3.6.1.4")];
        for (n, o) in roots {
            self.names.entry(n.to_string()).or_insert_with(|| o.to_string());
        }
        let mut added = 0;
        loop {
            let before = pending.len();
            pending.retain(|(name, parent, num)| {
                if let Some(p) = self.names.get(parent).cloned() {
                    let oid = format!("{p}.{num}");
                    if !self.names.contains_key(name) {
                        self.insert(name, &oid);
                        added += 1;
                    }
                    false
                } else {
                    true
                }
            });
            if pending.len() == before {
                break;
            }
        }
        Ok(added)
    }
}

/// Parse a dotted numeric OID into components.
pub fn parse_numeric(oid: &str) -> Option<Vec<u64>> {
    oid.trim_start_matches('.').split('.').map(|c| c.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_snmp_util() {
        let r = OidResolver::default();
        assert_eq!(r.resolve("ifHCInOctets.5").unwrap(), "1.3.6.1.2.1.31.1.1.1.6.5");
        assert_eq!(r.resolve("sysUptime").unwrap(), "1.3.6.1.2.1.1.3.0");
        assert_eq!(r.resolve(".1.3.6.1.2.1.2.2.1.10.3").unwrap(), "1.3.6.1.2.1.2.2.1.10.3");
        assert_eq!(r.resolve("ifInOctets").unwrap(), "1.3.6.1.2.1.2.2.1.10");
        assert!(r.resolve("noSuchName.1").is_none());
        assert!(r.resolve("1.3.x").is_none());
    }

    #[test]
    fn loads_mib_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("TEST-MIB.txt");
        std::fs::write(&p, "acme OBJECT IDENTIFIER ::= { enterprises 99999 }\n\
            acmeCounter OBJECT-TYPE\n SYNTAX Counter32 -- a comment ::= { bogus 1 }\n MAX-ACCESS read-only\n ::= { acme 7 }\n").unwrap();
        let mut r = OidResolver::default();
        assert_eq!(r.load_mib(&p).unwrap(), 2);
        assert_eq!(r.resolve("acmeCounter.0").unwrap(), "1.3.6.1.4.1.99999.7.0");
    }
}
