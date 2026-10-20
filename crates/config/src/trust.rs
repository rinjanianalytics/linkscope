// SPDX-License-Identifier: GPL-2.0-or-later
//! Ownership and permission checks for configuration files.
//!
//! In MRTG the configuration is executable (Target expressions, backtick
//! targets and ConversionCode run as the MRTG user). LinkScope no longer
//! evaluates Perl, but a config can still run programs (EXEC targets,
//! threshold programs), so it is still a trust boundary. A file is trusted
//! when it is owned by root or by the running user and neither it nor its
//! directory is writable by other accounts (a sticky directory is fine).

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::ConfigError;

/// What to do with a configuration file that other accounts could modify.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrustPolicy {
    /// Refuse to load it (default).
    #[default]
    Enforce,
    /// Load it but print a warning to stderr.
    Warn,
    /// Do not check (tests, or `--allow-insecure-config`).
    Off,
}

const S_IWGRP: u32 = 0o020;
const S_IWOTH: u32 = 0o002;
const S_ISVTX: u32 = 0o1000;

/// Check `path` against `policy`.
/// Check an already opened config file (`fstat` on the descriptor, so the
/// file checked is the file read) and the directory that really contains it.
pub fn check_file_trust(path: &Path, file: &std::fs::File, policy: TrustPolicy) -> Result<(), ConfigError> {
    if policy == TrustPolicy::Off {
        return Ok(());
    }
    let Some(problem) = trust_problem(path, file) else { return Ok(()) };
    match policy {
        TrustPolicy::Enforce => Err(ConfigError::Untrusted { path: path.to_path_buf(), reason: problem }),
        _ => {
            eprintln!("WARNING: {}: {problem}", path.display());
            Ok(())
        }
    }
}

fn trust_problem(path: &Path, file: &std::fs::File) -> Option<String> {
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(e) => return Some(format!("cannot inspect: {e}")),
    };
    // SAFETY: geteuid takes no arguments, has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != 0 && meta.uid() != euid {
        return Some(format!(
            "owned by uid {} (expected root or uid {euid}); other accounts could change what LinkScope runs",
            meta.uid()
        ));
    }
    if meta.mode() & (S_IWGRP | S_IWOTH) != 0 {
        return Some(format!(
            "writable by group or others (mode {:o}); run chmod go-w on it",
            meta.mode() & 0o7777
        ));
    }
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = real.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Ok(d) = std::fs::metadata(dir) {
            if d.mode() & S_IWOTH != 0 && d.mode() & S_ISVTX == 0 {
                return Some(format!(
                    "its directory {} is world-writable without the sticky bit, so others could replace the file",
                    dir.display()
                ));
            }
        }
    }
    None
}
