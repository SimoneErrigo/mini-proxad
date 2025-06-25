use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::filter::Filter;
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
