use bytes::Bytes;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http::{HeaderName, HeaderValue, StatusCode};
use http_body_util::combinators::BoxBody;
use hyper::{Request, Response};
use hyper_util::rt::TokioTimer;
use pyo3::types::{PyAnyMethods, PyBytesMethods, PyDictMethods, PyStringMethods};
use pyo3::{Bound, FromPyObject, IntoPyObject, Py, PyAny, PyErr, PyResult, Python};
use std::{io::Write, time::Duration};

use crate::config::Config;
use crate::filter::api::{PyHttpReq, PyHttpResp};

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
}

impl HttpConfig {
    pub fn new(config: &Config) -> anyhow::Result<HttpConfig> {
        Ok(HttpConfig {
            keep_alive: config.http_keep_alive,
            half_close: config.http_half_close,
            date_header: config.http_date_header,
            max_body: config.http_max_body.as_u64(),
            client_timeout: config.client_timeout,
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

impl HttpResponse {
    pub fn to_bytes(&self) -> Vec<u8> {
        let resp = &self.0;
        let mut buf = Vec::new();

        write!(
            &mut buf,
            "{} {} {}\r\n",
            version_to_bytes(resp.version()),
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

#[derive(Debug, Clone)]
pub struct HttpRequest(pub Request<Bytes>);

impl HttpRequest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let req = &self.0;
        let mut buf = Vec::new();

        write!(
            &mut buf,
            "{} {} {}\r\n",
            req.method(),
            req.uri(),
            version_to_bytes(req.version())
        )
        .unwrap();

        for (name, value) in req.headers() {
            write!(&mut buf, "{}: {}\r\n", name, value.to_str().unwrap()).unwrap();
        }
        buf.extend_from_slice(b"\r\n");

        buf.extend_from_slice(&req.body()[..]);
        buf
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

impl<'py> IntoPyObject<'py> for HttpResponse {
    type Target = PyHttpResp;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let resp: Py<PyHttpResp> = Py::new(py, PyHttpResp::new(self))?;
        Ok(resp.into_bound(py))
    }
}

impl<'py> FromPyObject<'py> for HttpResponse {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let resp_bound: &Bound<'py, PyHttpResp> = ob.downcast()?;
        let mut resp = resp_bound.borrow_mut();

        let inner = resp
            .resp
            .take()
            .unwrap_or_else(|| HttpResponse(Response::default()));

        let (mut parts, old_body) = inner.0.into_parts();

        if let Some(headers) = resp.headers.take() {
            parts.headers.clear();

            for (k, v) in headers.bind(ob.py()).iter() {
                let k: &str = k.extract()?;
                if k.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
                    || k.eq_ignore_ascii_case(TRANSFER_ENCODING.as_str())
                {
                    continue;
                }

                let hk = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid header name {}: {}",
                        k, e
                    ))
                })?;

                let hv = HeaderValue::from_str(v.extract()?).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid header value for {}: {}",
                        k, e
                    ))
                })?;

                parts.headers.insert(hk, hv);
            }
        }

        let body = if let Some(body) = resp.body.as_ref() {
            let bound = body.bind(ob.py());
            Bytes::copy_from_slice(&bound.as_bytes())
        } else {
            old_body
        };

        if let Some(status) = resp.status {
            parts.status = StatusCode::from_u16(status).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid status code: {}",
                    e
                ))
            })?;
        }

        Ok(HttpResponse(Response::from_parts(parts, body)))
    }
}

impl<'py> IntoPyObject<'py> for HttpRequest {
    type Target = PyHttpReq;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let req: Py<PyHttpReq> = Py::new(py, PyHttpReq::new(self))?;
        Ok(req.into_bound(py))
    }
}

impl<'py> FromPyObject<'py> for HttpRequest {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let req_bound: &Bound<'py, PyHttpReq> = ob.downcast()?;
        let mut req = req_bound.borrow_mut();

        let inner = req
            .req
            .take()
            .unwrap_or_else(|| HttpRequest(Request::default()));

        let (mut parts, old_body) = inner.0.into_parts();

        if let Some(headers) = req.headers.take() {
            parts.headers.clear();

            for (k, v) in headers.bind(ob.py()).iter() {
                let k: &str = k.extract()?;
                if k.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
                    || k.eq_ignore_ascii_case(TRANSFER_ENCODING.as_str())
                {
                    continue;
                }

                let hk = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid header name {}: {}",
                        k, e
                    ))
                })?;

                let hv = HeaderValue::from_str(v.extract()?).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid header value for {}: {}",
                        k, e
                    ))
                })?;

                parts.headers.insert(hk, hv);
            }
        }

        let body = if let Some(body) = req.body.as_ref() {
            let bound = body.bind(ob.py());
            Bytes::copy_from_slice(&bound.as_bytes())
        } else {
            old_body
        };

        if let Some(method) = req.method.as_ref() {
            let bound = method.bind(ob.py()).to_str()?;
            parts.method = bound.parse().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Bad method: {}", e))
            })?;
        }

        if let Some(uri) = req.uri.as_ref() {
            let bound = uri.bind(ob.py()).borrow();
            parts.uri = bound.uri.clone();
        }

        Ok(HttpRequest(Request::from_parts(parts, body)))
    }
}
