mod acceptor;
mod connector;
mod dumper;
mod filter;

use crate::config::Config;
use crate::flow::{Flow, History, HistoryChunk};
use crate::proxy::acceptor::Acceptor;
use crate::proxy::connector::Connector;
use crate::proxy::dumper::{Dumper, DumperChannel};
use crate::service::Service;
use crate::stream::{ChunkRead, ChunkStream, ChunkWrite};

pub use crate::proxy::filter::Filter;

use anyhow::Context;
use chrono::Utc;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time;
use tokio::{select, task::JoinHandle};
use tracing::{debug, info, trace, warn};

pub type ProxyStream = Pin<Box<dyn ChunkStream>>;

pub enum FlowStatus {
    Read,
    Closed,
    Timeout,
    HistoryTooBig,
}

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
    ($proxy:expr, $method:ident, $arg:expr, $on_break:block) => {
        if let Some(ref filter) = $proxy.inner.service.filter.clone() {
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
            let (mut client, client_addr) = match self.inner.acceptor.accept().await {
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

                    if let Err(e) = client.shutdown().await {
                        debug!("Failed to shutdown client {}: {}", client_addr, e);
                    }
                    continue;
                }
            };

            let mut flow = Flow::new(
                client_addr,
                self.inner.service.client_max_history,
                self.inner.service.server_addr,
                self.inner.service.server_max_history,
            );

            let proxy = self.clone();

            tokio::spawn(async move {
                match proxy.handle_flow(client, server, &mut flow).await {
                    Ok(_) => info!("Closed flow from {}", client_addr),
                    Err(e) => warn!("Error in flow from {}: {}", client_addr, e),
                };

                debug!(
                    client_history = flow.client_history.bytes.len(),
                    client_chunks = flow.client_history.chunks.len(),
                    server_history = flow.server_history.bytes.len(),
                    server_chunks = flow.server_history.chunks.len(),
                    "History size for flow {}",
                    flow.id,
                );

                if let Some(ref channel) = proxy.inner.dumper {
                    if let Err(e) = channel.try_send(flow) {
                        warn!("Could not send flow to dumper: {}", e);
                    }
                }
            });
        }
    }

    pub async fn read_chunk<R: ChunkRead>(
        stream: &mut R,
        history: &mut History,
        timeout: Duration,
    ) -> anyhow::Result<FlowStatus> {
        let start = history.bytes.len();
        let future = stream.read_chunk(&mut history.bytes);

        match time::timeout(timeout, future).await {
            Ok(Ok(0)) => Ok(FlowStatus::Closed),
            Ok(Ok(n)) => {
                history.chunks.push(HistoryChunk {
                    range: start..start + n,
                    timestamp: Utc::now(),
                });

                if start + n >= history.max_size {
                    Ok(FlowStatus::HistoryTooBig)
                } else {
                    Ok(FlowStatus::Read)
                }
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(FlowStatus::Timeout),
        }
    }

    pub async fn write_last_chunk<W: ChunkWrite>(
        stream: &mut W,
        history: &History,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let future = async {
            stream.write_chunk(history.last_chunk()).await?;
            stream.flush().await?;
            Ok(())
        };

        match time::timeout(timeout, future).await {
            Ok(ok) => ok,
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn handle_flow(
        &self,
        mut client: ProxyStream,
        mut server: ProxyStream,
        flow: &mut Flow,
    ) -> anyhow::Result<()> {
        let client_timeout = self.inner.service.client_timeout;
        let server_timeout = self.inner.service.server_timeout;

        //run_filter!(filter, on_flow_start, flow, {});

        loop {
            select! {
                // Client -> Server
                client_status = Self::read_chunk(&mut client, &mut flow.client_history, client_timeout) => {
                    match client_status? {
                        FlowStatus::Read => {
                            trace!(
                                "Client → Server: {:?}",
                                String::from_utf8_lossy(flow.client_history.last_chunk())
                            );

                            run_filter!(self, on_client_chunk, flow, {
                                info!("Python client filter killed flow {}", flow.id);
                                break;
                            });

                            Self::write_last_chunk(
                                &mut server,
                                &mut flow.client_history,
                                server_timeout,
                            )
                            .await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Client sent eof");
                            break;
                        }
                        FlowStatus::Timeout => {
                            info!("Client read timeout elapsed");
                            break;
                        }
                        FlowStatus::HistoryTooBig => {
                            warn!("Client history size reached limit, flow terminated");
                            break;
                        }
                    }
                }

                // Server -> Client
                server_status = Self::read_chunk(&mut server, &mut flow.server_history, server_timeout) => {
                    match server_status? {
                        FlowStatus::Read => {
                            trace!(
                                "Server → Client: {:?}",
                                String::from_utf8_lossy(flow.server_history.last_chunk())
                            );

                            run_filter!(self, on_server_chunk, flow, {
                                info!("Python server filter killed flow {}", flow.id);
                                break;
                            });

                            Self::write_last_chunk(
                                &mut client,
                                &mut flow.server_history,
                                client_timeout,
                            )
                            .await?;
                        }
                        FlowStatus::Closed => {
                            debug!("Server sent eof");
                            break;
                        }
                        FlowStatus::Timeout => {
                            info!("Server read timeout elapsed");
                            break;
                        }
                        FlowStatus::HistoryTooBig => {
                            warn!("Server history size reached limit, flow terminated");
                            break;
                        }
                    }
                }
                //else => warn!("WTF")
            }
        }

        //run_filter!(filter, on_flow_close, flow, {});

        // TODO: Is explicit shutdown needed?
        client.shutdown().await?;
        server.shutdown().await?;

        Ok(())
    }
}
