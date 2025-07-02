use bytes::Bytes;
use chrono::{DateTime, Utc};
use http_body_util::combinators::BoxBody;
use hyper::{Request, Response, client::conn::http1::Connection, service::HttpService};
use hyper_util::rt::TokioTimer;
use std::sync::Arc;
use std::{io::Write, time::Duration};

use crate::{config::Config, proxy::ProxyStream};

pub type BytesBody = BoxBody<Bytes, hyper::Error>;

pub type ClientBuilder = hyper::client::conn::http1::Builder;
pub type ServerBuilder = hyper::server::conn::http1::Builder;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub keep_alive: bool,
    pub half_close: bool,
    pub date_header: bool,
    pub max_body: u64,
    pub client_timeout: Duration,
    pub server_timeout: Duration,
}

impl HttpConfig {
    pub fn new(config: &Config) -> anyhow::Result<HttpConfig> {
        Ok(HttpConfig {
            keep_alive: config.http_keep_alive,
            half_close: config.http_half_close,
            date_header: config.http_date_header,
            max_body: config.http_max_body.as_u64(),
            client_timeout: config.client_timeout,
            server_timeout: config.server_timeout,
        })
    }

    pub fn server_builder(&self) -> ServerBuilder {
        let mut builder = ServerBuilder::new();
        builder.preserve_header_case(true);
        builder.half_close(self.half_close);
        builder.keep_alive(self.keep_alive);
        builder.auto_date_header(self.date_header);

        builder.timer(TokioTimer::new());
        builder.header_read_timeout(self.client_timeout);
        builder
    }

    pub fn client_builder(&self) -> ClientBuilder {
        let mut builder = ClientBuilder::new();
        builder.preserve_header_case(true);
        builder
    }
}

pub enum HttpMessage {
    Response {
        response: Response<Bytes>,
        timestamp: DateTime<Utc>,
    },
    Request {
        request: Request<Bytes>,
        timestamp: DateTime<Utc>,
    },
}

impl HttpMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            HttpMessage::Response { response, .. } => Self::response_to_bytes(response),
            HttpMessage::Request { request, .. } => Self::request_to_bytes(request),
        }
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            HttpMessage::Response { timestamp, .. } => *timestamp,
            HttpMessage::Request { timestamp, .. } => *timestamp,
        }
    }

    fn version_to_bytes(ver: http::Version) -> &'static str {
        match ver {
            http::Version::HTTP_09 => "HTTP/0.9",
            http::Version::HTTP_10 => "HTTP/1.0",
            http::Version::HTTP_11 => "HTTP/1.1",
            http::Version::HTTP_2 => "HTTP/2.0",
            http::Version::HTTP_3 => "HTTP/3.0",
            _ => panic!("There were more?"),
        }
    }

    fn request_to_bytes(req: &Request<Bytes>) -> Vec<u8> {
        let mut buf = Vec::new();
        write!(
            &mut buf,
            "{} {} {}\r\n",
            req.method(),
            req.uri(),
            Self::version_to_bytes(req.version())
        )
        .unwrap();

        for (name, value) in req.headers() {
            write!(&mut buf, "{}: {}\r\n", name, value.to_str().unwrap()).unwrap();
        }
        buf.extend_from_slice(b"\r\n");

        buf.extend_from_slice(&req.body()[..]);
        buf
    }

    fn response_to_bytes(resp: &Response<Bytes>) -> Vec<u8> {
        let mut buf = Vec::new();

        write!(
            &mut buf,
            "{} {} {}\r\n",
            Self::version_to_bytes(resp.version()),
            resp.status().as_u16(),
            resp.status().canonical_reason().unwrap_or(""),
        )
        .unwrap();

        for (name, value) in resp.headers() {
            write!(&mut buf, "{}: {}\r\n", name, value.to_str().unwrap()).unwrap();
        }
        buf.extend_from_slice(b"\r\n");

        buf.extend_from_slice(&resp.body()[..]);
        buf
    }
}
