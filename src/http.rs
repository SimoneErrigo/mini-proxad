use bytes::Bytes;
use chrono::{DateTime, Utc};
use http::header::CONTENT_LENGTH;
use http::{Method, Uri};
use http_body_util::combinators::BoxBody;
use hyper::{Request, Response, service::HttpService};
use hyper_util::rt::TokioTimer;
use pyo3::types::{PyAnyMethods, PyBytes, PyDict, PyDictMethods};
use pyo3::{Bound, FromPyObject, IntoPyObject, Py, PyAny, PyErr, PyResult, Python};
use std::{io::Write, time::Duration};

use crate::filter::api::{PyHttpMessage, PyHttpRequest, PyHttpResponse};
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

#[derive(Debug, Clone)]
pub struct HttpResponse(pub Response<Bytes>);

#[derive(Debug, Clone)]
pub struct HttpRequest(pub Request<Bytes>);

#[derive(Debug, Clone)]
pub enum HttpMessage {
    Response {
        response: HttpResponse,
        timestamp: DateTime<Utc>,
    },
    Request {
        request: HttpRequest,
        timestamp: DateTime<Utc>,
    },
}

impl HttpMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            HttpMessage::Response { response, .. } => Self::response_to_bytes(&response.0),
            HttpMessage::Request { request, .. } => Self::request_to_bytes(&request.0),
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

impl<'py> IntoPyObject<'py> for HttpResponse {
    type Target = PyHttpResponse;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let (parts, body) = self.0.into_parts();

        let headers = PyDict::new(py);
        for (name, value) in parts.headers.iter() {
            headers.set_item(name.as_str(), value.to_str().unwrap())?;
        }

        let body = PyBytes::new(py, &body).into();
        let status = parts.status.as_u16();

        let resp: Py<PyHttpResponse> = Py::new(
            py,
            PyHttpResponse::new(headers.clone().into(), body, status),
        )?;

        Ok(resp.into_bound(py))
    }
}

impl<'py> IntoPyObject<'py> for HttpRequest {
    type Target = PyHttpRequest;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let (parts, body) = self.0.into_parts();

        let headers = PyDict::new(py);
        for (name, value) in parts.headers.iter() {
            headers.set_item(name.as_str(), value.to_str().unwrap())?;
        }

        let body = PyBytes::new(py, &body).into();
        let method = parts.method.to_string();
        let uri = PyBytes::new(py, parts.uri.to_string().as_bytes()).into();

        let req: Py<PyHttpRequest> = Py::new(
            py,
            PyHttpRequest::new(headers.clone().into(), body, method, uri),
        )?;

        Ok(req.into_bound(py))
    }
}

impl<'py> IntoPyObject<'py> for HttpMessage {
    type Target = PyHttpMessage;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        match self {
            HttpMessage::Response { response, .. } => {
                let resp: Bound<'py, PyHttpResponse> = response.into_pyobject(py)?;
                Ok(resp.into_super())
            }
            HttpMessage::Request { request, .. } => {
                let req: Bound<'py, PyHttpRequest> = request.into_pyobject(py)?;
                Ok(req.into_super())
            }
        }
    }
}

impl<'py> FromPyObject<'py> for HttpRequest {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let req_bound: &Bound<'py, PyHttpRequest> = ob.downcast()?;
        let req = req_bound.borrow();

        let method = req.method.as_str();
        let uri = req.uri.as_bytes(ob.py());

        let msg_bound: &Bound<'py, PyHttpMessage> = req_bound.as_super();
        let msg = msg_bound.borrow();

        let headers: &Bound<'py, PyDict> = msg.headers.bind(ob.py());
        let body: &[u8] = msg.body.as_ref().extract(ob.py())?;

        let mut builder = Request::builder()
            .method(method.parse::<Method>().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("bad method: {}", e))
            })?)
            .uri(str::from_utf8(uri)?.parse::<Uri>().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("bad uri: {}", e))
            })?);

        for (k, v) in headers.iter() {
            let k: &str = k.extract()?;
            let v: &str = v.extract()?;
            builder = builder.header(k, v);
        }

        Ok(HttpRequest(
            builder
                .body(Bytes::copy_from_slice(body))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?,
        ))
    }
}

impl<'py> FromPyObject<'py> for HttpResponse {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let resp_bound: &Bound<'py, PyHttpResponse> = ob.downcast()?;
        let resp = resp_bound.borrow();

        let status = resp.status;

        let msg_bound: &Bound<'py, PyHttpMessage> = resp_bound.as_super();
        let msg = msg_bound.borrow();

        let headers: &Bound<'py, PyDict> = msg.headers.bind(ob.py());
        let body: &[u8] = msg.body.as_ref().extract(ob.py())?;

        let mut builder = Response::builder().status(status);

        for (k, v) in headers.iter() {
            let k: &str = k.extract()?;
            if k.eq_ignore_ascii_case(CONTENT_LENGTH.as_str()) {
                continue;
            }
            let v: &str = v.extract()?;
            builder = builder.header(k, v);
        }

        Ok(HttpResponse(
            builder
                .body(Bytes::copy_from_slice(body))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?,
        ))
    }
}
