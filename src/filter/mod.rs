pub mod api;

use anyhow::Context;
use either::Either;
use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use pyo3::ffi::c_str;
use pyo3::types::{PyBytes, PyDict, PyEllipsis, PyList, PyModule, PyString};
use pyo3::{IntoPyObjectExt, intern, prelude::*};
use std::ffi::{CStr, CString};
use std::fs;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, trace, warn};

use crate::filter::api::{PyHttpFlow, PyHttpResponse, PyRawFlow};
use crate::flow::history::RawHistory;
use crate::flow::{HttpFlow, RawFlow};
use crate::http::HttpResponse;

const INOTIFY_DEBOUNCE_TIME: Duration = Duration::from_secs(2);

const HTTP_FILTER_FUNC: &str = "http_filter";
const HTTP_OPEN_FUNC: &str = "http_open";

const RAW_CLIENT_FUNC: &str = "client_raw_filter";
const RAW_SERVER_FUNC: &str = "server_raw_filter";
const RAW_OPEN_FUNC: &str = "raw_open";

#[derive(Debug)]
pub struct Filter {
    pub script_path: CString,
    inner: RwLock<FilterModule>,
}

#[derive(Debug)]
struct FilterModule {
    module: Py<PyModule>,
    http_filter: Option<Py<PyAny>>,
    http_open: Option<Py<PyAny>>,
    raw_client: Option<Py<PyAny>>,
    raw_server: Option<Py<PyAny>>,
    raw_open: Option<Py<PyAny>>,
}

impl Filter {
    pub fn load_api() -> anyhow::Result<()> {
        Python::with_gil(|py| {
            let module = PyModule::new(py, "proxad")?;
            api::register_proxad(&module)?;

            let path_list = PyList::new(py, &[""])?;
            module.setattr("__path__", path_list)?;

            let sys_modules: Bound<PyDict> = PyModule::import(py, "sys")?
                .getattr("modules")?
                .downcast_into()
                .expect("Python exploded :(");

            sys_modules.set_item("proxad", module)?;

            Ok(())
        })
    }

    pub fn load_from_file(path: &str) -> anyhow::Result<Filter> {
        let path = CString::new(path)?;
        let module = Self::load_module(&path)?;

        Ok(Filter {
            script_path: path,
            inner: RwLock::new(module),
        })
    }

    fn load_module(path: &CStr) -> anyhow::Result<FilterModule> {
        let code = CString::new(fs::read(path.to_str()?)?)?;
        Python::with_gil(|py| {
            let module = PyModule::from_code(py, code.as_c_str(), path, c_str!("filter"))?;
            let clone = module.clone();

            let load_function = |name: &Bound<PyString>| -> PyResult<Option<Py<PyAny>>> {
                clone
                    .getattr_opt(name)?
                    .map(|bound| {
                        debug!("Loaded python function {}", name);
                        bound.into_py_any(py)
                    })
                    .transpose()
            };

            Ok(FilterModule {
                module: module.into(),
                http_filter: load_function(intern!(py, HTTP_FILTER_FUNC))?,
                http_open: load_function(intern!(py, HTTP_OPEN_FUNC))?,
                raw_client: load_function(intern!(py, RAW_CLIENT_FUNC))?,
                raw_server: load_function(intern!(py, RAW_SERVER_FUNC))?,
                raw_open: load_function(intern!(py, RAW_OPEN_FUNC))?,
            })
        })
    }

