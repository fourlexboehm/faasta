use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::task;
use tracing::{debug, warn};

use crate::worker_protocol::{
    DEFAULT_SLOT_BYTES, DEFAULT_SLOT_COUNT, STATE_DONE, STATE_EMPTY, STATE_ERROR, STATE_READY,
    STATE_RUNNING, STATE_WRITING, SharedRegion, WorkerRequest, WorkerResponse,
    wait_for_state_change, wake_state,
};

const WORKER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_WAIT_TIMEOUT: Duration = Duration::from_secs(12);

pub struct WorkerPool {
    worker_binary: PathBuf,
    worker_dir: PathBuf,
    handles: Mutex<HashMap<String, Arc<WorkerHandle>>>,
}

impl WorkerPool {
    pub fn new(worker_binary: PathBuf, worker_dir: PathBuf) -> Self {
        Self {
            worker_binary,
            worker_dir,
            handles: Mutex::new(HashMap::new()),
        }
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        artifact_path: &Path,
        sandbox_path: &Path,
        request: WorkerRequest,
    ) -> Result<WorkerResponse> {
        let handle = self
            .get_or_spawn(function_name, artifact_path, sandbox_path)
            .await?;

        match handle.clone().invoke(request).await {
            Ok(response) => Ok(response),
            Err(first_err) => {
                warn!("worker invocation failed for {function_name}: {first_err:#}; restarting");
                self.remove(function_name);
                let handle = self
                    .get_or_spawn(function_name, artifact_path, sandbox_path)
                    .await?;
                handle
                    .invoke(first_err.request)
                    .await
                    .map_err(|err| err.error)
            }
        }
    }

    pub fn remove(&self, function_name: &str) {
        if let Some(handle) = self
            .handles
            .lock()
            .expect("worker pool poisoned")
            .remove(function_name)
        {
            debug!("removed cached worker for {function_name}");
            drop(handle);
        }
    }

    async fn get_or_spawn(
        &self,
        function_name: &str,
        artifact_path: &Path,
        sandbox_path: &Path,
    ) -> Result<Arc<WorkerHandle>> {
        if let Some(handle) = self
            .handles
            .lock()
            .expect("worker pool poisoned")
            .get(function_name)
            .cloned()
        {
            return Ok(handle);
        }

        let handle = Arc::new(WorkerHandle::spawn(
            &self.worker_binary,
            &self.worker_dir,
            function_name,
            artifact_path,
            sandbox_path,
        )?);
        handle.wait_ready().await?;

        self.handles
            .lock()
            .expect("worker pool poisoned")
            .insert(function_name.to_string(), handle.clone());
        Ok(handle)
    }
}

struct InvocationError {
    request: WorkerRequest,
    error: anyhow::Error,
}

impl std::fmt::Debug for InvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::fmt::Display for InvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for InvocationError {}

pub struct WorkerHandle {
    shm_path: PathBuf,
    ready_path: PathBuf,
    region: SharedRegion,
    child: Mutex<Child>,
}

impl WorkerHandle {
    fn spawn(
        worker_binary: &Path,
        worker_dir: &Path,
        function_name: &str,
        artifact_path: &Path,
        sandbox_path: &Path,
    ) -> Result<Self> {
        fs::create_dir_all(worker_dir).with_context(|| {
            format!("failed to create worker directory {}", worker_dir.display())
        })?;
        let shm_path = worker_dir.join(format!("{function_name}.shm"));
        let ready_path = worker_dir.join(format!("{function_name}.ready"));
        let _ = fs::remove_file(&shm_path);
        let _ = fs::remove_file(&ready_path);

        let region = SharedRegion::create(&shm_path, DEFAULT_SLOT_COUNT, DEFAULT_SLOT_BYTES)?;

        let mut command = Command::new(worker_binary);
        command
            .arg("--function-name")
            .arg(function_name)
            .arg("--artifact-path")
            .arg(artifact_path)
            .arg("--sandbox-path")
            .arg(sandbox_path)
            .arg("--shm-path")
            .arg(&shm_path)
            .arg("--ready-path")
            .arg(&ready_path)
            .arg("--request-timeout-secs")
            .arg(REQUEST_TIMEOUT.as_secs().to_string())
            .stdin(Stdio::null());

        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn worker {}", worker_binary.display()))?;

        Ok(Self {
            shm_path,
            ready_path,
            region,
            child: Mutex::new(child),
        })
    }

    async fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + WORKER_READY_TIMEOUT;
        loop {
            self.ensure_running()?;
            if self.ready_path.exists() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "worker did not become ready at {}",
                    self.ready_path.display()
                );
            }
            tokio::time::sleep(crate::worker_protocol::READY_POLL_INTERVAL).await;
        }
    }

    async fn invoke(
        self: Arc<Self>,
        request: WorkerRequest,
    ) -> Result<WorkerResponse, InvocationError> {
        let original_request = request.clone();
        let result = task::spawn_blocking(move || self.invoke_blocking(request))
            .await
            .context("worker invocation task failed")
            .and_then(|result| result)
            .map_err(|error| InvocationError {
                request: original_request,
                error,
            })?;
        Ok(result)
    }

    fn invoke_blocking(&self, request: WorkerRequest) -> Result<WorkerResponse> {
        self.ensure_running()?;
        let slot_index = self.claim_slot(REQUEST_TIMEOUT)?;
        let slot = self.region.slot_header(slot_index);

        let result = (|| {
            self.region.encode_request(slot_index, &request)?;
            slot.generation.fetch_add(1, Ordering::Release);
            slot.state.store(STATE_READY, Ordering::Release);
            self.region.notify_worker();

            let deadline = Instant::now() + REQUEST_WAIT_TIMEOUT;
            loop {
                let state = slot.state.load(Ordering::Acquire);
                match state {
                    STATE_DONE => return self.region.read_response(slot_index),
                    STATE_ERROR => bail!("{}", self.region.read_error(slot_index)),
                    STATE_READY | STATE_RUNNING => {
                        if Instant::now() >= deadline {
                            bail!("worker request timed out");
                        }
                        wait_for_state_change(&slot.state, state, Duration::from_millis(1));
                    }
                    other => bail!("unexpected worker slot state {other}"),
                }
            }
        })();

        if matches!(slot.state.load(Ordering::Acquire), STATE_DONE | STATE_ERROR) {
            slot.state.store(STATE_EMPTY, Ordering::Release);
            wake_state(&slot.state);
        }
        result
    }

    fn claim_slot(&self, timeout: Duration) -> Result<usize> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.ensure_running()?;
            for index in 0..self.region.slot_count() {
                let slot = self.region.slot_header(index);
                if slot
                    .state
                    .compare_exchange(
                        STATE_EMPTY,
                        STATE_WRITING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return Ok(index);
                }
            }
            std::thread::yield_now();
        }
        bail!("no worker slot became available")
    }

    fn ensure_running(&self) -> Result<()> {
        let mut child = self.child.lock().expect("worker child lock poisoned");
        match child
            .try_wait()
            .context("failed to inspect worker process")?
        {
            Some(status) => bail!("worker process exited with status {status}"),
            None => Ok(()),
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let pid = child.id() as libc::pid_t;
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.shm_path);
        let _ = fs::remove_file(&self.ready_path);
    }
}
