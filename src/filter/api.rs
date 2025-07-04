use chrono::{DateTime, Utc};
use http::Uri;
use pyo3::types::{PyBytes, PyDict, PyList, PyString};
use pyo3::{PyTraverseError, PyVisit, prelude::*};
use uuid::Uuid;

use crate::http::{HttpRequest, HttpResponse};

// TODO: Add a way to convert lazily into python object

#[pyclass(module = "proxad", name = "RawFlow", frozen, dict, freelist = 64)]
pub struct PyRawFlow {
    /// Unique id of this flow
    #[pyo3(get)]
    id: Uuid,

    /// All the bytes sent by the client so far
    #[pyo3(get)]
    client_history: Option<Py<PyBytes>>,

    /// All the bytes sent by the server so far
    #[pyo3(get)]
    server_history: Option<Py<PyBytes>>,
}

#[pymethods]
impl PyRawFlow {
    fn __str__(&self) -> String {
        format!("RawFlow(id={})", self.id)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.client_history)?;
        visit.call(&self.server_history)?;
        Ok(())
    }
}

impl PyRawFlow {
    pub fn new_empty(id: Uuid) -> Self {
        PyRawFlow {
            id,
            client_history: None,
            server_history: None,
        }
    }

    pub fn new(id: Uuid, client_history: &[u8], server_history: &[u8]) -> Self {
        Python::with_gil(|py| PyRawFlow {
            id,
            client_history: Some(PyBytes::new(py, client_history).into()),
            server_history: Some(PyBytes::new(py, server_history).into()),
        })
    }
}

#[pyclass(module = "proxad", name = "HttpFlow", frozen, dict, freelist = 64)]
pub struct PyHttpFlow {
    /// Unique id of this flow
    #[pyo3(get)]
    pub id: Uuid,

    /// Start time of this flow
    #[pyo3(get)]
    pub start_time: DateTime<Utc>,

    /// Receive time of the last request
    #[pyo3(get)]
    pub request_time: Option<DateTime<Utc>>,

    /// Receive time of the last response
    #[pyo3(get)]
    pub response_time: Option<DateTime<Utc>>,
}

#[pymethods]
impl PyHttpFlow {
    fn __str__(&self) -> String {
        format!("HttpFlow(id={})", self.id)
    }
}

impl PyHttpFlow {
    pub fn new(
        id: Uuid,
        start: DateTime<Utc>,
        last_req: Option<DateTime<Utc>>,
        last_resp: Option<DateTime<Utc>>,
    ) -> Self {
        PyHttpFlow {
            id,
            start_time: start,
            request_time: last_req,
            response_time: last_resp,
        }
    }
}

macro_rules! cached_getter {
    ($self:ident, $py:ident, $which:ident, $cache:ident, $ctor:expr) => {
        if let Some(ref v) = $self.$cache {
            Ok(v.clone_ref($py))
        } else if let Some(ref $which) = $self.$which {
            let val = $ctor;
            $self.$cache = Some(val.clone_ref($py));
            Ok(val)
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Invalid Http object",
            ))
        }
    };
}

#[pyclass(module = "proxad", name = "HttpResp", freelist = 64)]
pub struct PyHttpResp {
    pub resp: Option<HttpResponse>,
    pub raw: Option<Py<PyBytes>>,

    #[pyo3(set)]
    pub headers: Option<Py<PyDict>>,
    #[pyo3(set)]
    pub body: Option<Py<PyBytes>>,
    #[pyo3(set)]
    pub status: Option<u16>,
}

#[pymethods]
impl PyHttpResp {
    #[new]
    pub fn py_new(headers: Py<PyDict>, body: Py<PyBytes>, status: u16) -> Self {
        PyHttpResp {
            resp: None,
            raw: None,
            headers: Some(headers),
            body: Some(body),
            status: Some(status),
        }
    }

    fn __str__(self_: PyRef<'_, Self>, py: Python<'_>) -> String {
        let headers_len = self_
            .headers
            .as_ref()
            .map(|h| h.bind(py).len())
            .or_else(|| self_.resp.as_ref().map(|r| r.0.headers().len()))
            .unwrap_or(0);

        let body_len = self_
            .body
            .as_ref()
            .map(|b| b.bind(py).as_bytes().len())
            .or_else(|| self_.resp.as_ref().map(|r| r.0.body().len()))
            .unwrap_or(0);

        let status = self_
            .status
            .or_else(|| self_.resp.as_ref().map(|r| r.0.status().as_u16()))
            .unwrap_or(0);

        format!(
            "HttpResp(status={}, headers.len={}, body.len={})",
            status, headers_len, body_len
        )
    }

