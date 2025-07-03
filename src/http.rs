use bytes::Bytes;
use http::Method;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http_body_util::combinators::BoxBody;
use hyper::{Request, Response};
use hyper_util::rt::TokioTimer;
use pyo3::types::{PyAnyMethods, PyBytes, PyDict, PyDictMethods, PyString, PyStringMethods};
use pyo3::{Bound, FromPyObject, IntoPyObject, Py, PyAny, PyErr, PyResult, Python};
use std::{io::Write, time::Duration};

use crate::config::Config;
use crate::filter::api::{PyHttpReq, PyHttpResp, PyUri};

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

        if let Some(inner) = resp.resp.take() {
            return Ok(inner);
        }

        let headers: &Bound<'py, PyDict> = resp
            .headers
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing headers"))?
            .bind(ob.py());

        let body: &[u8] = resp
            .body
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing body"))?
            .extract(ob.py())?;

        let status = resp
            .status
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing status"))?;

        let mut builder = Response::builder().status(status);
        for (k, v) in headers.iter() {
            let k: &str = k.extract()?;
            if k.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
                || k.eq_ignore_ascii_case(TRANSFER_ENCODING.as_str())
            {
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

        if let Some(inner) = req.req.take() {
            return Ok(inner);
        }

        let headers: &Bound<'py, PyDict> = req
            .headers
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing headers"))?
            .bind(ob.py());

        let body: &[u8] = req
            .body
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing body"))?
            .extract(ob.py())?;

        let method: &Bound<'py, PyString> = req
            .method
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing method"))?
            .bind(ob.py());

        let uri_obj = req
            .uri
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Missing uri"))?
            .bind(ob.py());

        let uri = uri_obj.borrow().uri.clone();
        let mut builder = Request::builder().method(method.to_str()?).uri(uri);

        for (k, v) in headers.iter() {
            let k: &str = k.extract()?;
            if k.eq_ignore_ascii_case(CONTENT_LENGTH.as_str())
                || k.eq_ignore_ascii_case(TRANSFER_ENCODING.as_str())
            {
                continue;
            }
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
