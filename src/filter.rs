use pyo3::ffi::c_str;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use std::ffi::CString;
use std::fs;

#[derive(Debug)]
pub struct Filter {
    path: CString,
    module: Py<PyModule>,
}

impl Filter {
    // TODO: Check if filters are actually there
    pub fn load_from_file(path: &str) -> anyhow::Result<Filter> {
        Python::with_gil(|py| {
            let code = CString::new(fs::read(path)?)?;
            let path = CString::new(path)?;

            let module =
                PyModule::from_code(py, code.as_c_str(), path.as_c_str(), c_str!("filter"))?;

            Ok(Filter {
                path,
                module: module.into(),
            })
        })
    }

    pub fn apply(&self, name: &str, chunk: &[u8]) -> anyhow::Result<Py<PyBytes>> {
        Python::with_gil(|py| {
            let module = self.module.as_ref();
            let func = module.getattr(py, name)?;
            let args = (PyBytes::new(py, chunk),);
            Ok(func.call1(py, args)?.extract(py)?)
        })
    }
}
