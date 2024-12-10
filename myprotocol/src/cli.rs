use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(name = "server")]
pub struct Opt {
    /// file to log TLS keys to for debugging
    #[clap(long = "keylog")]
    pub keylog: bool,
    /// TLS private key in PEM format
    #[clap(short = 'k', long = "key", requires = "cert")]
    pub key: Option<PathBuf>,
    /// TLS certificate in PEM format
    #[clap(short = 'c', long = "cert", requires = "key")]
    pub cert: Option<PathBuf>,
    /// Enable stateless retries
    #[clap(long = "stateless-retry")]
    pub stateless_retry: bool,
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:4433")]
    pub listen: SocketAddr,
    /// Client address to block
    #[clap(long = "block")]
    pub block: Option<SocketAddr>,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit")]
    pub connection_limit: Option<usize>,
}