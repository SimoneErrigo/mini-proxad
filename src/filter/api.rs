use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

#[pyclass(module = "proxad", name = "HttpMessage", subclass, get_all, set_all)]
pub struct PyHttpMessage {
    pub headers: Py<PyDict>,
    pub body: Py<PyBytes>,
}

#[pymethods]
impl PyHttpMessage {
    #[new]
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
}

#[pyclass(module = "proxad", name = "HttpResponse", extends = PyHttpMessage)]
pub struct PyHttpResponse {
    #[pyo3(get, set)]
    pub status: u16,
}

#[pymethods]
impl PyHttpResponse {
    #[new]
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

#[pyclass(module = "proxad", name = "HttpRequest", extends = PyHttpMessage)]
pub struct PyHttpRequest {
    #[pyo3(get, set)]
    pub method: String,

    #[pyo3(get, set)]
    pub uri: Py<PyBytes>,
}

#[pymethods]
impl PyHttpRequest {
    #[new]
    pub fn new(
        headers: Py<PyDict>,
        body: Py<PyBytes>,
        method: String,
        uri: Py<PyBytes>,
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
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyHttpRequest>()?;
    m.add_class::<PyHttpResponse>()?;
    m.add_class::<PyHttpMessage>()?;
    Ok(())
}
