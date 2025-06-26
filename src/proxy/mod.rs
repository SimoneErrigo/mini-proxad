mod acceptor;
mod connector;
mod stream;

use crate::service::Service;
use crate::{filter::Filter, proxy::stream::ProxyStream};
use acceptor::Acceptor;
use connector::Connector;
use pyo3::Python;

use std::sync::Arc;
use tokio::{io::AsyncWriteExt, select, task::JoinHandle, time::timeout};
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct Proxy {
    inner: Arc<ProxyInner>,
}

struct ProxyInner {
    service: Service,
    acceptor: Acceptor,
    connector: Connector,
}

pub struct Connection {
    pub client: ProxyStream,
    pub server: ProxyStream,
}

impl Proxy {
    pub async fn start(service: Service) -> anyhow::Result<JoinHandle<()>> {
        let acceptor = Acceptor::new(&service).await?;
        let connector = Connector::new(&service).await?;

        let proxy = Proxy {
            inner: Arc::new(ProxyInner {
                service,
                acceptor,
                connector,
            }),
        };

        Ok(tokio::spawn(async move { proxy.handle_accept().await }))
    }

    async fn handle_accept(&self) {
        loop {
            let (client, addr) = match self.inner.acceptor.accept().await {
                Ok((client, addr)) => {
                    info!("Accepted connection from {}", addr);
                    (client, addr)
                }
                Err(e) => {
                    warn!("Failed to connect to client: {}", e);
                    continue;
                }
            };

            let server = match self.inner.connector.connect().await {
                Ok(server) => server,
                Err(e) => {
                    warn!(
                        "Failed to connect to service on {}: {}",
                        self.inner.service.server_addr, e
                    );
                    continue;
                }
            };

            let mut connection = Connection { client, server };
            let proxy = self.clone();

            tokio::spawn(async move {
                match proxy.handle_connection(&mut connection).await {
                    Ok(_) => info!("Closed connection from {}", addr),
                    Err(e) => warn!("Error in connection from {}: {}", addr, e),
                };
            });
        }
    }

    async fn handle_filter(filter: &Option<Arc<Filter>>, name: &str, chunk: &mut Vec<u8>) {
        if let Some(filter) = filter {
            debug!("Running filter {}", name);
            match filter.apply(name, &chunk).await {
                Ok(bytes) => {
                    chunk.clear();
                    Python::with_gil(|py| match bytes.extract::<&[u8]>(py) {
                        Ok(bytes) => {
                            debug!("Modified chunk: {:?}", String::from_utf8_lossy(&bytes));
                            chunk.extend_from_slice(bytes);
                        }
                        Err(e) => warn!("Failed to convert bytes: {}", e),
                    });
                }
                Err(e) => warn!("Failed to run filter {}: {}", name, e),
            }
        }
    }

    async fn handle_connection(&self, connection: &mut Connection) -> anyhow::Result<()> {
        let mut client_buf = vec![];
        let mut server_buf = vec![];

        let client_timeout = self.inner.service.client_timeout;
        let server_timeout = self.inner.service.server_timeout;

        let filter = self.inner.service.filter.clone();

        loop {
            select! {
                // Client -> Server
                client_read = timeout(client_timeout, connection.client.read_chunk(&mut client_buf)) => {
                    match client_read {
                        Ok(Ok(0)) => {
                            debug!("Client sent eof");
                            connection.server.shutdown().await?;
                            break;
                        }
                        Ok(Ok(_)) => {
                            debug!("Client → Server: {:?}", String::from_utf8_lossy(&client_buf));
                            Self::handle_filter(&filter, "client_filter", &mut client_buf).await;
                            connection.server.write_chunk(&client_buf).await?;
                            client_buf.clear();
                        }
                        Ok(Err(e)) => return Err(e.into()),
                        Err(_) => {
                            debug!("Client reached timeout");
                            break;
                        }
                    }
                }

                // Server -> Client
                server_read = timeout(server_timeout, connection.server.read_chunk(&mut server_buf)) => {
                    match server_read {
                        Ok(Ok(0)) => {
                            debug!("Server sent eof");
                            connection.client.shutdown().await?;
                            break;
                        }
                        Ok(Ok(_)) => {
                            debug!("Server → Client: {:?}", String::from_utf8_lossy(&server_buf));
                            Self::handle_filter(&filter, "server_filter", &mut server_buf).await;
                            connection.client.write_chunk(&server_buf).await?;
                            server_buf.clear();
                        }
                        Ok(Err(e)) => return Err(e.into()),
                        Err(_) => {
                            debug!("Client reached timeout");
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
