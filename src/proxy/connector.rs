use rustls::pki_types::ServerName;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::proxy::stream::ProxyStream;
use crate::service::Service;

pub struct Connector {
    server_addr: SocketAddr,
    server_name: Option<ServerName<'static>>,
    tls_connector: Option<TlsConnector>,
}

impl Connector {
    pub async fn new(service: &Service) -> anyhow::Result<Connector> {
        Ok(Connector {
            server_addr: service.server_addr,
            server_name: service
                .tls_config
                .as_ref()
                .map(|_| ServerName::from(service.server_addr.ip())),
            tls_connector: service
                .tls_config
                .as_ref()
                .map(|config| TlsConnector::from(config.client_config.clone())),
        })
    }

    pub async fn connect(&self) -> anyhow::Result<ProxyStream> {
        let stream = TcpStream::connect(self.server_addr).await?;
        if let Some(ref connector) = self.tls_connector {
            let server_name = self.server_name.clone().unwrap();
            Ok(Box::pin(connector.connect(server_name, stream).await?))
        } else {
            Ok(Box::pin(stream))
        }
    }
}
