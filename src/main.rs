mod filter;
mod proxy;
mod service;
mod stream;
mod tls;

use clap::Parser;
use humantime::Duration;
use std::net::{IpAddr, SocketAddr};
use tokio;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

use crate::service::Service;

#[derive(Debug, Clone, Parser)]
#[command(version = "0.1", about = "Service configuration", long_about = None)]
pub struct Args {
    #[arg(long)]
    pub service_name: String,

    #[arg(long, value_name = "IP", default_value = "0.0.0.0")]
    pub client_ip: IpAddr,

    #[arg(long, value_name = "PORT", required = true)]
    pub client_port: u16,

    #[arg(long, value_name = "IP", default_value = "127.0.0.1")]
    pub server_ip: IpAddr,

    #[arg(long, value_name = "PORT", required = true)]
    pub server_port: u16,

    #[arg(long, value_name = "DURATION", default_value = "1s")]
    pub client_timeout: Duration,

    #[arg(long, value_name = "DURATION", default_value = "1s")]
    pub server_timeout: Duration,

    #[arg(long = "tls", default_value = "false")]
    pub tls_enabled: bool,

    #[arg(long, value_name = "PATH", required_if_eq("tls_enabled", "true"))]
    pub tls_cert_file: Option<String>,

    #[arg(long, value_name = "PATH", required_if_eq("tls_enabled", "true"))]
    pub tls_key_file: Option<String>,

    #[arg(long, value_name = "PATH")]
    pub tls_ca_file: Option<String>,

    #[arg(long, value_name = "PATH")]
    pub python_script: Option<String>,
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args = Args::parse();
    debug!("{:?}", args);

    let service = Service {
        name: args.service_name,
        client_addr: SocketAddr::new(args.client_ip, args.client_port),
        server_addr: SocketAddr::new(args.server_ip, args.server_port),
        client_timeout: args.client_timeout,
        server_timeout: args.server_timeout,
        tls_cert_file: args.tls_cert_file,
        tls_key_file: args.tls_key_file,
        tls_ca_file: args.tls_ca_file,
    };

    wait_forever().await;
}

async fn wait_forever() {
    futures::future::pending::<()>().await
}
