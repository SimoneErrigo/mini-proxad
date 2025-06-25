use std::net::SocketAddr;
use std::sync::Arc;

use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::proxy::stream::ProxyStream;
use crate::service::Service;

pub struct Acceptor {
    listener: TcpListener,
    tls_config: Option<Arc<ServerConfig>>,
}

impl Acceptor {
    pub async fn new(service: &Service) -> anyhow::Result<Acceptor> {
        let listener = TcpListener::bind(service.client_addr).await?;
        Ok(Acceptor {
            listener,
            tls_config: service
                .tls_config
                .clone()
                .map(|config| config.server_config),
        })
    }

    pub async fn accept(&self) -> anyhow::Result<(ProxyStream, SocketAddr)> {
        let (stream, addr) = self.listener.accept().await?;
        if let Some(config) = self.tls_config.clone() {
            let acceptor = TlsAcceptor::from(config);
            Ok((Box::pin(acceptor.accept(stream).await?), addr))
        } else {
            Ok((Box::pin(stream), addr))
        }
    }
}
