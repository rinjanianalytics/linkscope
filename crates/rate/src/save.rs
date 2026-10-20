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

#[cfg(target_os = "linux")]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
const O_NONBLOCK: i32 = 0x0004;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly")))]
const O_NONBLOCK: i32 = 0;

/// `O_NOFOLLOW` for the targets we know; elsewhere `O_EXCL` alone already
/// refuses to open an existing path, symlink or not.
#[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64", target_arch = "s390x", target_arch = "riscv64", target_arch = "loongarch64")))]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm", target_arch = "powerpc", target_arch = "powerpc64")))]
const O_NOFOLLOW: i32 = 0o100000;
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
const O_NOFOLLOW: i32 = 0x0100;
#[cfg(not(any(
    all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64", target_arch = "s390x", target_arch = "riscv64", target_arch = "loongarch64", target_arch = "aarch64", target_arch = "arm", target_arch = "powerpc", target_arch = "powerpc64")),
    target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"
)))]
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
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "base name too long"));
    }
    // Unpredictable temp name, so another account cannot pre-create it.
    let nonce = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        u64::from(t) ^ N.fetch_add(0x9e37_79b9, Ordering::Relaxed).rotate_left(17)
    };
    let tmp = dir.join(format!("{base}.tmp_{}_{nonce:x}", std::process::id()));
    let log = dir.join(format!("{base}.log"));
    let old = dir.join(format!("{base}.old"));
    let mut f = create_exclusive(&tmp)?;
    let written = f.write_all(content).and_then(|()| if sync { f.sync_all() } else { Ok(()) });
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
    if sync {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}
