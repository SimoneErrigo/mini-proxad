use std::net::SocketAddr;

use rustls::{ClientConfig, pki_types::ServerName};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::{proxy::stream::ProxyStream, service::Service};

pub struct Connector {
    server_addr: SocketAddr,
    tls_config: Option<Arc<ClientConfig>>,
}

impl Connector {
    pub async fn new(service: &Service) -> anyhow::Result<Connector> {
        Ok(Connector {
            server_addr: service.server_addr,
            tls_config: service
                .tls_config
                .clone()
                .map(|config| config.client_config),
        })
    }

    pub async fn connect(&self) -> anyhow::Result<ProxyStream> {
        let stream = TcpStream::connect(self.server_addr).await?;
        if let Some(config) = self.tls_config.clone() {
            let server_name = ServerName::try_from(self.server_addr.ip())?;
            let connector = TlsConnector::from(config);
            Ok(Box::pin(connector.connect(server_name, stream).await?))
        } else {
            Ok(Box::pin(stream))
        }
    }
}
