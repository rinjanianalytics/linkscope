// SPDX-License-Identifier: GPL-2.0-or-later
//! `linkscope`: drop-in replacement for the `mrtg` command.
//!
//! Usage matches MRTG: `linkscope [options] /etc/mrtg/mrtg.cfg`. Run without
//! `--daemon` (and without `RunAsDaemon: yes`) it polls once and exits, as
//! from cron. `linkscope cfgmaker ...` (or a symlink named `cfgmaker`) runs
//! the config generator.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use clap::Parser;
use linkscope_config::{CheckOptions, Config, ReadOptions, TrustPolicy};
use linkscope::cycle::{Engine, EngineOptions, default_cache_path};
use linkscope::daemon;
use linkscope::oids::OidResolver;
use linkscope_web::{LiveState, SharedState};

#[derive(Parser, Debug)]
#[command(name = "linkscope", version, about = "LinkScope: link utilization monitoring, compatible with MRTG")]
struct Args {
    /// Configuration file (MRTG syntax).
    #[arg(default_value = "/etc/mrtg/mrtg.cfg")]
    config: PathBuf,
    /// Run as this user after startup (name or uid).
    #[arg(long)]
    user: Option<String>,
    /// Run as this group (name or gid).
    #[arg(long)]
    group: Option<String>,
    /// Lock file (default: <config>_l).
    #[arg(long = "lock-file")]
    lock_file: Option<PathBuf>,
    /// Interface cache file (default: <config> with extension .ok).
    #[arg(long = "confcache-file")]
    confcache_file: Option<PathBuf>,
    /// Log to this file instead of stderr ("eventlog" is accepted and means stderr).
    #[arg(long)]
    logging: Option<String>,
    /// Check the configuration and exit.
    #[arg(long)]
    check: bool,
    /// Use FHS paths: /var/lib/mrtg, /run/mrtg, /var/lock/mrtg, /var/log/mrtg.
    #[arg(long)]
    fhs: bool,
    /// Keep running and poll every Interval.
    #[arg(long)]
    daemon: bool,
    /// PID file in daemon mode (default: <config> with extension .pid).
    #[arg(long = "pid-file")]
    pid_file: Option<PathBuf>,
    /// Debug categories (MRTG compatible); any value raises the log level.
    #[arg(long)]
    debug: Option<String>,
    /// Accepted for compatibility (LinkScope never writes HTML pages).
    #[arg(long = "log-only")]
    log_only: bool,
    /// Serve the web UI on this address (overrides WebListen:).
    #[arg(long)]
    listen: Option<String>,
    /// Maximum number of agents polled at the same time.
    #[arg(long, default_value_t = 256)]
    concurrency: usize,
    /// With --daemon: stay in the foreground (for systemd and other supervisors).
    #[arg(long)]
    foreground: bool,
    /// Load configuration files even if other accounts can modify them.
    #[arg(long = "allow-insecure-config")]
    allow_insecure_config: bool,
}

fn init_logging(target: Option<&str>, debug: bool) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LINKSCOPE_LOG").unwrap_or_else(|_| EnvFilter::new(if debug { "debug" } else { "info" }));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false);
    match target {
        Some(f) if f != "eventlog" => {
            use std::os::unix::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .mode(0o640)
                .custom_flags(libc::O_NOFOLLOW)
                .open(f)
                .with_context(|| format!("opening log file {f}"))?;
            builder.with_ansi(false).with_writer(std::sync::Mutex::new(file)).init();
        }
        _ => builder.with_writer(std::io::stderr).init(),
    }
    Ok(())
}

