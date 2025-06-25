mod acceptor;
mod connector;
mod stream;

use crate::proxy::stream::ProxyStream;
use crate::service::Service;
use acceptor::Acceptor;
use connector::Connector;

use std::sync::Arc;
use tokio::{io::AsyncWriteExt, select, time::sleep};
use tracing::{debug, error, info, warn};

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
    pub async fn from_service(service: Service) -> anyhow::Result<Proxy> {
        let acceptor = Acceptor::new(&service).await?;
        let connector = Connector::new(&service).await?;

        Ok(Proxy {
            inner: Arc::new(ProxyInner {
                service,
                acceptor,
                connector,
            }),
        })
    }

    async fn handle_connection(&self, connection: &mut Connection) -> anyhow::Result<()> {
        let mut client_buf = vec![];
        let mut server_buf = vec![];

        loop {
            select! {
                // Client -> Server
                client_read = connection.client.read_chunk(&mut client_buf) => {
                    if client_read? == 0 {
                        debug!("Client sent EOF");
                        connection.server.shutdown().await?;
                        break;
                    }

                    debug!("Client → Server: {:?}", String::from_utf8_lossy(&client_buf));
                    connection.server.write_chunk(&client_buf).await?;
                    client_buf.clear();
                }

                // Server -> Client
                server_read = connection.server.read_chunk(&mut server_buf) => {
                    if server_read? == 0 {
                        debug!("Server sent EOF");
                        connection.client.shutdown().await?;
                        break;
                    }

                    debug!("Server → Client: {:?}", String::from_utf8_lossy(&server_buf));
                    connection.client.write_chunk(&server_buf).await?;
                    server_buf.clear();
                }
            }
        }

        Ok(())
    }

    pub async fn start(&self) {
        info!(
            "Proxying {} for service '{}' at {}",
            self.inner.service.client_addr, self.inner.service.name, self.inner.service.server_addr
        );

        loop {
            let (client, addr) = match self.inner.acceptor.accept().await {
                Ok((client, addr)) => {
                    info!("Accepted connection from {}", addr);
                    (client, addr)
                }
                Err(e) => {
                    warn!("Could not connect to service: {}", e);
                    continue;
                }
            };

            let server = match self.inner.connector.connect().await {
                Ok(server) => server,
                Err(e) => {
                    error!(
                        "Failed to connect to service on {}: {}",
                        self.inner.service.server_addr, e
                    );
                    continue;
                }
            };

            let mut connection = Connection {
                client,
                server: Box::pin(server),
            };

            let proxy = self.clone();
            tokio::spawn(async move {
                match proxy.handle_connection(&mut connection).await {
                    Ok(_) => info!("Closed connection from {}", addr),
                    Err(e) => error!("Error in connection {}: {}", addr, e),
                };
            });
        }
    }
}
