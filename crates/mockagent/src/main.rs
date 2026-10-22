// SPDX-License-Identifier: GPL-2.0-or-later
//! `linkscope-mockagent gnmi`: a pretend gNMI router, to try LinkScope's gNMI targets without
//! one. Its counters grow at the given rates. Plain gRPC unless `--cert` and `--key` are given.

use clap::{Parser, Subcommand, ValueEnum};
use linkscope_mockagent::gnmi::{Device, Interface, Model, serve};

#[derive(Parser)]
#[command(version, about = "Test devices for LinkScope")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModelArg {
    Openconfig,
    Srl,
}

#[derive(Subcommand)]
enum Cmd {
    /// Answer gNMI like a router.
    Gnmi {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:9339")]
        listen: String,
        #[arg(long, value_enum, default_value = "openconfig")]
        model: ModelArg,
        /// Interface names, comma separated.
        #[arg(long, default_value = "ethernet-1/1,ethernet-1/2")]
        interfaces: String,
        /// Bytes per second each interface's in counter grows by (out grows by a tenth).
        #[arg(long, default_value_t = 250_000)]
        rate: u64,
        #[arg(long, default_value = "mock")]
        hostname: String,
        /// `user:password` every request must carry.
        #[arg(long)]
        login: Option<String>,
        /// Server certificate and key (PEM) for TLS.
        #[arg(long, requires = "key")]
        cert: Option<std::path::PathBuf>,
        #[arg(long, requires = "cert")]
        key: Option<std::path::PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cmd::Gnmi {
        listen,
        model,
        interfaces,
        rate,
        hostname,
        login,
        cert,
        key,
    } = Cli::parse().cmd;
    let model = match model {
        ModelArg::Openconfig => Model::OpenConfig,
        ModelArg::Srl => Model::Srl,
    };
    let ifs = interfaces
        .split(',')
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(i, n)| {
            let mut f = Interface::new(n, i as u32 + 1, 0, 0);
            f.in_rate = rate;
            f.out_rate = rate / 10;
            f
        })
        .collect();
    let mut dev = Device::new(model, &hostname, ifs);
    if let Some(l) = login {
        let (u, p) = l.split_once(':').ok_or("--login is user:password")?;
        dev.login = Some((u.into(), p.into()));
    }
    let identity = match (cert, key) {
        (Some(c), Some(k)) => Some((std::fs::read(c)?, std::fs::read(k)?)),
        _ => None,
    };
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    eprintln!("gNMI mock on {listen}");
    serve(dev, listener, identity).await?;
    Ok(())
}
