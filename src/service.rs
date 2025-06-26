use anyhow::Context;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::proxy::Filter;
use crate::tls::TlsConfig;

#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_timeout: Duration,
    pub server_timeout: Duration,
    pub tls_config: Option<TlsConfig>,
    pub filter: Option<Arc<Filter>>,
}

impl Service {
    pub fn from_config(config: &Config) -> anyhow::Result<Service> {
        let tls_config = config
            .tls_enabled
            .then(|| {
                TlsConfig::new(
                    &config
                        .tls_cert_file
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("TLS certificate is required"))?,
                    &config
                        .tls_key_file
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("TLS key is required"))?,
                    config.tls_ca_file.as_deref(),
                )
                .context("Failed to load TLS config")
            })
            .transpose()?;

        Ok(Service {
            name: config.service_name.clone(),
            client_addr: SocketAddr::new(config.client_ip, config.client_port),
            server_addr: SocketAddr::new(config.server_ip, config.server_port),
            client_timeout: config.client_timeout,
            server_timeout: config.server_timeout,
            tls_config,
            filter: None,
        })
    }
}