    /// Returns the HTTP headers as a dict[str, str]
    #[getter]
    fn get_headers(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        cached_getter!(self, py, resp, headers, {
            let headers = PyDict::new(py);
            for (name, value) in resp.0.headers().iter() {
                headers.set_item(
                    name.as_str(),
                    value.to_str().map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "Invalid header: {}",
                            e
                        ))
                    })?,
                )?;
            }
            <Py<PyDict>>::from(headers)
        })
    }

    /// Returns the body content as bytes
    #[getter]
    fn get_body(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        cached_getter!(self, py, resp, body, {
            <Py<PyBytes>>::from(PyBytes::new(py, &resp.0.body()))
        })
    }

    /// Returns the status code of this response
    #[getter]
    fn get_status(&mut self) -> PyResult<u16> {
        if let Some(status) = self.status {
            return Ok(status);
        }

        if let Some(ref resp) = self.resp {
            let status = resp.0.status().as_u16();
            self.status = Some(status);
            Ok(status)
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Invalid HttpResp object",
            ))
        }
    }

    /// Returns the raw response
    #[getter]
    fn get_raw(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        cached_getter!(self, py, resp, raw, {
            <Py<PyBytes>>::from(PyBytes::new(
                py,
                &resp.to_bytes().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid HttpResp object: {}",
                        e
                    ))
                })?,
            ))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.headers)?;
        visit.call(&self.body)?;
        visit.call(&self.raw)?;
        Ok(())
    }

    fn __clear__(&mut self) {
        self.headers = None;
        self.body = None;
        self.raw = None;
    }
}

impl PyHttpResp {
    pub fn new(resp: HttpResponse) -> Self {
        PyHttpResp {
            resp: Some(resp),
            raw: None,
            headers: None,
            body: None,
            status: None,
        }
    }
}

#[pyclass(module = "proxad", name = "HttpReq", freelist = 64)]
pub struct PyHttpReq {
    pub req: Option<HttpRequest>,
    pub raw: Option<Py<PyBytes>>,

    #[pyo3(set)]
    pub headers: Option<Py<PyDict>>,
    #[pyo3(set)]
    pub body: Option<Py<PyBytes>>,
    #[pyo3(set)]
    pub method: Option<Py<PyString>>,
    #[pyo3(set)]
    pub uri: Option<Py<PyUri>>,
}

#[pymethods]
impl PyHttpReq {
    #[new]
    pub fn py_new(
        headers: Py<PyDict>,
        body: Py<PyBytes>,
        method: Py<PyString>,
        uri: Py<PyUri>,
    ) -> Self {
        PyHttpReq {
            req: None,
            raw: None,
            headers: Some(headers),
            body: Some(body),
            method: Some(method),
            uri: Some(uri),
        }
    }

    fn __str__(self_: PyRef<'_, Self>, py: Python<'_>) -> PyResult<String> {
        let headers_len = self_
            .headers
            .as_ref()
            .map(|h| h.bind(py).len())
            .or_else(|| self_.req.as_ref().map(|r| r.0.headers().len()))
            .unwrap_or(0);

        let body_len = self_
            .body
            .as_ref()
            .map(|b| b.bind(py).as_bytes().len())
            .or_else(|| self_.req.as_ref().map(|r| r.0.body().len()))
            .unwrap_or(0);

        let method = if let Some(method) = &self_.method {
            method.bind(py).to_str().ok()
        } else if let Some(req) = &self_.req {
            Some(req.0.method().as_str())
        } else {
            None
        };

        let uri = if let Some(uri) = &self_.uri {
            let pyuri = uri.bind(py).borrow();
            Some(pyuri.uri.to_string())
        } else if let Some(req) = &self_.req {
            Some(req.0.uri().to_string())
        } else {
            None
        };

        Ok(format!(
            "HttpReq(method={}, uri={}, headers.len={}, body.len={})",
            method.unwrap_or("<invalid>"),
            uri.as_deref().unwrap_or("<invalid>"),
            headers_len,
            body_len
        ))
    }

