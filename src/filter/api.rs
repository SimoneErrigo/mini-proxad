use bytes::Bytes;
use hyper::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyMemoryView};

#[pyfunction]
fn double(x: usize) -> usize {
    x * 2
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(double, m)?)
}
