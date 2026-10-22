// SPDX-License-Identifier: GPL-2.0-or-later
//! gNMI data sources, a LinkScope addition (MRTG reads SNMP only).
//!
//! A target reads one interface of a gNMI device with `gnmi:<interface>@<host>[:port]`, for
//! example `Target[srl1_e1]: gnmi:ethernet-1/1@srl1.example.net`. How to reach the device comes
//! from `GnmiOptions[]`, the way `SnmpOptions[]` works: space-separated `key=value` pairs, usually
//! set once for all targets with `GnmiOptions[_]:`.
//!
//! The password is never written in the config itself: it comes from a file (`password-file=`)
//! or an environment variable (`password-env=`). Paths and the password stay out of every output.

use std::path::PathBuf;

use serde::Serialize;

/// The port gNMI servers listen on by convention (IANA's `gnmi` registration).
pub const DEFAULT_PORT: u16 = 9339;

/// How the connection is protected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// TLS, and the device's certificate must chain to a trusted CA and match its name (default).
    #[default]
    Verify,
    /// TLS without checking the certificate (`tls=insecure`): encrypted, but anyone on the path
    /// can pretend to be the device.
    Insecure,
    /// No TLS at all (`tls=off`): everything, the password included, travels in clear.
    Off,
}

/// Which set of paths a device answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GnmiModel {
    /// OpenConfig (`/interfaces/interface[name=…]/state/counters/…`), the default: Arista,
    /// Cisco, Juniper and Nokia all publish it.
    #[default]
    OpenConfig,
    /// Nokia SR Linux's own model (`/interface[name=…]/statistics/…`), for SR Linux with
    /// OpenConfig turned off.
    Srl,
}

impl GnmiModel {
    pub fn name(self) -> &'static str {
        match self {
            GnmiModel::OpenConfig => "openconfig",
            GnmiModel::Srl => "srl",
        }
    }
}

/// `GnmiOptions[]`: how to reach and read a gNMI device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct GnmiOptions {
    /// The user name sent with each request (gRPC metadata `username`).
    pub user: Option<String>,
    /// A file holding the password (first line). Never serialized.
    #[serde(skip)]
    pub password_file: Option<PathBuf>,
    /// An environment variable holding the password. Never serialized.
    #[serde(skip)]
    pub password_env: Option<String>,
    pub tls: TlsMode,
    /// CA certificates (PEM) to trust instead of the system's. Never serialized.
    #[serde(skip)]
    pub ca: Option<PathBuf>,
    /// A client certificate and key (PEM) for mutual TLS. Never serialized.
    #[serde(skip)]
    pub cert: Option<PathBuf>,
    #[serde(skip)]
    pub key: Option<PathBuf>,
    /// The name to check the device's certificate against, when it differs from the host
    /// written in the target (for example a device reached by its IP address).
    pub tls_name: Option<String>,
    pub model: GnmiModel,
    /// The gNMI `target` field, for devices behind a gNMI proxy or gateway.
    pub target: Option<String>,
}

impl GnmiOptions {
    /// Parse `key=value key=value …`. Errors name the key, never the value.
    pub fn parse(text: &str) -> Result<GnmiOptions, String> {
        let mut o = GnmiOptions::default();
        let mut seen = std::collections::HashSet::new();
        for pair in text.split_whitespace() {
            let Some((k, v)) = pair.split_once('=') else {
                return Err(format!(
                    "\"{}\" is not key=value",
                    pair.chars().take(40).collect::<String>()
                ));
            };
            let k = k.to_ascii_lowercase();
            if k == "password" {
                return Err(
                    "the password is not written in the config: put it in a file \
                            (password-file=) or an environment variable (password-env=)"
                        .into(),
                );
            }
            if !seen.insert(k.clone()) {
                return Err(format!("{k}= is given twice"));
            }
            if v.is_empty() {
                return Err(format!("{k}= is empty"));
            }
            match k.as_str() {
                "user" => o.user = Some(v.to_string()),
                "password-file" => o.password_file = Some(PathBuf::from(v)),
                "password-env" => o.password_env = Some(v.to_string()),
                "tls" => {
                    o.tls = match v.to_ascii_lowercase().as_str() {
                        "verify" | "yes" | "on" => TlsMode::Verify,
                        "insecure" => TlsMode::Insecure,
                        "off" | "no" => TlsMode::Off,
                        _ => return Err("tls= must be verify, insecure or off".into()),
                    }
                }
                "ca" => o.ca = Some(PathBuf::from(v)),
                "cert" => o.cert = Some(PathBuf::from(v)),
                "key" => o.key = Some(PathBuf::from(v)),
                "tls-name" => o.tls_name = Some(v.to_string()),
                "model" => {
                    o.model = match v.to_ascii_lowercase().as_str() {
                        "openconfig" | "oc" => GnmiModel::OpenConfig,
                        "srl" => GnmiModel::Srl,
                        _ => return Err("model= must be openconfig or srl".into()),
                    }
                }
                "target" => o.target = Some(v.to_string()),
                _ => {
                    return Err(format!(
                        "{k}= is unknown (known: user, password-file, password-env, tls, ca, \
                         cert, key, tls-name, model, target)"
                    ));
                }
            }
        }
        o.consistent()?;
        Ok(o)
    }

