use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{client::conn::http1::Connection, service::HttpService};
use hyper_util::rt::TokioIo;

use crate::{config::Config, proxy::ProxyStream};

pub type BytesBody = BoxBody<Bytes, hyper::Error>;

pub type ClientBuilder = hyper::client::conn::http1::Builder;
pub type ServerBuilder = hyper::server::conn::http1::Builder;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub max_body: u64,
}

impl HttpConfig {
    pub fn new(config: &Config) -> anyhow::Result<HttpConfig> {
        Ok(HttpConfig {
            max_body: config.http_max_body.as_u64(),
        })
    }

    pub fn server_builder(&self) -> ServerBuilder {
        let mut builder = ServerBuilder::new();
        builder.preserve_header_case(true);
        builder.title_case_headers(true);
        builder
    }

    pub fn client_builder(&self) -> ClientBuilder {
        let mut builder = ClientBuilder::new();
        builder.preserve_header_case(true);
        builder.title_case_headers(true);
        builder
    }
}
