use crate::flow::history::RawHistory;
use crate::flow::{HttpFlow, RawFlow};
use anyhow::Context;
use either::Either;
use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use pyo3::ffi::c_str;
use pyo3::types::{PyBytes, PyEllipsis, PyModule};
use pyo3::{intern, prelude::*};
use std::ffi::{CStr, CString};
use std::fs;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

const INOTIFY_DEBOUNCE_TIME: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct Filter {
    pub script_path: CString,
    module: RwLock<Py<PyModule>>,
}

impl Filter {
    // TODO: Check if filters are actually there
    pub fn load_from_file(path: &str) -> anyhow::Result<Filter> {
        let path = CString::new(path)?;
        let module = Self::load_module(&path)?;

        Ok(Filter {
            script_path: path,
            module: RwLock::new(module),
        })
    }

    fn load_module(path: &CStr) -> anyhow::Result<Py<PyModule>> {
        let code = CString::new(fs::read(path.to_str()?)?)?;
        Python::with_gil(|py| {
            Ok(PyModule::from_code(py, code.as_c_str(), path, c_str!("filter"))?.into())
        })
    }

    fn apply_result(
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
                        //debug!("Modified chunk: {:?}", String::from_utf8_lossy(&bytes));
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

    pub async fn on_response(&self, flow: &mut HttpFlow) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    // TODO: Unify functions
    pub async fn on_client_chunk(&self, flow: &mut RawFlow) -> ControlFlow<()> {
        let module = self.module.read().await;
        let result: anyhow::Result<Either<Option<Py<PyBytes>>, Py<PyEllipsis>>> =
            Python::with_gil(|py| {
                let module = module.bind(py);
                let filter_history_name = intern!(py, "client_filter_history");

                if let Some(func) = module.getattr_opt(filter_history_name)? {
                    let args = (
                        flow.id,
                        PyBytes::new(py, flow.client_history.last_chunk()),
                        &flow.client_history.bytes,
                        &flow.server_history.bytes,
                    );

                    debug!(
                        "Running filter {} for flow {}",
                        filter_history_name, flow.id
                    );
                    Ok(func.call1(args)?.extract()?)
                } else {
                    let filter_name = intern!(py, "client_filter");
                    let func = module.getattr(filter_name)?;
                    let args = (flow.id, PyBytes::new(py, flow.client_history.last_chunk()));

                    debug!("Running filter {} for flow {}", filter_name, flow.id);
                    Ok(func.call1(args)?.extract()?)
                }
            });

        self.apply_result(result, &mut flow.client_history)
    }

    pub async fn on_server_chunk(&self, flow: &mut RawFlow) -> ControlFlow<()> {
        let module = self.module.read().await;
        let result: anyhow::Result<Either<Option<Py<PyBytes>>, Py<PyEllipsis>>> =
            Python::with_gil(|py| {
                let module = module.bind(py);
                let filter_history_name = intern!(py, "server_filter_history");

                if let Some(func) = module.getattr_opt(filter_history_name)? {
                    let args = (
                        flow.id,
                        PyBytes::new(py, flow.server_history.last_chunk()),
                        &flow.client_history.bytes,
                        &flow.server_history.bytes,
                    );

                    debug!(
                        "Running filter {} for flow {}",
                        filter_history_name, flow.id
                    );
                    Ok(func.call1(args)?.extract()?)
                } else {
                    let filter_name = intern!(py, "server_filter");
                    let func = module.getattr(filter_name)?;
                    let args = (flow.id, PyBytes::new(py, flow.server_history.last_chunk()));

                    debug!("Running filter {} for flow {}", filter_name, flow.id);
                    Ok(func.call1(args)?.extract()?)
                }
            });

        self.apply_result(result, &mut flow.server_history)
    }

    pub async fn on_flow_start(&self, _flow: &mut RawFlow) {}

    pub async fn on_flow_close(&self, _flow: &mut RawFlow) {}

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
                                 let mut guard = filter.module.write().await;
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
