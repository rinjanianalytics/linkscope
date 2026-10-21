// SPDX-License-Identifier: GPL-2.0-or-later
//! Ownership and permission checks for configuration files.
//!
//! In MRTG the configuration is executable (Target expressions, backtick
//! targets and ConversionCode run as the MRTG user). LinkScope no longer
//! evaluates Perl, but a config can still run programs (EXEC targets,
//! threshold programs), so it is still a trust boundary. A file is trusted
//! when it is owned by root or by the running user, it is not writable by
//! group or others, and no directory on the way to it lets another account
//! rename entries (each belongs to root or the running user and is writable
//! by no one else, a sticky directory or the user's private group aside).

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
pub fn check_file_trust(
    path: &Path,
    file: &std::fs::File,
    policy: TrustPolicy,
) -> Result<(), ConfigError> {
    if policy == TrustPolicy::Off {
        return Ok(());
    }
    let Some(problem) = trust_problem(path, file) else {
        return Ok(());
    };
    match policy {
        TrustPolicy::Enforce => Err(ConfigError::Untrusted {
            path: path.to_path_buf(),
            reason: problem,
        }),
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
    // The directory that really holds the file that was opened (not the name resolved again).
    let real = opened_path(file)
        .or_else(|| std::fs::canonicalize(path).ok())
        .unwrap_or_else(|| path.to_path_buf());
    // SAFETY: getegid takes no arguments, has no preconditions and cannot fail.
    let egid = unsafe { libc::getegid() };
    // Whoever can rename an entry in any directory on the way can put another file in its place.
    for dir in real
        .ancestors()
        .skip(1)
        .filter(|d| !d.as_os_str().is_empty())
    {
        let Ok(d) = std::fs::metadata(dir) else {
            continue;
        };
        if let Some(why) = dir_problem(d.uid(), d.gid(), d.mode(), euid, egid) {
            return Some(format!(
                "{} {why}, so others could replace the file",
                dir.display()
            ));
        }
    }
    None
}

/// The path of an opened file, as the kernel knows it.
fn opened_path(file: &std::fs::File) -> Option<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .ok()
        .filter(|p| p.is_absolute())
}

/// Why a directory on the way to the config is not safe, if it is not: it must belong to root or
/// the running account, and not be writable by others, nor by a group other than root's or the
/// running account's own (a user private group), unless it is sticky.
fn dir_problem(uid: u32, gid: u32, mode: u32, euid: u32, egid: u32) -> Option<String> {
    if uid != 0 && uid != euid {
        return Some(format!(
            "belongs to uid {uid} (expected root or uid {euid})"
        ));
    }
    if mode & S_ISVTX != 0 {
        return None;
    }
    if mode & S_IWOTH != 0 {
        return Some("is writable by others without the sticky bit".into());
    }
    if mode & S_IWGRP != 0 && gid != 0 && gid != egid {
        return Some(format!("is writable by group {gid} without the sticky bit"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_on_the_way_belong_to_root_or_the_user_and_no_wider_group() {
        let (me, mine) = (1000, 1000);
        // Root's, the user's own, and the user's private group: fine.
        assert_eq!(dir_problem(0, 0, 0o755, me, mine), None);
        assert_eq!(dir_problem(me, mine, 0o775, me, mine), None);
        assert_eq!(dir_problem(0, 0, 0o1777, me, mine), None);
        assert_eq!(dir_problem(0, 0, 0o775, me, mine), None);
        // Another account's directory, a world-writable one, a group of other people: refused.
        assert!(dir_problem(1001, 1001, 0o755, me, mine).is_some());
        assert!(dir_problem(me, mine, 0o777, me, mine).is_some());
        assert!(dir_problem(0, 50, 0o775, me, mine).is_some());
        assert!(dir_problem(me, 50, 0o2775, me, mine).is_some());
        // Sticky: entries can be renamed by their owners only.
        assert_eq!(dir_problem(0, 50, 0o1775, me, mine), None);
    }
}
