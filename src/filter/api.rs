use http::Uri;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::{PyTraverseError, PyVisit, prelude::*};
use uuid::Uuid;

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

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.client_history)?;
        visit.call(&self.server_history)?;
        Ok(())
    }
}

#[pyclass(module = "proxad", name = "HttpFlow", frozen, dict, freelist = 64)]
pub struct PyHttpFlow {
    /// Unique id of this flow
    #[pyo3(get)]
    pub id: Uuid,
}

#[pymethods]
impl PyHttpFlow {
    fn __str__(&self) -> String {
        format!("HttpFlow(id={})", self.id)
    }
}

impl PyHttpFlow {
    pub fn new(id: Uuid) -> Self {
        PyHttpFlow { id }
    }
}

#[pyclass(
    module = "proxad",
    name = "HttpPart",
    subclass,
    get_all,
    set_all,
    freelist = 64
)]
pub struct PyHttpMessage {
    /// HTTP headers as a dict[str, str]
    pub headers: Py<PyDict>,

    /// Body content as bytes
    pub body: Py<PyBytes>,
}

#[pymethods]
impl PyHttpMessage {
    #[new]
    #[pyo3(text_signature = "(headers: dict[str, str], body: bytes, /)")]
    pub fn new(headers: Py<PyDict>, body: Py<PyBytes>) -> Self {
        PyHttpMessage { headers, body }
    }

    fn __str__(&self) -> String {
        Python::with_gil(|py| {
            format!(
                "HttpMessage(headers.len={}, body.len={})",
                self.headers.bind(py).len(),
                self.body.bind(py).len().unwrap_or(0)
            )
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.headers)?;
        visit.call(&self.body)?;
        Ok(())
    }
}

#[pyclass(module = "proxad", name = "HttpResp", extends = PyHttpMessage, freelist = 64)]
pub struct PyHttpResponse {
    /// Status code (int) of this response
    #[pyo3(get, set)]
    pub status: u16,
}

#[pymethods]
impl PyHttpResponse {
    #[new]
    #[pyo3(text_signature = "(headers: dict[str, str], body: bytes, status: int, /)")]
    pub fn new(headers: Py<PyDict>, body: Py<PyBytes>, status: u16) -> PyClassInitializer<Self> {
        PyClassInitializer::from(PyHttpMessage::new(headers, body))
            .add_subclass(PyHttpResponse { status })
    }

    fn __str__(self_: PyRef<'_, Self>) -> String {
        Python::with_gil(|py| {
            format!(
                "HttpResponse(status={}, headers.len={}, body.len={})",
                self_.status,
                self_.as_super().headers.bind(py).len(),
                self_.as_super().body.bind(py).len().unwrap_or(0)
            )
        })
    }
}

#[pyclass(module = "proxad", name = "HttpReq", extends = PyHttpMessage, freelist = 64)]
pub struct PyHttpRequest {
    /// String with the HTTP method of this request
    #[pyo3(get, set)]
    pub method: String,

    /// Uri object of this request
    #[pyo3(get, set)]
    pub uri: Py<PyUri>,
}

#[pymethods]
impl PyHttpRequest {
    #[new]
    #[pyo3(
        text_signature = "(headers: dict[str, str], body: bytes, method: str, uri: proxad.Uri, /)"
    )]
    pub fn new(
        headers: Py<PyDict>,
        body: Py<PyBytes>,
        method: String,
        uri: Py<PyUri>,
    ) -> PyClassInitializer<Self> {
        PyClassInitializer::from(PyHttpMessage::new(headers, body))
            .add_subclass(PyHttpRequest { method, uri })
    }

    fn __str__(self_: PyRef<'_, Self>) -> String {
        Python::with_gil(|py| {
            format!(
                "HttpRequest(method={}, uri={}, headers.len={}, body.len={})",
                self_.method,
                self_.uri,
                self_.as_super().headers.bind(py).len(),
                self_.as_super().body.bind(py).len().unwrap_or(0)
            )
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.uri)?;
        Ok(())
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
    module.add_class::<PyHttpMessage>()?;
    module.add_class::<PyHttpResponse>()?;
    module.add_class::<PyHttpRequest>()?;
    module.add_class::<PyUri>()?;
    Ok(())
}
