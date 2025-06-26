use anyhow::Context;
use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use pyo3::ffi::c_str;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use std::ffi::{CStr, CString};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

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

    pub async fn apply(&self, name: &str, chunk: &[u8]) -> anyhow::Result<Py<PyBytes>> {
        let module = self.module.read().await;
        Python::with_gil(|py| {
            let func = module.getattr(py, name)?;
            let args = (PyBytes::new(py, chunk),);
            Ok(func.call1(py, args)?.extract(py)?)
        })
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
            .add(&parent, WatchMask::MODIFY | WatchMask::CREATE)
            .with_context(|| format!("Failed to watch directory {}", parent.to_string_lossy()))?;

        let filter = self.clone();
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let mut stream = inotify.into_event_stream(&mut buffer).unwrap();
            let path = filter.script_path.to_str().unwrap();

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) if event.name.as_deref().is_some_and(|name| name == basename) => {
                        info!("Detected change to python filter {}", path);

                        match Self::load_module(&filter.script_path) {
                            Ok(module) => {
                                let mut guard = filter.module.write().await;
                                *guard = module;
                                info!("Reloaded python filter script");
                            }
                            Err(e) => error!("Failed to reload python filter: {}", e),
                        }
                    }
                    Ok(_) => (),
                    Err(e) => warn!("Inotify error: {}", e),
                }
            }
        });

        Ok(())
    }
}
