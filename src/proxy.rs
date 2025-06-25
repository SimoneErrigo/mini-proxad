use crate::service::{Service, ServiceListener, ServiceStream};

use rustls::{ClientConfig, ServerConfig};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Notify, OwnedRwLockWriteGuard, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    select,
    time::sleep,
};
use tokio_rustls::{TlsAcceptor, TlsStream};
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct Proxy {}

pub struct Connection {
    pub client: ServiceStream,
    pub server: ServiceStream,
}

impl Proxy {
    async fn handle_connection(
        &self,
        connection: &mut Connection,
        service: Arc<Service>,
    ) -> anyhow::Result<()> {
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

                }
            }
        }

        Ok(())
    }

    async fn handle_listener(&self, listener: ServiceListener, service: Arc<Service>) {
        loop {
            let (client, addr) = listener.accept().await.unwrap();
            info!("Accepted connection from {}", addr);

            let server = match TcpStream::connect(service.server_addr).await {
                Ok(server) => server,
                Err(e) => {
                    error!(
                        "Failed to connect to service on {}: {}",
                        service.server_addr, e
                    );
                    continue;
                }
            };

            let mut connection = Connection {
                client,
                server: Box::pin(server),
            };

            let service = service.clone();
            let proxy = self.clone();

            tokio::spawn(async move {
                match proxy.handle_connection(&mut connection, service).await {
                    Ok(_) => info!("Closed connection from {}", addr),
                    Err(e) => error!("Error in connection {}: {}", addr, e),
                };
            });
        }
    }

    pub async fn start(&self, service: Arc<Service>) -> anyhow::Result<()> {
        //tokio::spawn(async move {
        //    let (client_config, server_config) =
        //        crate::tls::load_tls_config(&tls_cert_file, &tls_key_file, &tls_ca_file);
        //    let tls_acceptor = TlsAcceptor::from(server_config);
        //    proxy
        //        .listen_tls(Arc::new(tls_acceptor), client_config, service)
        //        .await;
        //});

        let listener = if service.tls_cert_file.is_some() {
            info!("Starting TLS service {}", service.name);
            let tls_cert_file = service.tls_cert_file.clone().unwrap();
            let tls_key_file = service.tls_key_file.clone().unwrap();
            let tls_ca_file = service.tls_ca_file.clone().unwrap();
            todo!();
        } else {
            info!("Starting TCP service {}", service.name);
            ServiceListener::bind_tcp(service.client_addr)
                .await
                .unwrap()
        };

        let proxy = self.clone();
        tokio::spawn(async move {
            proxy.handle_listener(listener, service).await;
        });

        Ok(())
    }
}