    /// Returns the HTTP headers as a dict[str, str]
    #[getter]
    fn get_headers(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        cached_getter!(self, py, req, headers, {
            let headers = PyDict::new(py);
            for (name, value) in req.0.headers().iter() {
                headers.set_item(
                    name.as_str(),
                    value.to_str().map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "Invalid header: {}",
                            e
                        ))
                    })?,
                )?;
            }
            <Py<PyDict>>::from(headers)
        })
    }

    /// Returns the body content as bytes
    #[getter]
    fn get_body(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        cached_getter!(self, py, req, body, {
            <Py<PyBytes>>::from(PyBytes::new(py, &req.0.body()))
        })
    }

    /// Returns the method of this request
    #[getter]
    fn get_method(&mut self, py: Python<'_>) -> PyResult<Py<PyString>> {
        cached_getter!(self, py, req, method, {
            <Py<PyString>>::from(PyString::new(py, req.0.method().as_str()))
        })
    }

    /// Returns the uri of this request
    #[getter]
    fn get_uri(&mut self, py: Python<'_>) -> PyResult<Py<PyUri>> {
        cached_getter!(self, py, req, uri, {
            <Py<PyUri>>::from(Py::new(py, PyUri::new(req.0.uri().clone()))?)
        })
    }

    /// Returns the raw request
    #[getter]
    fn get_raw(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        cached_getter!(self, py, req, raw, {
            <Py<PyBytes>>::from(PyBytes::new(
                py,
                &req.to_bytes().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid HttpReq object: {}",
                        e
                    ))
                })?,
            ))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.headers)?;
        visit.call(&self.body)?;
        visit.call(&self.method)?;
        visit.call(&self.uri)?;
        visit.call(&self.raw)?;
        Ok(())
    }

    fn __clear__(&mut self) {
        self.headers = None;
        self.body = None;
        self.method = None;
        self.uri = None;
        self.raw = None;
    }
}

impl PyHttpReq {
    pub fn new(req: HttpRequest) -> Self {
        PyHttpReq {
            req: Some(req),
            raw: None,
            headers: None,
            body: None,
            method: None,
            uri: None,
        }
    }
}

#[pyclass(module = "proxad", name = "Uri", freelist = 64)]
pub struct PyUri {
    pub uri: Uri,
    params: Option<Py<PyDict>>,
}

#[pymethods]
impl PyUri {
    #[new]
    #[pyo3(text_signature = "(raw: str, /)")]
    pub fn py_new(raw: String) -> PyResult<Self> {
        Ok(PyUri {
            uri: raw.parse::<Uri>().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Bad uri: {}", e))
            })?,
            params: None,
        })
    }

    fn __repr__(self_: PyRef<'_, Self>) -> String {
        format!(
            "Uri({})",
            self_
                .uri
                .path_and_query()
                .map(|v| v.as_str())
                .unwrap_or_else(|| self_.uri.path())
        )
    }

    /// Return the original string representation of the uri
    #[getter]
    fn raw(self_: PyRef<'_, Self>) -> String {
        self_.uri.to_string()
    }

    /// Return the scheme of the uri if present
    #[getter]
    fn scheme(&self) -> Option<String> {
        self.uri.scheme_str().map(|v| v.to_string())
    }

    /// Return the authority of the uri if present
    #[getter]
    fn authority(&self) -> Option<String> {
        self.uri.authority().map(|v| v.as_str().to_string())
    }

    /// Return the host of the uri if present
    #[getter]
    fn host(&self) -> Option<String> {
        self.uri.host().map(|v| v.to_string())
    }

    /// Return the integer port of the uri if present
    #[getter]
    fn port(&self) -> Option<u16> {
        self.uri.port_u16()
    }

    /// Return the path part of the uri
    #[getter]
    fn path(&self) -> String {
        self.uri.path().to_string()
    }

    /// Return the query part of the uri if present
    #[getter]
    fn query(&self) -> String {
        self.uri.query().unwrap_or_default().to_string()
    }

    /// Return the parsed query parameters if present
    #[getter]
    fn params(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        if let Some(ref dict) = self.params {
            return Ok(dict.clone_ref(py));
        }

        let dict = PyDict::new(py);
        if let Some(query) = self.uri.query() {
            // Make a dict[str, list[str]]
            let pairs = form_urlencoded::parse(query.as_bytes());
            for (k, v) in pairs {
                if let Some(prev) = dict.get_item(&k)? {
                    let list: &Bound<PyList> = prev.downcast()?;
                    list.append(v)?;
                } else {
                    let list = PyList::empty(py);
                    list.append(v)?;
                    dict.set_item(k, list)?;
                }
            }
        }

        let dict: Py<PyDict> = dict.into();
        self.params = Some(dict.clone_ref(py));
        Ok(dict)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.params)?;
        Ok(())
    }

    fn __clear__(&mut self) {
        self.params = None;
    }
}

impl PyUri {
    pub fn new(uri: Uri) -> Self {
        PyUri { uri, params: None }
    }
}

pub fn register_proxad(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyRawFlow>()?;
    module.add_class::<PyHttpFlow>()?;
    module.add_class::<PyHttpResp>()?;
    module.add_class::<PyHttpReq>()?;
    module.add_class::<PyUri>()?;
    Ok(())
}
