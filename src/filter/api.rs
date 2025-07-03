use std::sync::Arc;

use pyo3::types::{PyBytes, PyDict};
use pyo3::{PyTraverseError, PyVisit, prelude::*};
use uuid::Uuid;

use crate::flow::history::RawHistory;

// TODO: Add a way to convert lazily into python object

#[pyclass(module = "proxad", name = "RawFlow", frozen)]
pub struct PyRawFlow {
    #[pyo3(get)]
    id: Uuid,

    #[pyo3(get)]
    client_history: Option<Py<PyBytes>>,

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
}

#[pyclass(module = "proxad", name = "HttpFlow", frozen)]
pub struct PyHttpFlow {
    pub id: Uuid,
}

#[pymethods]
impl PyHttpFlow {
    #[new]
    pub fn new(id: Uuid) -> Self {
        PyHttpFlow { id }
    }

    fn __str__(&self) -> String {
        format!("HttpFlow(id={})", self.id)
    }
}

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

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.headers)?;
        visit.call(&self.body)?;
        Ok(())
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

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.uri)?;
        Ok(())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRawFlow>()?;
    m.add_class::<PyHttpFlow>()?;
    m.add_class::<PyHttpMessage>()?;
    m.add_class::<PyHttpResponse>()?;
    m.add_class::<PyHttpRequest>()?;
    Ok(())
}
