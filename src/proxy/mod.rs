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

pub use crate::proxy::filter::Filter;

use anyhow::Context;
use std::ops::ControlFlow;
use std::sync::Arc;
use tokio::{select, task::JoinHandle};
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

macro_rules! run_filter {
    ($filter:expr, $method:ident, $arg:expr, $on_break:block) => {
        if let Some(ref filter) = $filter {
            if let ControlFlow::Break(_) = filter.$method($arg).await {
                $on_break
            }
        }
    };
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

            let mut flow = Flow::new(
                client,
                self.inner.service.client_addr,
                server,
                self.inner.service.server_addr,
            );
            let proxy = self.clone();

            tokio::spawn(async move {
                match proxy.handle_flow(&mut flow).await {
                    Ok(_) => info!("Closed flow from {}", addr),
                    Err(e) => warn!("Error in flow from {}: {}", addr, e),
                };

                if let Some(ref channel) = proxy.inner.dumper {
                    if let Err(e) = channel.send(flow) {
                        warn!("Could not send flow to dumper: {}", e);
                    }
                }
            });
        }
    }

    async fn handle_flow(&self, flow: &mut Flow) -> anyhow::Result<()> {
        let client_timeout = self.inner.service.client_timeout;
        let server_timeout = self.inner.service.server_timeout;
        let filter = self.inner.service.filter.clone();

        //run_filter!(filter, on_flow_start, flow, {});

        loop {
            select! {
                // Client -> Server
                client_status = Flow::read_chunk(flow.client.as_mut().unwrap(), &mut flow.client_history, client_timeout) => {
                    match client_status? {
                        FlowStatus::Read => {
                            debug!(
                                "Client → Server: {:?}",
                                String::from_utf8_lossy(flow.client_history.last_chunk())
                            );

                            run_filter!(filter, on_client_chunk, flow, {
                                info!("Python client filter killed flow {}", flow.id);
                                break;
                            });

                            Flow::write_last_chunk(
                                flow.server.as_mut().unwrap(),
                                &mut flow.client_history,
                                client_timeout,
                            )
                            .await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Client sent eof");
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
                server_status = Flow::read_chunk(flow.server.as_mut().unwrap(), &mut flow.server_history, server_timeout) => {
                    match server_status? {
                        FlowStatus::Read => {
                            debug!(
                                "Server → Client: {:?}",
                                String::from_utf8_lossy(flow.server_history.last_chunk())
                            );

                            run_filter!(filter, on_server_chunk, flow, {
                                info!("Python server filter killed flow {}", flow.id);
                                break;
                            });

                            Flow::write_last_chunk(
                                flow.client.as_mut().unwrap(),
                                &mut flow.server_history,
                                server_timeout,
                            )
                            .await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Server sent eof");
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

        //run_filter!(filter, on_flow_close, flow, {});

        // Always close the connection
        flow.close().await?;

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
