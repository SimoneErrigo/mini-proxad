use http::HeaderValue;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http_body_util::{BodyExt, Full, Limited, combinators::BoxBody};
use hyper::body::{Bytes, Incoming as IncomingBody};
use hyper::client::conn::http1::SendRequest;
use hyper::service::Service as HyperService;
use hyper::{Request, Response};
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{error, info, trace};

use crate::flow::{HttpFlow, IsFlow};
use crate::http::{BytesBody, HttpResponse};
use crate::proxy::Proxy;
use crate::run_filter;

const RESPONSE_TOO_BIG: &str = "Response body too big";
const REQUEST_TOO_BIG: &str = "Response body too big";
const FILTER_KILLED: &str = "Killed by filter";
const FILTER_INVALID: &str = "Invalid filter output";
const SERVER_TIMEOUT: &str = "Server timeout elapsed";
const CLIENT_HISTORY: &str = "Client history too big";
const SERVER_HISTORY: &str = "Server history too big";

struct ProxyHyperInner {
    sender: SendRequest<BytesBody>,
    flow: HttpFlow,
    error: Option<anyhow::Error>,
}

#[derive(Clone)]
pub struct ProxyHyper {
    pub proxy: Proxy,
    pub max_body: u64,
    inner: Arc<Mutex<ProxyHyperInner>>,
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
            max_body,
            inner: Arc::new(Mutex::new(ProxyHyperInner {
                sender,
                flow,
                error: None,
            })),
        }
    }

    pub fn into_flow(self) -> Option<HttpFlow> {
        if let Ok(mutex) = Arc::try_unwrap(self.inner) {
            Some(mutex.into_inner().flow)
        } else {
            None
        }
    }

    fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
        Full::new(chunk.into())
            .map_err(|never| match never {})
            .boxed()
    }

    async fn push_request(
        inner: &mut ProxyHyperInner,
        mut req: Request<Bytes>,
        len: usize,
    ) -> anyhow::Result<()> {
        info!("Client requested {} {}", req.method(), req.uri());
        trace!("{:#?}", req);

        if req.headers().contains_key(TRANSFER_ENCODING) {
            req.headers_mut().remove(TRANSFER_ENCODING);
            req.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from(len));
        }

        if !inner.flow.history.push_request(req, len) {
            Err(anyhow::anyhow!(CLIENT_HISTORY))
        } else {
            Ok(())
        }
    }

    async fn push_response(
        inner: &mut ProxyHyperInner,
        mut resp: Response<Bytes>,
        len: usize,
    ) -> anyhow::Result<()> {
        info!("Server responded with status {}", resp.status().as_u16());
        trace!("{:#?}", resp);

        if resp.headers().contains_key(TRANSFER_ENCODING) {
            resp.headers_mut().remove(TRANSFER_ENCODING);
            resp.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from(len));
        }

        if !inner.flow.history.push_response(resp, len) {
            Err(anyhow::anyhow!(SERVER_HISTORY))
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
        let service = self.clone();
        Box::pin(async move {
            let mut guard = service.inner.lock().await;

            if let Some(error) = guard.error.take() {
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
                    Self::push_request(&mut guard, history_req, 0).await?;

                    let mut resp = Response::new(Self::full(REQUEST_TOO_BIG));
                    *resp.status_mut() = hyper::StatusCode::PAYLOAD_TOO_LARGE;

                    let mut history_resp = Response::new(Bytes::from(REQUEST_TOO_BIG));
                    *history_resp.status_mut() = hyper::StatusCode::PAYLOAD_TOO_LARGE;
                    Self::push_response(&mut guard, history_resp, 0).await?;

                    // Flag connection as dead
                    guard.error = Some(anyhow::anyhow!(REQUEST_TOO_BIG));
                    return Ok(resp);
                }
            };

            let history_req = Request::from_parts(parts.clone(), body.clone());
            Self::push_request(&mut guard, history_req, body.len()).await?;

            // Check if the request should be blocked by the filter
            run_filter!(service.proxy, on_http_request, &mut guard.flow, {
                info!("Python request filter killed flow {}", guard.flow.get_id());
                
                // If filter killed the connection, return the custom response if available
                if let Some((HttpResponse(resp), _)) = guard.flow.history.responses.last() {
                    let (parts, body) = resp.clone().into_parts();
                    return Ok(Response::from_parts(parts, Self::full(body)));
                } else {
                    // Default blocked response
                    let mut resp = Response::new(Self::full(FILTER_KILLED));
                    *resp.status_mut() = hyper::StatusCode::FORBIDDEN;
                    return Ok(resp);
                }
            });

            // Send the request to the real service
            let req = Request::from_parts(parts, Self::full(body));
            let resp = {
                let timeout = service.proxy.inner.service.server_timeout;
                match time::timeout(timeout, guard.sender.send_request(req)).await {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(e)) => Err(e)?,
                    Err(_) => anyhow::bail!(SERVER_TIMEOUT),
                }
            };

            // Make a copy of the response
            let (parts, incoming) = resp.into_parts();
            let body = match Limited::new(incoming, service.max_body as usize)
                .collect()
                .await
            {
                Ok(body) => body.to_bytes(),
                Err(_) => {
                    let resp = Response::from_parts(parts.clone(), Bytes::from(RESPONSE_TOO_BIG));
                    Self::push_response(&mut guard, resp, 0).await?;
                    return Err(anyhow::anyhow!(RESPONSE_TOO_BIG));
                }
            };

            let body_len = body.len();
            let history_resp = Response::from_parts(parts, body);
            Self::push_response(&mut guard, history_resp, body_len).await?;

            let resp = {
                run_filter!(service.proxy, on_http_response, &mut guard.flow, {
                    info!("Python server filter killed flow {}", guard.flow.get_id());
                    anyhow::bail!(FILTER_KILLED)
                });

                match guard.flow.history.responses.last() {
                    Some((HttpResponse(resp), _)) => {
                        let (parts, body) = resp.clone().into_parts();
                        Response::from_parts(parts, Self::full(body))
                    }
                    _ => {
                        error!(
                            "Python filter did not return a HTTP response for flow {}",
                            guard.flow.get_id()
                        );
                        anyhow::bail!(FILTER_INVALID)
                    }
                }
            };

            Ok(resp)
        })
    }
}
