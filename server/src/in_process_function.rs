use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use faasta_types::{FaastaFuture, FaastaRequest};
use libloading::{Library, Symbol};
use tracing::debug;

use crate::worker_protocol::{WorkerRequest, WorkerResponse, function_symbol_name};

#[allow(improper_ctypes_definitions)]
type HandleRequestFn = unsafe extern "C" fn(FaastaRequest, Dir) -> FaastaFuture;

pub struct InProcessFunctionPool {
    loaded: Mutex<HashMap<String, Arc<LoadedFunction>>>,
}

impl InProcessFunctionPool {
    pub fn new() -> Self {
        Self {
            loaded: Mutex::new(HashMap::new()),
        }
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        artifact_path: &Path,
        sandbox_path: &Path,
        request: WorkerRequest,
    ) -> Result<WorkerResponse> {
        let loaded = self.load_function(function_name, artifact_path)?;
        let sandbox = Dir::open_ambient_dir(sandbox_path, ambient_authority())
            .await
            .with_context(|| format!("failed to open sandbox dir {}", sandbox_path.display()))?;
        let future = unsafe { (loaded.handle())(request.into_faasta(), sandbox) };
        Ok(future.await.into())
    }

    pub fn remove(&self, function_name: &str) {
        self.loaded
            .lock()
            .expect("in-process function pool poisoned")
            .remove(function_name);
        debug!("removed cached in-process function {function_name}");
    }

    fn load_function(
        &self,
        function_name: &str,
        artifact_path: &Path,
    ) -> Result<Arc<LoadedFunction>> {
        if let Some(loaded) = self
            .loaded
            .lock()
            .expect("in-process function pool poisoned")
            .get(function_name)
            .cloned()
        {
            return Ok(loaded);
        }

        let library = unsafe {
            Library::new(artifact_path)
                .with_context(|| format!("failed to load library {}", artifact_path.display()))?
        };
        let symbol_name = function_symbol_name(function_name);
        let handle_fn = unsafe {
            let symbol: Symbol<HandleRequestFn> =
                library.get(symbol_name.as_bytes()).with_context(|| {
                    format!(
                        "function symbol '{symbol_name}' missing in {}",
                        artifact_path.display()
                    )
                })?;
            *symbol
        };

        let loaded = Arc::new(LoadedFunction {
            _library: library,
            handle_fn,
        });
        self.loaded
            .lock()
            .expect("in-process function pool poisoned")
            .insert(function_name.to_string(), loaded.clone());
        Ok(loaded)
    }
}

struct LoadedFunction {
    _library: Library,
    handle_fn: HandleRequestFn,
}

impl LoadedFunction {
    fn handle(&self) -> HandleRequestFn {
        self.handle_fn
    }
}
