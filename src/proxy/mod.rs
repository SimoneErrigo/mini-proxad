mod acceptor;
mod connector;
mod dumper;
#[macro_use]
mod filter;
mod flow;
mod stream;

use crate::config::Config;
use crate::proxy::acceptor::Acceptor;
use crate::proxy::connector::Connector;
use crate::proxy::dumper::{Dumper, DumperChannel};
use crate::proxy::flow::{Flow, FlowStatus};
use crate::service::Service;

// TODO: Figure out visibility
pub use crate::proxy::filter::Filter;
//use crate::proxy::filter::run_filter;

use anyhow::Context;
use pyo3::Python;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{io::AsyncWriteExt, select, task::JoinHandle};
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct Proxy {
    inner: Arc<ProxyInner>,
}

struct ProxyInner {
    service: Service,
    acceptor: Acceptor,
    connector: Connector,
    dumper: Option<DumperChannel>,
}

impl Proxy {
    pub async fn start(service: Service, config: &Config) -> anyhow::Result<JoinHandle<()>> {
        let acceptor = Acceptor::new(&service).await?;
        let connector = Connector::new(&service).await?;
        let dumper = if config.dump_enabled {
            Some(
                Dumper::start(&service, config)
                    .await
                    .context("Failed to start dumper")?,
            )
        } else {
            None
        };

        let proxy = Proxy {
            inner: Arc::new(ProxyInner {
                service,
                acceptor,
                connector,
                dumper,
            }),
        };

        Ok(tokio::spawn(async move { proxy.handle_accept().await }))
    }

    async fn handle_accept(&self) {
        loop {
            let (client, addr) = match self.inner.acceptor.accept().await {
                Ok((client, addr)) => {
                    info!("Accepted flow from {}", addr);
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

            let mut flow = Flow::new(client, server);
            let proxy = self.clone();

            tokio::spawn(async move {
                match proxy.handle_flow(&mut flow).await {
                    Ok(_) => info!("Closed flow from {}", addr),
                    Err(e) => warn!("Error in flow from {}: {}", addr, e),
                };
            });
        }
    }

    async fn dump_chunk(
        &self,
        src: SocketAddr,
        dst: SocketAddr,
        chunk: Vec<u8>,
    ) -> anyhow::Result<()> {
        if let Some(ref channel) = self.inner.dumper {
            Dumper::write_chunk(channel, src, dst, chunk).await
        } else {
            Ok(())
        }
    }

    async fn handle_flow(&self, flow: &mut Flow) -> anyhow::Result<()> {
        let client_timeout = self.inner.service.client_timeout;
        let server_timeout = self.inner.service.server_timeout;
        let filter = self.inner.service.filter.clone();

        run_filter!(filter, on_flow_start, flow);

        loop {
            select! {
                // Client -> Server
                client_status = Flow::read_chunk(&mut flow.client, &mut flow.client_history, client_timeout) => {
                    match client_status? {
                        FlowStatus::Read => {
                            debug!(
                                "Client → Server: {:?}",
                                String::from_utf8_lossy(flow.client_history.last_chunk())
                            );

                            run_filter!(filter, on_client_chunk, flow);

                            Flow::write_last_chunk(
                                &mut flow.server,
                                &mut flow.client_history,
                                client_timeout,
                            )
                            .await?;

                            //flow.server.write_chunk(&chunk).await?;
                            //self.dump_chunk(client_addr, server_addr, std::mem::take(&mut client_buf)).await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Client sent eof");
                            flow.server.shutdown().await?;
                            break;
                        }
                        FlowStatus::Timeout => {
                            debug!("Client timeout elapsed");
                            break;
                        }
                        FlowStatus::HistoryTooBig => {
                            debug!("Client history size reached limit, flow terminated");
                            break;
                        }
                    }
                }

                // Server -> Client
                server_status = Flow::read_chunk(&mut flow.server, &mut flow.server_history, server_timeout) => {
                    match server_status? {
                        FlowStatus::Read => {
                            debug!(
                                "Server → Client: {:?}",
                                String::from_utf8_lossy(flow.server_history.last_chunk())
                            );

                            run_filter!(filter, on_server_chunk, flow);

                            Flow::write_last_chunk(
                                &mut flow.client,
                                &mut flow.server_history,
                                server_timeout,
                            )
                            .await?;

                            //flow.client.write_chunk(&chunk).await?;
                            //self.dump_chunk(server_addr, client_addr, std::mem::take(&mut server_buf)).await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Server sent eof");
                            flow.client.shutdown().await?;
                            break;
                        }
                        FlowStatus::Timeout => {
                            debug!("Server timeout elapsed");
                            break;
                        }
                        FlowStatus::HistoryTooBig => {
                            debug!("Server history size reached limit, flow terminated");
                            break;
                        }
                    }
                }
            }
        }

        run_filter!(filter, on_flow_close, flow);

        debug!(
            client_history = flow.client_history.bytes.len(),
            client_chunks = flow.client_history.chunks.len(),
            server_history = flow.server_history.bytes.len(),
            server_chunks = flow.server_history.chunks.len(),
            "History size for flow {}",
            flow.id,
        );

        Ok(())
    }
}
