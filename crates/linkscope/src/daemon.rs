// SPDX-License-Identifier: GPL-2.0-or-later
//! Process management: PID file, lock, privilege drop, daemonizing.
//!
//! Fixes for weaknesses in MRTG 2.17.10:
//! * `--fhs` directories are created and chowned without following
//!   symlinks (MRTG chowned `/var/lock/mrtg` by path while still root);
//! * the PID file is created with `O_EXCL|O_NOFOLLOW` and written through
//!   the descriptor, also after forking;
//! * dropping privileges clears supplementary groups and verifies both the
//!   user and the group (MRTG kept root's groups and only checked the uid);
//! * mutual exclusion uses `flock`, held for the life of the process (the
//!   MRTG daemon deleted its own lock when the parent exited, and relied on
//!   `/proc` to detect stale locks).

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

use anyhow::{Context, Result, bail};
use nix::unistd::{Gid, Uid};

/// Paths used with `--fhs`, as in MRTG.
pub const FHS_CONFCACHE: &str = "/var/lib/mrtg/mrtg.ok";
pub const FHS_PID: &str = "/run/mrtg/mrtg.pid";
pub const FHS_LOCK: &str = "/var/lock/mrtg/mrtg.lck";
pub const FHS_LOG: &str = "/var/log/mrtg/mrtg.log";

/// Create `dir` if needed and give it to `uid:gid`, refusing symlinks and
/// anything that is not a real directory.
pub fn ensure_owned_dir(dir: &Path, uid: Uid, gid: Gid) -> Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => bail!("{} is a symlink; refusing to use it", dir.display()),
        Ok(m) if !m.is_dir() => bail!("{} exists and is not a directory", dir.display()),
        Ok(_) => {}
        Err(_) => std::fs::DirBuilder::new().mode(0o755).create(dir).with_context(|| format!("creating {}", dir.display()))?,
    }
    let fd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir)
        .with_context(|| format!("opening {}", dir.display()))?;
    if fd.metadata()?.uid() != uid.as_raw() || fd.metadata()?.gid() != gid.as_raw() {
        nix::unistd::fchown(fd.as_raw_fd(), Some(uid), Some(gid)).with_context(|| format!("chown {}", dir.display()))?;
    }
    Ok(())
}

/// A PID file created exclusively; its contents are rewritten through the
/// open descriptor (for example after forking).
pub struct PidFile {
    file: File,
    path: std::path::PathBuf,
}

impl PidFile {
    /// Create the PID file. A stale file (its process is gone) is replaced;
    /// a live one is an error.
    pub fn create(path: &Path, owner: Option<(Uid, Gid)>) -> Result<PidFile> {
        for attempt in 0..2 {
            match crate::fsutil::create_new(path, 0o644) {
                Ok(file) => {
                    if let Some((u, g)) = owner {
                        nix::unistd::fchown(file.as_raw_fd(), Some(u), Some(g)).context("chown pid file")?;
                    }
                    let mut p = PidFile { file, path: path.to_path_buf() };
                    p.write_current()?;
                    return Ok(p);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                    let meta = std::fs::symlink_metadata(path)?;
                    if meta.file_type().is_symlink() {
                        bail!("{} is a symlink; refusing to use it", path.display());
                    }
                    let old = std::fs::read_to_string(path).unwrap_or_default();
                    if let Ok(pid) = old.trim().parse::<i32>() {
                        // EPERM means the process exists but is not ours: still alive.
                        let alive = match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                            Ok(()) => true,
                            Err(e) => e == nix::errno::Errno::EPERM,
                        };
                        if pid > 0 && alive {
                            bail!("LinkScope is already running with pid {pid} (pid file {})", path.display());
                        }
                    }
                    std::fs::remove_file(path).with_context(|| format!("removing stale {}", path.display()))?;
                }
                Err(e) => return Err(e).with_context(|| format!("creating pid file {}", path.display())),
            }
        }
        bail!("could not create pid file {}", path.display())
    }

    /// Write the current process id into the file.
    pub fn write_current(&mut self) -> Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        writeln!(self.file, "{}", std::process::id())?;
        self.file.sync_data()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// An exclusive `flock` held until drop.
