mod filter;
mod proxy;
mod service;
mod tls;

use clap::Parser;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::filter::Filter;
use crate::proxy::Proxy;
use crate::service::Service;
use crate::tls::TlsConfig;

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

    #[arg(
        long,
        value_parser = humantime::parse_duration,
        value_name = "DURATION",
        default_value = "30s"
    )]
    pub client_timeout: Duration,

    #[arg(
        long,
        value_parser = humantime::parse_duration,
        value_name = "DURATION",
        default_value = "10s"
    )]
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

    #[clap(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let filter = if args.verbose {
        EnvFilter::new("mini_proxad=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Welcome to Proxad (mini edition) ^-^");

    let tls_config = if args.tls_enabled {
        match TlsConfig::new(
            &args.tls_cert_file.unwrap(),
            &args.tls_key_file.unwrap(),
            args.tls_ca_file.as_deref(),
        ) {
            Ok(config) => {
                info!("Tls configuration loaded");
                Some(config)
            }
            Err(e) => {
                error!("Failed to load Tls config: {}", e);
                return;
            }
        }
    } else {
        info!("Tls disabled");
        None
    };

    let filter = args.python_script.as_ref().and_then(|path| {
        Filter::load_from_file(&path)
            .map(|filter| {
                info!("Loaded python script {}", path);
                Arc::new(filter)
            })
            .map_err(|e| {
                error!("Failed to load python script {}: {}", path, e);
            })
            .ok()
    });

    let service = Service {
        name: args.service_name,
        client_addr: SocketAddr::new(args.client_ip, args.client_port),
        server_addr: SocketAddr::new(args.server_ip, args.server_port),
        client_timeout: args.client_timeout,
        server_timeout: args.server_timeout,
        tls_config,
        filter,
    };

    match Proxy::from_service(service).await {
        Ok(proxy) => proxy.start().await,
        Err(e) => error!("Proxy failed to start: {}", e),
    }
}
