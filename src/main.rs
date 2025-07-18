mod config;
mod filter;
mod flow;
mod http;
mod proxy;
mod service;
mod stream;
mod tls;

use clap::Parser;
use std::process::exit;
use std::sync::Arc;
use tokio;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::Config;
use crate::filter::Filter;
use crate::proxy::Proxy;
use crate::service::Service;

#[derive(Debug, Clone, Parser)]
#[command(author = "nect", version)]
#[command(about = "Proxy options", long_about = None)]
struct Args {
    #[arg(short, long)]
    verbose: bool,

    #[arg(short, long = "config", required = true)]
    config_path: String,

    #[arg(short, long, default_value = "true")]
    watcher: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let filter = if args.verbose {
        EnvFilter::new("mini_proxad=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    fmt::fmt()
        .with_span_events(fmt::format::FmtSpan::CLOSE)
        .with_env_filter(filter)
        .init();

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

    let mut service = match Service::from_config(&config) {
        Ok(service) => {
            info!("Loaded service {}", service.name);
            service
        }
        Err(e) => {
            error!("Failed to load service: {}", e);
            exit(1);
        }
    };

    if config.python_script.is_some() {
        match Filter::load_api() {
            Ok(()) => debug!("Loaded api python module"),
            Err(e) => error!("Failed to load api python module: {}", e),
        }
    }

    let filter = config
        .python_script
        .as_ref()
        .map(|path| Filter::load_from_file(&path))
        .transpose();

    match filter {
        Ok(Some(filter)) => {
            let filter = Arc::new(filter);
            info!(
                "Loaded python filter {}",
                config.python_script.as_ref().unwrap()
            );
            service.filter = Some(filter.clone());

            if args.watcher {
                match filter.spawn_watcher().await {
                    Ok(_) => info!("Started watcher for python filter"),
                    Err(e) => error!("Failed to start watcher for filter: {}", e),
                }
            }
        }
        Ok(None) => debug!("No python filter loaded"),
        Err(e) => {
            error!("Failed to load python filter: {:?}", e);
            exit(1);
        }
    }

    match rlimit::increase_nofile_limit(u64::MAX) {
        Ok(lim) => info!(soft = lim, "Raised NOFILE limits"),
        Err(e) => warn!("Failed to raise NOFILE limits: {}", e),
    }

    let task = match Proxy::start(service, &config).await {
        Ok(task) => {
            info!(
                "Started proxying {}:{} -> {}:{}",
                &config.client_ip, &config.client_port, &config.server_ip, &config.server_port
            );
            task
        }
        Err(e) => {
            error!("Proxy failed to start: {}", e);
            exit(1);
        }
    };

    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            info!("Bye!");
            exit(0)
        }
        Err(e) => error!("Unable to listen for shutdown signal: {}", e),
    }

    task.await.unwrap()
}