pub struct Lock {
    _file: File,
}

/// Take the lock on `path` or fail if another instance holds it.
pub fn acquire_lock(path: &Path) -> Result<Lock> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .with_context(|| format!("opening lock file {}", path.display()))?;
    // SAFETY: flock is called on a valid, owned descriptor.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        bail!("another LinkScope instance is running with this configuration (lock {} is held)", path.display());
    }
    Ok(Lock { _file: file })
}

/// Resolve `--user` / `--group` names or numbers.
pub fn lookup_ids(user: Option<&str>, group: Option<&str>) -> Result<Option<(Uid, Gid)>> {
    let Some(user) = user else {
        if group.is_some() {
            bail!("--group needs --user");
        }
        return Ok(None);
    };
    let u = match user.parse::<u32>() {
        Ok(n) => nix::unistd::User::from_uid(Uid::from_raw(n))?,
        Err(_) => nix::unistd::User::from_name(user)?,
    }
    .with_context(|| format!("unknown user {user}"))?;
    let gid = match group {
        Some(g) => match g.parse::<u32>() {
            Ok(n) => Gid::from_raw(n),
            Err(_) => nix::unistd::Group::from_name(g)?.with_context(|| format!("unknown group {g}"))?.gid,
        },
        None => u.gid,
    };
    Ok(Some((u.uid, gid)))
}

/// Drop to `uid:gid`, clearing supplementary groups, and verify.
pub fn drop_privileges(uid: Uid, gid: Gid) -> Result<()> {
    if Uid::effective().is_root() {
        nix::unistd::setgroups(&[gid]).context("setgroups")?;
    }
    nix::unistd::setgid(gid).context("setgid")?;
    nix::unistd::setuid(uid).context("setuid")?;
    let ok = Uid::current() == uid && Uid::effective() == uid && Gid::current() == gid && Gid::effective() == gid;
    let groups_ok = nix::unistd::getgroups().map(|g| g.iter().all(|x| *x == gid)).unwrap_or(false);
    if !ok || !groups_ok {
        bail!("failed to drop privileges to uid {uid} gid {gid}");
    }
    if uid.is_root() {
        tracing::warn!("running as root; use --user to run with less privilege");
    }
    Ok(())
}

/// Detach from the terminal (must be called before any threads start).
pub fn daemonize() -> Result<()> {
    use nix::unistd::{ForkResult, fork, setsid};
    // SAFETY: called from main before the async runtime or any thread exists.
    match unsafe { fork() }.context("fork")? {
        ForkResult::Parent { .. } => {
            // Exit immediately without running destructors (the child owns
            // the lock and PID file now). This is the bug MRTG had: its
            // parent's END block deleted the child's lock.
            // SAFETY: _exit is always safe to call.
            unsafe { libc::_exit(0) };
        }
        ForkResult::Child => {}
    }
    setsid().context("setsid")?;
    let null = OpenOptions::new().read(true).write(true).open("/dev/null")?;
    for fd in 0..=2 {
        // SAFETY: dup2 onto the standard descriptors with a valid source fd.
        unsafe { libc::dup2(null.as_raw_fd(), fd) };
    }
    // Unlike many daemons (and like MRTG) the working directory is kept;
    // all paths are absolute by now anyway.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_file_refuses_symlink_and_replaces_stale() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.pid");
        std::fs::write(&p, "999999999\n").unwrap();
        let pf = PidFile::create(&p, None).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), std::process::id().to_string());
        assert!(PidFile::create(&p, None).is_err(), "live pid must block");
        drop(pf);
        assert!(!p.exists());
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "data").unwrap();
        std::os::unix::fs::symlink(&victim, &p).unwrap();
        assert!(PidFile::create(&p, None).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "data");
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.lck");
        let l = acquire_lock(&p).unwrap();
        assert!(acquire_lock(&p).is_err());
        drop(l);
        assert!(acquire_lock(&p).is_ok());
    }

    #[test]
    fn owned_dir_refuses_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let (u, g) = (Uid::current(), Gid::current());
        assert!(ensure_owned_dir(&link, u, g).is_err());
        assert!(ensure_owned_dir(&dir.path().join("new"), u, g).is_ok());
    }
}
