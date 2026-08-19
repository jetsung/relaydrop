//! RelayDrop: a minimal encrypted file relay over a WebSocket/TLS
//! relay transport, written in Rust.
//!
//! Subcommands:
//!   relay    - run the relay server (plain ws)
//!   send     - send one or more files/folders
//!   receive  - receive a file or folder

mod client;
mod crypto;
mod framed;
mod relay;

use std::time::Duration;

use clap::{Parser, Subcommand};
use rustls::crypto::ring;

#[derive(Parser)]
#[command(name = "relaydrop", version, about = "RelayDrop: a minimal encrypted file relay over WebSocket/TLS, written in Rust")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the relay server. Listen on a local port; TLS may be terminated by a
    /// reverse proxy in front of it.
    Relay {
        /// Listen address, e.g. 127.0.0.1:9090
        #[arg(long, default_value = "127.0.0.1:9090", env = "RELAYDROP_LISTEN")]
        listen: String,
        /// Relay password (shared with clients via --password)
        #[arg(long, default_value = "", env = "RELAYDROP_PASSWORD")]
        password: String,
        /// Room TTL in seconds before an unpaired room is evicted
        #[arg(long, default_value_t = 300, env = "RELAYDROP_TTL")]
        ttl: u64,
    },
    /// Send one or more files/folders (or a mix) to the peer. Omit --code to
    /// have it generated randomly; a copy-paste receiver command is then printed.
    Send {
        /// Relay URL: tcp://host:port, ws://host:port/path or wss://host/path
        #[arg(long, env = "RELAYDROP_RELAY")]
        relay: String,
        /// Shared secret / room name (random if omitted)
        #[arg(long, env = "RELAYDROP_CODE")]
        code: Option<String>,
        /// Relay password (fixed per relay; embedded in the printed command)
        #[arg(long, default_value = "", env = "RELAYDROP_PASSWORD")]
        password: String,
        /// Path to a file or folder to send (may be given multiple times; alias
        /// of the positional paths)
        #[arg(long)]
        file: Vec<String>,
        /// Files or directories to send (positional; may be given multiple times)
        #[arg(env = "RELAYDROP_PATH")]
        paths: Vec<String>,
    },
    /// Receive a file or folder from the peer with the same --code.
    Receive {
        /// Relay URL
        #[arg(long, env = "RELAYDROP_RELAY")]
        relay: String,
        /// Shared secret / room name (must match the sender)
        #[arg(long, env = "RELAYDROP_CODE")]
        code: String,
        /// Relay password
        #[arg(long, default_value = "", env = "RELAYDROP_PASSWORD")]
        password: String,
        /// Output directory
        #[arg(long, default_value = ".", env = "RELAYDROP_OUT")]
        out: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls 0.23 no longer auto-installs a CryptoProvider from crate features;
    // install the `ring` provider explicitly before any TLS connection is made.
    ring::default_provider()
        .install_default()
        .expect("failed to install rustls ring crypto provider");

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Relay {
            listen,
            password,
            ttl,
        } => {
            let r = relay::Relay::new(&password, Duration::from_secs(ttl));
            r.run(&listen).await
        }
        Cmd::Send {
            relay: url,
            code,
            password,
            file,
            paths,
        } => {
            let all: Vec<String> = {
                let mut v = paths;
                v.extend(file);
                v
            };
            if all.is_empty() {
                anyhow::bail!("no file or path given (use --file or positional paths)");
            }
            client::send(&url, &password, code.as_deref(), &all).await
        }
        Cmd::Receive {
            relay: url,
            code,
            password,
            out,
        } => client::receive(&url, &password, &code, &out).await,
    }
}
