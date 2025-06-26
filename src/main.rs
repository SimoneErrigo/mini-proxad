mod config;
mod filter;
mod proxy;
mod service;
mod tls;

use clap::Parser;
use std::process::exit;
use std::sync::{Arc, RwLock};
use tokio;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::proxy::Proxy;
use crate::service::Service;

#[derive(Debug, Clone, Parser)]
#[command(author = "nect", version)]
#[command(about = "Proxy options", long_about = None)]
struct Args {
    #[arg(short, long)]
    verbose: bool,

    #[arg(long = "config", default_value = "proxad.yml")]
    config_path: String,
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

    let config = match Config::load_from_file(&args.config_path) {
        Ok(config) => {
            info!("Loaded configuration from {}", args.config_path);
            config
        }
        Err(e) => {
            error!("Failed to load config from {}: {}", args.config_path, e);
            exit(1);
        }
    };

    let service = match Service::from_config(config) {
        Ok(service) => {
            info!("Loaded service {}", service.name);
            service
        }
        Err(e) => {
            error!("Failed to load service: {}", e);
            exit(1);
        }
    };

    match Proxy::from_service(service).await {
        Ok(proxy) => proxy.start().await,
        Err(e) => error!("Proxy failed to start: {}", e),
    }
}
