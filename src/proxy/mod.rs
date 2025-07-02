mod acceptor;
mod connector;
mod dumper;
mod filter;

use crate::config::Config;
use crate::flow::history::{HttpHistory, RawChunk, RawHistory};
use crate::flow::{Flow, HttpFlow, RawFlow};
use crate::http::BytesBody;
use crate::proxy::acceptor::Acceptor;
use crate::proxy::connector::Connector;
use crate::proxy::dumper::{Dumper, DumperChannel};
use crate::service::Service;
use crate::stream::{ChunkRead, ChunkStream, ChunkWrite};

use anyhow::Context;
use chrono::Utc;
use http_body_util::{BodyExt, Empty, Full, Limited, combinators::BoxBody};
use hyper::body::{Bytes, Incoming as IncomingBody};
use hyper::service::Service as HyperService;
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::time::{self, timeout};
use tokio::{select, task::JoinHandle};
use tracing::{debug, info, trace, warn};

pub use crate::proxy::filter::Filter;

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

            let proxy = self.clone();
            let dumper = self.inner.dumper.clone();

            if let Some(http) = self.inner.service.http_config.clone() {
                let flow = HttpFlow::new(
                    client_addr,
                    self.inner.service.client_max_history,
                    self.inner.service.server_addr,
                    self.inner.service.server_max_history,
                );

                let client_io = TokioIo::new(client);
                let server_io = TokioIo::new(server);

                if let Ok((sender, conn)) = http.client_builder().handshake(server_io).await {
                    let upstream = tokio::spawn(async move {
                        match conn.await {
                            Ok(_) => debug!("Upstream HTTP connection closed"),
                            Err(e) => warn!("Upstream HTTP connection error: {:?}", e),
                        }
                    });

                    let service = ProxyHyper::new(proxy, sender, http.max_body, flow);
                    let inner = service.inner.clone();

                    tokio::task::spawn(async move {
                        match http
                            .server_builder()
                            .serve_connection(client_io, service)
                            .await
                        {
                            Ok(_) => info!("Closed HTTP flow from {}", client_addr),
                            Err(e) => warn!("Error in flow from {}: {:?}", client_addr, e),
                        };

                        upstream.abort();

                        if let Ok(mutex) = Arc::try_unwrap(inner) {
                            let flow = mutex.into_inner().flow;

                            if let Some(ref channel) = dumper {
                                if let Err(e) = channel.try_send(Flow::Http(flow)) {
                                    warn!("Could not send flow to dumper: {}", e);
                                }
                            }
                        }
                    });
                }
            } else {
                let mut flow = RawFlow::new(
                    client_addr,
                    self.inner.service.client_max_history,
                    self.inner.service.server_addr,
                    self.inner.service.server_max_history,
                );

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

                    if let Some(ref channel) = dumper {
                        if let Err(e) = channel.try_send(Flow::Raw(flow)) {
                            warn!("Could not send flow to dumper: {}", e);
                        }
                    }
                });
            }
        }
    }

    pub async fn read_chunk<R: ChunkRead>(
        stream: &mut R,
        history: &mut RawHistory,
        timeout: Duration,
    ) -> anyhow::Result<FlowStatus> {
        let start = history.bytes.len();
        let future = stream.read_chunk(&mut history.bytes);

        match time::timeout(timeout, future).await {
            Ok(Ok(0)) => Ok(FlowStatus::Closed),
            Ok(Ok(n)) => {
                history.chunks.push(RawChunk {
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
        history: &RawHistory,
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

    #[tracing::instrument(level = "trace", skip_all)]
    async fn handle_flow(
        &self,
        mut client: ProxyStream,
        mut server: ProxyStream,
        flow: &mut RawFlow,
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

        client.flush().await?;
        server.flush().await?;

        client.shutdown().await?;
        server.shutdown().await?;

        Ok(())
    }
}

use hyper::client::conn::http1::SendRequest;

struct ProxyHyperInner {
    sender: SendRequest<BytesBody>,
    flow: HttpFlow,
    error: Option<anyhow::Error>,
}

#[derive(Clone)]
struct ProxyHyper {
    proxy: Proxy,
    inner: Arc<Mutex<ProxyHyperInner>>,
    max_body: u64,
}

impl ProxyHyper {
    pub fn new(
        proxy: Proxy,
        sender: SendRequest<BytesBody>,
        max_body: u64,
        flow: HttpFlow,
    ) -> ProxyHyper {
        ProxyHyper {
            proxy,
            inner: Arc::new(Mutex::new(ProxyHyperInner {
                sender,
                flow,
                error: None,
            })),
            max_body,
        }
    }

    fn empty() -> BoxBody<Bytes, hyper::Error> {
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed()
    }

    fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
        Full::new(chunk.into())
            .map_err(|never| match never {})
            .boxed()
    }

    async fn push_request(&self, req: Request<Bytes>, len: usize) -> anyhow::Result<()> {
        info!("Client requested {} {}", req.method(), req.uri());
        trace!("{:#?}", req);

        let mut guard = self.inner.lock().await;
        if !guard.flow.history.push_request(req, len) {
            Err(anyhow::anyhow!("Client history too big"))
        } else {
            Ok(())
        }
    }

    async fn push_response(&self, resp: Response<Bytes>, len: usize) -> anyhow::Result<()> {
        info!("Server responded with {}", resp.status().as_u16());
        trace!("{:#?}", resp);

        let mut guard = self.inner.lock().await;
        if !guard.flow.history.push_response(resp, len) {
            Err(anyhow::anyhow!("Server history too big"))
        } else {
            Ok(())
        }
    }
}

impl HyperService<Request<IncomingBody>> for ProxyHyper {
    type Response = Response<BoxBody<Bytes, hyper::Error>>;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        const RESPONSE_TOO_BIG: &str = "Response body too big";
        const REQUEST_TOO_BIG: &str = "Response body too big";

        let service = self.clone();
        Box::pin(async move {
            if let Some(error) = service.inner.lock().await.error.take() {
                return Err(error);
            }

            // Make a copy of the request
            let (parts, incoming) = req.into_parts();
            let body = match Limited::new(incoming, service.max_body as usize)
                .collect()
                .await
            {
                Ok(body) => body.to_bytes(),
                Err(_) => {
                    let history_req = Request::from_parts(parts, Bytes::from(REQUEST_TOO_BIG));
                    service.push_request(history_req, 0).await?;

                    let mut resp = Response::new(Self::full(REQUEST_TOO_BIG));
                    *resp.status_mut() = hyper::StatusCode::PAYLOAD_TOO_LARGE;

                    let mut history_resp = Response::new(Bytes::from(REQUEST_TOO_BIG));
                    *history_resp.status_mut() = hyper::StatusCode::PAYLOAD_TOO_LARGE;
                    service.push_response(history_resp, 0).await?;

                    // Flag connection as dead
                    service.inner.lock().await.error = Some(anyhow::anyhow!(REQUEST_TOO_BIG));
                    return Ok(resp);
                }
            };

            let history_req = Request::from_parts(parts.clone(), body.clone());
            service.push_request(history_req, body.len()).await?;

            // Send the request to the real service
            let req = Request::from_parts(parts, Self::full(body));
            let resp = service.inner.lock().await.sender.send_request(req).await?;

            // Make a copy of the response
            let (parts, incoming) = resp.into_parts();
            let body = match Limited::new(incoming, service.max_body as usize)
                .collect()
                .await
            {
                Ok(body) => body.to_bytes(),
                Err(_) => {
                    let resp = Response::from_parts(parts.clone(), Bytes::from(RESPONSE_TOO_BIG));
                    service.push_response(resp, 0).await?;
                    return Err(anyhow::anyhow!(RESPONSE_TOO_BIG));
                }
            };

            let history_resp = Response::from_parts(parts.clone(), body.clone());
            service.push_response(history_resp, body.len()).await?;

            let resp = Response::from_parts(parts, Self::full(body));
            Ok(resp)
        })
    }
}
