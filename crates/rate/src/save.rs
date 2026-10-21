// SPDX-License-Identifier: GPL-2.0-or-later
//! File access: reading logs like `fopen`/`fread`, and the atomic save.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

/// Reads a whole file. `None` means `fopen` would fail; a file that opens
/// but cannot be read (for example a directory) yields the bytes read so
/// far, which is what rateup's `fscanf`/`fgets` would see.
///
/// Hardening: symlinks are not followed, the open does not block (so a FIFO
/// cannot hang the caller), only regular files are read, and at most
/// [`MAX_LOG_BYTES`] are read (a normal log is about 50 KB).
pub(crate) fn read_file(path: &Path) -> Option<Vec<u8>> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(O_NOFOLLOW | O_NONBLOCK);
    }
    let f = opts.open(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    let _ = f.take(MAX_LOG_BYTES).read_to_end(&mut buf);
    Some(buf)
}

/// Largest log file that will be read.
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// `O_NONBLOCK` and `O_NOFOLLOW` as the platform defines them. They were hard-coded per target, and
/// an architecture missing from the list (mips, sparc, ...) silently got 0: no hardening at all.
#[cfg(unix)]
const O_NONBLOCK: i32 = libc::O_NONBLOCK;
#[cfg(not(unix))]
const O_NONBLOCK: i32 = 0;
#[cfg(unix)]
const O_NOFOLLOW: i32 = libc::O_NOFOLLOW;
#[cfg(not(unix))]
const O_NOFOLLOW: i32 = 0;

fn create_exclusive(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o644).custom_flags(O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = O_NOFOLLOW;
    opts.open(path)
}

/// Writes `<base>.tmp_<pid>`, fsyncs it, renames `<base>.log` to
/// `<base>.old` (ignoring a missing log, as rateup only warns) and the
/// temporary file to `<base>.log`, then fsyncs the directory.
pub(crate) fn save_atomic(dir: &Path, base: &str, content: &[u8], sync: bool) -> io::Result<()> {
    if base.len() > crate::MAX_BASE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "base name too long",
        ));
    }
    // Unpredictable temp name, so another account cannot pre-create it.
    let nonce = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        u64::from(t) ^ N.fetch_add(0x9e37_79b9, Ordering::Relaxed).rotate_left(17)
    };
    let tmp = dir.join(format!("{base}.tmp_{}_{nonce:x}", std::process::id()));
    let log = dir.join(format!("{base}.log"));
    let old = dir.join(format!("{base}.old"));
    let mut f = create_exclusive(&tmp)?;
    let written = f
        .write_all(content)
        .and_then(|()| if sync { f.sync_all() } else { Ok(()) });
    drop(f);
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    match fs::rename(&log, &old) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = fs::rename(&tmp, &log) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Make the renames durable; not all platforms can open a directory.
    if sync && let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}
