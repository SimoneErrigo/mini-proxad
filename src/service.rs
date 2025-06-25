use humantime::Duration;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::stream::{AsyncListener, ChunkStream};

#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_timeout: Duration,
    pub server_timeout: Duration,
    pub tls_cert_file: Option<String>,
    pub tls_key_file: Option<String>,
    pub tls_ca_file: Option<String>,
}

pub type ServiceStream = Pin<Box<dyn ChunkStream>>;

pub struct ServiceListener {
    tcp: TcpListener,
    tls: Option<TlsAcceptor>,
}

impl ServiceListener {
    pub async fn bind_tcp(addr: SocketAddr) -> anyhow::Result<Self> {
        let tcp = TcpListener::bind(addr).await?;
        Ok(Self { tcp, tls: None })
    }

    pub async fn bind_tls(addr: SocketAddr) -> anyhow::Result<Self> {
        let tcp = TcpListener::bind(addr).await?;
        Ok(Self { tcp, tls: None })
    }

    pub async fn accept(&self) -> anyhow::Result<(ServiceStream, SocketAddr)> {
        let (stream, addr) = self.tcp.accept().await?;

        if let Some(ref acceptor) = self.tls {
            Ok((Box::pin(acceptor.accept(stream).await?), addr))
        } else {
            Ok((Box::pin(stream), addr))
        }
    }
}
