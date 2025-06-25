use crate::proxy::stream::ProxyStream;
use crate::service::Service;

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub struct Acceptor {
    listener: TcpListener,
    tls_acceptor: Option<TlsAcceptor>,
}

impl Acceptor {
    pub async fn new(service: &Service) -> anyhow::Result<Acceptor> {
        let listener = TcpListener::bind(service.client_addr).await?;
        Ok(Acceptor {
            listener,
            tls_acceptor: service
                .tls_config
                .clone()
                .map(|config| TlsAcceptor::from(config.server_config)),
        })
    }

    pub async fn accept(&self) -> anyhow::Result<(ProxyStream, SocketAddr)> {
        let (stream, addr) = self.listener.accept().await?;
        if let Some(ref acceptor) = self.tls_acceptor {
            Ok((Box::pin(acceptor.accept(stream).await?), addr))
        } else {
            Ok((Box::pin(stream), addr))
        }
    }
}