    fn consistent(&self) -> Result<(), String> {
        if self.password_file.is_some() && self.password_env.is_some() {
            return Err("give password-file= or password-env=, not both".into());
        }
        let has_password = self.password_file.is_some() || self.password_env.is_some();
        if self.user.is_some() != has_password {
            return Err(
                "user= and a password (password-file= or password-env=) go together".into(),
            );
        }
        if self.cert.is_some() != self.key.is_some() {
            return Err("cert= and key= go together".into());
        }
        if self.tls != TlsMode::Verify && (self.ca.is_some() || self.tls_name.is_some()) {
            return Err("ca= and tls-name= need tls=verify (the default)".into());
        }
        if self.tls == TlsMode::Off && self.cert.is_some() {
            return Err("cert= and key= need TLS (tls=verify or tls=insecure)".into());
        }
        Ok(())
    }

    /// The options as `GnmiOptions[]` text, for configs that LinkScope writes (`cfgmaker`).
    pub fn to_text(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut kv = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                out.push(format!("{k}={v}"));
            }
        };
        kv("user", self.user.clone());
        kv(
            "password-file",
            self.password_file.as_ref().map(|p| p.display().to_string()),
        );
        kv("password-env", self.password_env.clone());
        kv(
            "tls",
            match self.tls {
                TlsMode::Verify => None,
                TlsMode::Insecure => Some("insecure".into()),
                TlsMode::Off => Some("off".into()),
            },
        );
        kv("ca", self.ca.as_ref().map(|p| p.display().to_string()));
        kv("cert", self.cert.as_ref().map(|p| p.display().to_string()));
        kv("key", self.key.as_ref().map(|p| p.display().to_string()));
        kv("tls-name", self.tls_name.clone());
        kv("model", Some(self.model.name().to_string()));
        kv("target", self.target.clone());
        out.join(" ")
    }

    /// True when a password is configured.
    pub fn has_password(&self) -> bool {
        self.password_file.is_some() || self.password_env.is_some()
    }
}

/// One gNMI data source: one interface of one device.
#[derive(Debug, Clone, Serialize)]
pub struct GnmiSource {
    /// The interface name as the device knows it (`ethernet-1/1`, `Ethernet1`, `xe-0/0/0`).
    pub interface: String,
    /// Host as written; numeric IPv6 keeps its brackets.
    pub host: String,
    pub port: u16,
    pub options: GnmiOptions,
}

impl GnmiSource {
    /// `host:port` for display and labels (no credentials in it).
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// The `gnmi:<interface>@<host>[:port]` token of a `Target[]`.
pub(crate) fn pattern() -> &'static regex::bytes::Regex {
    use std::sync::LazyLock;
    static R: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
        // Group 1: everything before (empty, or ending in a non-word character, so `xgnmi:` is
        // not a token; `??` so the first token wins). 2: the whole token. 3: interface (no space, no @). 4: IPv6 host in
        // brackets. 5: DNS name or IPv4. 6: port. 7: the rest.
        regex::bytes::RegexBuilder::new(
            r"^((?:.*?[^A-Za-z0-9_])??)(gnmi:([^ \t\n\r\x0b\x0c@`]+)@(?:(\[[a-fA-F0-9:]+\])|([-A-Za-z0-9_]+(?:\.[-A-Za-z0-9_]+)*))(?::([0-9]{1,5}))?)(.*)$",
        )
        .unicode(false)
        .dot_matches_new_line(true)
        .build()
        .expect("gnmi target regex")
    });
    &R
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_and_reject() {
        let o = GnmiOptions::parse(
            "user=linkscope password-file=/etc/ls/pass ca=/etc/ls/ca.pem model=srl tls-name=srl1",
        )
        .unwrap();
        assert_eq!(o.user.as_deref(), Some("linkscope"));
        assert_eq!(o.model, GnmiModel::Srl);
        assert_eq!(o.tls, TlsMode::Verify);
        assert_eq!(GnmiOptions::parse("").unwrap(), GnmiOptions::default());

        let err = GnmiOptions::parse("user=a password=hunter2-SECRET").unwrap_err();
        assert!(
            err.contains("password-file") && !err.contains("hunter2"),
            "{err}"
        );
        let err = GnmiOptions::parse("user=a password-file=/p tls=bogus-SECRET").unwrap_err();
        assert!(!err.contains("SECRET"), "{err}");
        assert!(GnmiOptions::parse("user=a").is_err());
        assert!(GnmiOptions::parse("password-file=/p").is_err());
        assert!(GnmiOptions::parse("cert=/c").is_err());
        assert!(GnmiOptions::parse("tls=insecure ca=/ca").is_err());
        assert!(GnmiOptions::parse("tls=off cert=/c key=/k").is_err());
        assert!(GnmiOptions::parse("model=cisco").is_err());
        assert!(GnmiOptions::parse("model=oc model=srl").is_err());
        assert!(GnmiOptions::parse("colour=red").is_err());
        let text = "user=u password-env=PW tls=insecure model=srl target=r1";
        assert_eq!(GnmiOptions::parse(text).unwrap().to_text(), text);
    }

    #[test]
    fn the_token_needs_a_boundary() {
        let p = pattern();
        let c = p.captures(b" gnmi:ethernet-1/1@srl1:57400 * 8").unwrap();
        assert_eq!(&c[3], b"ethernet-1/1");
        assert_eq!(&c[5], b"srl1");
        assert_eq!(&c[6], b"57400");
        let c = p.captures(b"gnmi:xe-0/0/0:1@[2001:db8::1]").unwrap();
        assert_eq!(&c[3], b"xe-0/0/0:1");
        assert_eq!(&c[4], b"[2001:db8::1]");
        assert!(p.captures(b"xgnmi:e1@r1").is_none());
        assert!(p.captures(b"2:public@router1").is_none());
    }
}
