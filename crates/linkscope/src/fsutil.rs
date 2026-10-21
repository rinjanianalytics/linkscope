// SPDX-License-Identifier: GPL-2.0-or-later
//! Safe file writing helpers.
//!
//! MRTG opened its output files with plain `open(">...")`, which follows
//! symlinks. When the work directory is writable by another account (often
//! the web server), that allows overwriting arbitrary files as the MRTG
//! user. Everything LinkScope writes goes through [`write_atomic`]: a fresh
//! temporary file created with `O_CREAT|O_EXCL|O_NOFOLLOW`, fsynced, then
//! renamed over the destination (rename replaces a symlink, it does not
//! follow it).

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Create a new file that must not exist yet and must not be a symlink.
pub fn create_new(path: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

/// Mode of the state files LinkScope keeps beside its config (`.ok`, `.alerts`): owner only. The
/// `.ok` cache keys each device by its SNMP community, as MRTG's does, so it is as secret as the
/// config (2026-10-21: it was written 0644).
pub const STATE_MODE: u32 = 0o600;

/// Largest state file read (the `.ok` cache keeps at most 100,000 rows per method).
pub const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;

/// Read a state file as the logs are read: no symlink followed, no blocking on a FIFO, regular
/// files only, at most `max` bytes. `Ok(None)` when it does not exist. Also says whether others
/// than the owner may read it (so it is written again with [`STATE_MODE`]).
pub fn read_state(path: &Path, max: u64) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let f = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf)?;
    Ok(Some((buf, meta.permissions().mode() & 0o077 != 0)))
}

fn temp_name(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp_{}_{n}", std::process::id()))
}

/// Atomically replace `path` with `data`.
pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let tmp = temp_name(path);
    let res = (|| {
        let mut f = create_new(&tmp, mode)?;
        f.write_all(data)?;
        f.sync_data()?;
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_write_through_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let link = dir.path().join("out.ok");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        write_atomic(&link, b"new", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "new");
        assert!(!std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }
}