fn load_config(path: &Path, insecure: bool) -> Result<Config> {
    let read = ReadOptions { trust: if insecure { TrustPolicy::Warn } else { TrustPolicy::Enforce } };
    // Directories are created later, as the account that writes the logs.
    Ok(linkscope_config::load(path, &read, &CheckOptions { no_create: true })?)
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn mtimes(cfg: &Config) -> Vec<Option<SystemTime>> {
    cfg.files.iter().map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).collect()
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let prog = Path::new(argv.first().map(String::as_str).unwrap_or("")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if prog == "cfgmaker" || argv.get(1).map(String::as_str) == Some("cfgmaker") {
        let rest: Vec<String> = if prog == "cfgmaker" { argv.clone() } else { argv[1..].to_vec() };
        return linkscope::cfgmaker::main(rest);
    }
    match run(Args::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run(mut args: Args) -> Result<ExitCode> {
    // Absolute paths throughout, so nothing depends on the working directory.
    args.config = absolute(&args.config);
    if args.fhs {
        args.confcache_file.get_or_insert_with(|| daemon::FHS_CONFCACHE.into());
        args.pid_file.get_or_insert_with(|| daemon::FHS_PID.into());
        args.lock_file.get_or_insert_with(|| daemon::FHS_LOCK.into());
        args.logging.get_or_insert_with(|| daemon::FHS_LOG.into());
    }
    for p in [&mut args.confcache_file, &mut args.pid_file, &mut args.lock_file] {
        if let Some(x) = p.as_mut() {
            *x = absolute(x);
        }
    }
    if let Some(l) = args.logging.as_mut().filter(|l| *l != "eventlog") {
        *l = absolute(Path::new(l.as_str())).display().to_string();
    }

    // Read and check the configuration first. --check needs nothing else
    // (and does not take the lock, so it works while the daemon runs).
    let cfg = load_config(&args.config, args.allow_insecure_config)?;
    if args.check {
        for w in &cfg.warnings {
            eprintln!("WARNING: {w}");
        }
        println!("{}: {} targets, {} data sources: OK", args.config.display(), cfg.targets.len(), cfg.sources.len());
        return Ok(ExitCode::SUCCESS);
    }

    // Privileged steps: FHS directories, pid file, lock, web socket.
    let ids = daemon::lookup_ids(args.user.as_deref(), args.group.as_deref())?;
    if args.fhs && nix::unistd::Uid::effective().is_root() {
        let (u, g) = ids.unwrap_or((nix::unistd::Uid::current(), nix::unistd::Gid::current()));
        for f in [daemon::FHS_CONFCACHE, daemon::FHS_PID, daemon::FHS_LOCK, daemon::FHS_LOG] {
            if let Some(dir) = Path::new(f).parent() {
                daemon::ensure_owned_dir(dir, u, g)?;
            }
        }
    }
    let daemon_mode = args.daemon || cfg.global.run_as_daemon;
    let pid_path = args.pid_file.clone().unwrap_or_else(|| args.config.with_extension("pid"));
    let mut pid = if daemon_mode { Some(daemon::PidFile::create(&pid_path, ids)?) } else { None };
    let lock_path = args.lock_file.clone().unwrap_or_else(|| PathBuf::from(format!("{}_l", args.config.display())));
    let _lock = daemon::acquire_lock(&lock_path)?;
    let listen = args.listen.clone().or_else(|| cfg.global.web_listen.clone());
    let listener = match &listen {
        Some(addr) => {
            let a: std::net::SocketAddr = addr.parse().with_context(|| format!("bad listen address {addr}"))?;
            let l = std::net::TcpListener::bind(a).with_context(|| format!("cannot listen on {a}"))?;
            l.set_nonblocking(true)?;
            Some(l)
        }
        None => None,
    };

    if let Some((u, g)) = ids {
        daemon::drop_privileges(u, g)?;
    }
    init_logging(args.logging.as_deref(), args.debug.is_some())?;
    for w in &cfg.warnings {
        tracing::warn!("{w}");
    }
    linkscope_config::create_dirs(&cfg)?;

    // Like MRTG: detach unless NoDetach (or --foreground for supervisors).
    if daemon_mode && !cfg.global.no_detach && !args.foreground && std::env::var_os("LINKSCOPE_FOREGROUND").is_none() {
        if args.logging.is_none() {
            eprintln!("linkscope: detaching; messages go to /dev/null (use --logging FILE, or --foreground under systemd)");
        }
        daemon::daemonize()?;
    }
    if let Some(p) = pid.as_mut() {
        p.write_current()?;
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async_main(args, cfg, daemon_mode, listener))
}

async fn async_main(args: Args, cfg: Config, daemon_mode: bool, listener: Option<std::net::TcpListener>) -> Result<ExitCode> {
    let mut oids = OidResolver::default();
    for m in &cfg.global.load_mibs {
        match oids.load_mib(Path::new(m)) {
            Ok(n) => tracing::debug!("loaded {n} names from {m}"),
            Err(e) => tracing::warn!("LoadMIBs {m}: {e}"),
        }
    }
    let oids = Arc::new(oids);
    let state: SharedState = Arc::new(RwLock::new(LiveState { started: linkscope::poll::now(), ..Default::default() }));
    let cache_path = args.confcache_file.clone().unwrap_or_else(|| default_cache_path(&args.config));
    let opts = EngineOptions { concurrency: args.concurrency, ..Default::default() };
    let mut cfg = Arc::new(cfg);
    let mut engine = Engine::new(cfg.clone(), oids.clone(), state.clone(), cache_path.clone(), opts.clone());

    if let Some(l) = listener {
        let st = state.clone();
        let l = tokio::net::TcpListener::from_std(l)?;
        tokio::spawn(async move {
            if let Err(e) = linkscope_web::serve_listener(l, st).await {
                tracing::error!("web UI stopped: {e:#}");
            }
        });
    }

    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    let mut hup = signal(SignalKind::hangup())?;

    if !daemon_mode {
        let info = tokio::select! {
            i = engine.cycle() => i,
            _ = term.recv() => return Ok(ExitCode::from(1)),
            _ = int.recv() => return Ok(ExitCode::from(1)),
        };
        engine.finish().await;
        tracing::info!("polled {} targets in {} ms ({} ok, {} failed)", info.targets, info.duration_ms, info.ok, info.failed);
        return Ok(match (info.ok, info.failed) {
            (_, 0) => ExitCode::SUCCESS,
            (0, _) => ExitCode::from(92),
            _ => ExitCode::from(91),
        });
    }

    let mut stamps = mtimes(&cfg);
    tracing::info!("LinkScope daemon started: {} targets every {} s", cfg.targets.len(), cfg.global.interval_secs);
    'outer: loop {
        let start = tokio::time::Instant::now();
        // SIGTERM cancels a running cycle; logs are replaced atomically.
        let info = tokio::select! {
            i = engine.cycle() => i,
            _ = term.recv() => break,
            _ = int.recv() => break,
        };
        tracing::info!("cycle: {} targets in {} ms ({} failed, {} log write failures)", info.targets, info.duration_ms, info.failed, info.write_failures);
        let interval = Duration::from_secs(cfg.global.interval_secs.max(1));
        let deadline = start + interval;
        if deadline <= tokio::time::Instant::now() {
            tracing::warn!("polling took longer than Interval ({} s); consider raising --concurrency", interval.as_secs());
        }
        // Wait out the interval; a SIGHUP reloads but keeps the schedule.
        loop {
            let mut reload = false;
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {}
                _ = term.recv() => break 'outer,
                _ = int.recv() => break 'outer,
                _ = hup.recv() => reload = true,
            }
            if reload || mtimes(&cfg) != stamps {
                // A reader bug must not take the running daemon down: the old configuration stays.
                let loaded = std::panic::catch_unwind(|| load_config(&args.config, args.allow_insecure_config))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("the configuration reader failed unexpectedly")));
                match loaded.and_then(|c| {
                    linkscope_config::create_dirs(&c)?;
                    Ok(c)
                }) {
                    Ok(new) => {
                        tracing::info!("configuration reloaded: {} targets", new.targets.len());
                        for w in &new.warnings {
                            tracing::warn!("{w}");
                        }
                        cfg = Arc::new(new);
                        stamps = mtimes(&cfg);
                        engine.finish().await;
                        engine = Engine::new(cfg.clone(), oids.clone(), state.clone(), cache_path.clone(), opts.clone());
                    }
                    Err(e) => {
                        tracing::error!("keeping the previous configuration; reload failed: {e:#}");
                        stamps = mtimes(&cfg);
                    }
                }
            }
            if !reload {
                break;
            }
        }
    }
    tracing::info!("LinkScope stopping");
    Ok(ExitCode::SUCCESS)
}