    pub async fn on_http_response(&self, flow: &mut HttpFlow) -> ControlFlow<()> {
        if let Some(ref func) = self.inner.read().await.http_filter {
            let result: anyhow::Result<Either<Option<Py<PyHttpResponse>>, Py<PyEllipsis>>> =
                Python::with_gil(|py| {
                    let req = flow
                        .history
                        .requests
                        .last()
                        .map(|(req, _)| req)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Where is the request?"))?
                        .into_pyobject(py)?;

                    let resp = flow
                        .history
                        .responses
                        .last()
                        .map(|(resp, _)| resp)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Where is the response?"))?
                        .into_pyobject(py)?;

                    debug!("Running filter {} for flow {}", HTTP_FILTER_FUNC, flow.id);
                    let args = (PyHttpFlow::new(flow.id), &req, &resp);
                    let result = func.bind(py).call1(args)?;
                    Ok(result.extract()?)
                });

            match result {
                // Kill connection on ellipses
                Ok(Either::Right(_)) => return ControlFlow::Break(()),
                // Replace last chunk on bytes
                Ok(Either::Left(Some(resp))) => {
                    Python::with_gil(|py| match resp.extract::<HttpResponse>(py) {
                        Ok(resp) => {
                            trace!("Modified response: {:?}", resp);
                            if let Some((last, _)) = flow.history.responses.last_mut() {
                                *last = resp;
                            }
                        }
                        Err(e) => warn!("Failed to convert bytes: {}", e),
                    });
                }
                // Do nothing on none
                Ok(Either::Left(None)) => (),
                Err(e) => warn!("Failed to run python filter: {}", e),
            };
        };
        ControlFlow::Continue(())
    }

    pub async fn on_http_open(&self, flow: &mut HttpFlow) -> ControlFlow<()> {
        if let Some(ref func) = self.inner.read().await.http_open {
            let result: anyhow::Result<Option<Py<PyEllipsis>>> = Python::with_gil(|py| {
                let args = (PyHttpFlow::new(flow.id),);
                debug!("Running filter {} on flow {}", HTTP_OPEN_FUNC, flow.id);
                let result = func.bind(py).call1(args)?;
                Ok(result.extract()?)
            });

            match result {
                // Kill connection on ellipses
                Ok(Some(_)) => return ControlFlow::Break(()),
                // Do nothing on none
                Ok(None) => (),
                Err(e) => warn!("Failed to run python filter: {}", e),
            }
        };
        ControlFlow::Continue(())
    }

    fn apply_raw_chunk(
        &self,
        result: anyhow::Result<Either<Option<Py<PyBytes>>, Py<PyEllipsis>>>,
        history: &mut RawHistory,
    ) -> ControlFlow<()> {
        match result {
            // Kill connection on ellipses
            Ok(Either::Right(_)) => return ControlFlow::Break(()),
            // Replace last chunk on bytes
            Ok(Either::Left(Some(bytes))) => {
                Python::with_gil(|py| match bytes.extract::<&[u8]>(py) {
                    Ok(bytes) => {
                        trace!("Modified chunk: {:?}", String::from_utf8_lossy(&bytes));
                        history.set_last_chunk(bytes);
                    }
                    Err(e) => warn!("Failed to convert bytes: {}", e),
                });
            }
            // Do nothing on none
            Ok(Either::Left(None)) => (),
            Err(e) => warn!("Failed to run python filter: {}", e),
        };
        ControlFlow::Continue(())
    }

    pub async fn on_raw_client(&self, flow: &mut RawFlow) -> ControlFlow<()> {
        if let Some(ref func) = self.inner.read().await.raw_client {
            let result: anyhow::Result<Either<Option<Py<PyBytes>>, Py<PyEllipsis>>> =
                Python::with_gil(|py| {
                    let bytes = PyBytes::new(py, flow.client_history.last_chunk());
                    let args = (
                        PyRawFlow::new(
                            flow.id,
                            &flow.client_history.bytes,
                            &flow.server_history.bytes,
                        ),
                        &bytes,
                    );

                    debug!("Running filter {} on flow {}", RAW_CLIENT_FUNC, flow.id);
                    let result = func.bind(py).call1(args)?;

                    if result.is(bytes) {
                        trace!("Python returned the original response, ignoring");
                        Ok(Either::Left(None))
                    } else {
                        Ok(result.extract()?)
                    }
                });

            self.apply_raw_chunk(result, &mut flow.client_history)
        } else {
            ControlFlow::Continue(())
        }
    }

