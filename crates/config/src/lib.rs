// SPDX-License-Identifier: GPL-2.0-or-later
//! MRTG configuration for LinkScope.
//!
//! * [`read_config`] reads `mrtg.cfg` with MRTG's exact `readcfg` grammar.
//! * [`check`] applies `cfgcheck`'s defaults and validation and returns a
//!   typed [`Config`].
//! * [`target`] and [`expr`] replace MRTG's string `eval` of `Target[]`
//!   values with a parser that produces data, never code.
//!
//! Load a config in one call with [`load`].

pub mod check;
pub mod expr;
pub mod gnmi;
pub mod perlhash;
pub mod raw;
pub mod rules;
pub mod target;
pub mod trust;

use std::path::{Path, PathBuf};

pub use check::{CheckOptions, Config, Global, Options, Target, Thresholds, check, create_dirs};
pub use expr::{Dir, EvalError, Expr};
pub use gnmi::{GnmiModel, GnmiOptions, GnmiSource, TlsMode};
pub use perlhash::SnmpOptions;
pub use raw::{RawConfig, ReadOptions, read_config};
pub use target::{ExecSource, IfSel, SnmpConn, SnmpSource, SnmpVersion, Source};
pub use trust::TrustPolicy;

/// Errors raised while reading or checking a configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}:{line}: {message}")]
    At {
        file: PathBuf,
        line: usize,
        message: String,
    },
    #[error("refusing to use {path}: {reason} (use --allow-insecure-config to override)")]
    Untrusted { path: PathBuf, reason: String },
    #[error("configuration errors:\n  {}", .0.join("\n  "))]
    Invalid(Vec<String>),
}

impl ConfigError {
    pub(crate) fn at(file: &Path, line: usize, message: impl Into<String>) -> Self {
        ConfigError::At {
            file: file.to_path_buf(),
            line,
            message: message.into(),
        }
    }
}

/// Read and check a configuration file in one step.
pub fn load(path: &Path, read: &ReadOptions, opts: &CheckOptions) -> Result<Config, ConfigError> {
    let raw = read_config(path, read)?;
    check(raw, opts)
}
