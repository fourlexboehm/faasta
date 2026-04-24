use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, bail};
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use faasta_types::{FaastaFuture, FaastaRequest};
use libloading::{Library, Symbol};

#[path = "../worker_protocol.rs"]
mod worker_protocol;

use worker_protocol::{
    STATE_READY, STATE_RUNNING, SharedRegion, WorkerResponse, function_symbol_name,
};

#[allow(improper_ctypes_definitions)]
type HandleRequestFn = unsafe extern "C" fn(FaastaRequest, Dir) -> FaastaFuture;

#[derive(Debug)]
struct Args {
    function_name: String,
    artifact_path: PathBuf,
    sandbox_path: PathBuf,
    shm_path: PathBuf,
    ready_path: PathBuf,
    request_timeout_secs: u32,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("faasta-worker failed: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    if let Some(parent) = args.ready_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create ready dir {}", parent.display()))?;
    }
    let _ = fs::remove_file(&args.ready_path);

    let library = unsafe {
        Library::new(&args.artifact_path)
            .with_context(|| format!("failed to load library {}", args.artifact_path.display()))?
    };

    let symbol_name = function_symbol_name(&args.function_name);
    let handle_fn = unsafe {
        let symbol: Symbol<HandleRequestFn> =
            library.get(symbol_name.as_bytes()).with_context(|| {
                format!(
                    "function symbol '{symbol_name}' missing in {}",
                    args.artifact_path.display()
                )
            })?;
        *symbol
    };

    let region = SharedRegion::open(&args.shm_path)?;
    fs::write(&args.ready_path, b"ready")
        .with_context(|| format!("failed to write {}", args.ready_path.display()))?;

    supervisor_loop(
        region,
        handle_fn,
        &args.sandbox_path,
        args.request_timeout_secs,
    )
}

fn supervisor_loop(
    region: SharedRegion,
    handle_fn: HandleRequestFn,
    sandbox_path: &Path,
    request_timeout_secs: u32,
) -> Result<()> {
    let mut running = HashMap::<libc::pid_t, usize>::new();

    loop {
        reap_children(&region, &mut running);

        let mut started = false;
        for index in 0..region.slot_count() {
            let slot = region.slot_header(index);
            if slot
                .state
                .compare_exchange(
                    STATE_READY,
                    STATE_RUNNING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                continue;
            }

            let pid = unsafe { libc::fork() };
            match pid {
                -1 => {
                    region.write_error(index, "worker fork failed");
                }
                0 => child_main(region, index, handle_fn, sandbox_path, request_timeout_secs),
                pid => {
                    running.insert(pid, index);
                    started = true;
                }
            }
        }

        if !started {
            let seen = region.notify_value();
            reap_children(&region, &mut running);
            region.wait_for_work(seen);
        }
    }
}

fn child_main(
    region: SharedRegion,
    slot_index: usize,
    handle_fn: HandleRequestFn,
    sandbox_path: &Path,
    request_timeout_secs: u32,
) -> ! {
    unsafe {
        libc::alarm(request_timeout_secs);
    }

    let result = handle_request(&region, slot_index, handle_fn, sandbox_path)
        .and_then(|response| region.write_response(slot_index, &response));
    if let Err(err) = result {
        region.write_error(slot_index, &err.to_string());
    }
    process::exit(0);
}

fn handle_request(
    region: &SharedRegion,
    slot_index: usize,
    handle_fn: HandleRequestFn,
    sandbox_path: &Path,
) -> Result<WorkerResponse> {
    let request = region.read_request(slot_index)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()
        .context("failed to build worker runtime")?;

    runtime.block_on(async move {
        let sandbox = Dir::open_ambient_dir(sandbox_path, ambient_authority())
            .await
            .with_context(|| format!("failed to open sandbox dir {}", sandbox_path.display()))?;
        let future = unsafe { handle_fn(request.into_faasta(), sandbox) };
        Ok::<_, anyhow::Error>(future.await.into())
    })
}

fn reap_children(region: &SharedRegion, running: &mut HashMap<libc::pid_t, usize>) {
    loop {
        let mut status = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break;
        }
        if let Some(slot_index) = running.remove(&pid) {
            let slot = region.slot_header(slot_index);
            if slot.state.load(Ordering::Acquire) == STATE_RUNNING {
                let message = child_status_message(status);
                region.write_error(slot_index, &message);
            }
        }
    }
}

fn child_status_message(status: i32) -> String {
    if libc::WIFSIGNALED(status) {
        format!(
            "worker child terminated by signal {}",
            libc::WTERMSIG(status)
        )
    } else if libc::WIFEXITED(status) {
        format!(
            "worker child exited with status {}",
            libc::WEXITSTATUS(status)
        )
    } else {
        "worker child exited before writing a response".to_string()
    }
}

fn parse_args() -> Result<Args> {
    let mut function_name = None;
    let mut artifact_path = None;
    let mut sandbox_path = None;
    let mut shm_path = None;
    let mut ready_path = None;
    let mut request_timeout_secs = 10;

    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--function-name" => function_name = Some(next_string(&mut args, "--function-name")?),
            "--artifact-path" => artifact_path = Some(next_path(&mut args, "--artifact-path")?),
            "--sandbox-path" => sandbox_path = Some(next_path(&mut args, "--sandbox-path")?),
            "--shm-path" => shm_path = Some(next_path(&mut args, "--shm-path")?),
            "--ready-path" => ready_path = Some(next_path(&mut args, "--ready-path")?),
            "--request-timeout-secs" => {
                let value = next_string(&mut args, "--request-timeout-secs")?;
                request_timeout_secs = value
                    .parse()
                    .with_context(|| format!("invalid --request-timeout-secs value: {value}"))?;
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        function_name: function_name.context("--function-name is required")?,
        artifact_path: artifact_path.context("--artifact-path is required")?,
        sandbox_path: sandbox_path.context("--sandbox-path is required")?,
        shm_path: shm_path.context("--shm-path is required")?,
        ready_path: ready_path.context("--ready-path is required")?,
        request_timeout_secs,
    })
}

fn next_string(args: &mut impl Iterator<Item = OsString>, name: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("{name} requires a value"))?
        .into_string()
        .map_err(|_| anyhow::anyhow!("{name} must be valid UTF-8"))
}

fn next_path(args: &mut impl Iterator<Item = OsString>, name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(next_string(args, name)?))
}