    pub async fn on_raw_server(&self, flow: &mut RawFlow) -> ControlFlow<()> {
        if let Some(ref func) = self.inner.read().await.raw_server {
            let result: anyhow::Result<Either<Option<Py<PyBytes>>, Py<PyEllipsis>>> =
                Python::with_gil(|py| {
                    let bytes = PyBytes::new(py, flow.server_history.last_chunk());
                    let args = (
                        PyRawFlow::new(
                            flow.id,
                            &flow.client_history.bytes,
                            &flow.server_history.bytes,
                        ),
                        &bytes,
                    );

                    debug!("Running filter {} on flow {}", RAW_SERVER_FUNC, flow.id);
                    let result = func.bind(py).call1(args)?;

                    if result.is(bytes) {
                        trace!("Python returned the original response, ignoring");
                        Ok(Either::Left(None))
                    } else {
                        Ok(result.extract()?)
                    }
                });

            self.apply_raw_chunk(result, &mut flow.server_history)
        } else {
            ControlFlow::Continue(())
        }
    }

    pub async fn on_raw_open(&self, flow: &mut RawFlow) -> ControlFlow<()> {
        if let Some(ref func) = self.inner.read().await.raw_open {
            let result: anyhow::Result<Option<Py<PyEllipsis>>> = Python::with_gil(|py| {
                let args = (PyRawFlow::new_empty(flow.id),);
                debug!("Running filter {} on flow {}", RAW_OPEN_FUNC, flow.id);
                let result = func.bind(py).call1(args)?;
                Ok(result.extract()?)
            });

            match result {
                // Kill connection on ellipses
                Ok(Some(_)) => return ControlFlow::Break(()),
                // Do nothing on none
                Ok(None) => (),
                Err(e) => warn!("Failed to run python filter: {}", e),
            }
        };
        ControlFlow::Continue(())
    }

    pub async fn spawn_watcher(self: Arc<Self>) -> anyhow::Result<()> {
        let inotify = Inotify::init().context("Failed to initialize inotify")?;

        let path = self.script_path.to_str()?;

        let parent = PathBuf::from(&path)
            .parent()
            .context("Script path has no parent directory")?
            .to_path_buf();

        let basename = PathBuf::from(&path)
            .file_name()
            .context("Failed to get file name")?
            .to_os_string();

        inotify
            .watches()
            .add(&parent, WatchMask::MODIFY)
            .with_context(|| format!("Failed to watch directory {}", parent.to_string_lossy()))?;

        let filter = self.clone();
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let mut stream = inotify.into_event_stream(&mut buffer).unwrap();
            let path = filter.script_path.to_str().unwrap();

            let mut recent = false;
            loop {
                tokio::select! {
                    maybe_event = stream.next() => {
                        match maybe_event {
                            Some(Ok(event)) if event.name.as_deref().is_some_and(|name| name == basename) => {
                                info!("Detected change to python filter {}", path);
                                recent = true;
                            }
                            Some(Ok(_)) => (),
                            Some(Err(e)) => warn!("Inotify error: {}", e),
                            None => warn!("Stopping the filter watcher"),
                        }
                    }

                    _ = async {
                         if recent {
                             sleep(INOTIFY_DEBOUNCE_TIME).await;
                         } else {
                             futures::future::pending::<()>().await;
                         }
                     }, if recent => {
                         match Self::load_module(&filter.script_path) {
                             Ok(module) => {
                                 let mut guard = filter.inner.write().await;
                                 *guard = module;
                                 info!("Reloaded python filter script");
                             }
                             Err(e) => error!("Failed to reload python filter: {}", e),
                         }
                         recent = false;
                     }
                }
            }
        });

        Ok(())
    }
}
