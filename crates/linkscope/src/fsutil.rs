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
