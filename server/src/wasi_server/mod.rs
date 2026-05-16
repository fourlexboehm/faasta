use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::body::Body;
use bytes::Bytes;
use http::{HeaderMap, Method, Response, Uri, header::HeaderName, header::HeaderValue};
use once_cell::sync::OnceCell;
use tracing::debug;

use crate::db::Database;
use crate::github_auth::GitHubAuth;
use crate::metrics::Timer;
use crate::wasm_function::{WasmFunctionRuntime, WasmRequest, WasmResponse, WireHeader};

pub static SERVER: OnceCell<Arc<FaastaServer>> = OnceCell::new();

pub struct FaastaServer {
    pub metadata_db: Arc<Database>,
    pub base_domain: String,
    pub functions_dir: PathBuf,
    sandbox_root: PathBuf,
    pub github_auth: GitHubAuth,
    invoker: FunctionInvoker,
}

impl FaastaServer {
    pub async fn new(
        metadata_db: Arc<Database>,
        base_domain: String,
        functions_dir: PathBuf,
        invoker: FunctionInvoker,
    ) -> Result<Self> {
        if !functions_dir.exists() {
            std::fs::create_dir_all(&functions_dir).with_context(|| {
                format!(
                    "failed to create functions directory at {:?}",
                    functions_dir
                )
            })?;
        }

        let sandbox_root = functions_dir.join("sandbox");
        std::fs::create_dir_all(&sandbox_root)
            .with_context(|| format!("failed to create sandbox directory at {:?}", sandbox_root))?;

        let github_auth = GitHubAuth::new(metadata_db.clone()).await?;

        Ok(Self {
            metadata_db,
            base_domain,
            functions_dir,
            sandbox_root,
            github_auth,
            invoker,
        })
    }

    pub fn artifact_path(&self, function_name: &str) -> PathBuf {
        let wasm = self.functions_dir.join(format!("{function_name}.wasm"));
        if wasm.exists() {
            wasm
        } else {
            self.functions_dir.join(format!("{function_name}.cwasm"))
        }
    }

    fn ensure_exists(path: &Path) -> Result<()> {
        if !path.exists() {
            bail!("function artifact missing at {}", path.display());
        }
        Ok(())
    }

    pub async fn prepare_sandbox_path(&self, function_name: &str) -> Result<PathBuf> {
        let sandbox_path = self.sandbox_root.join(function_name);
        std::fs::create_dir_all(&sandbox_path)
            .with_context(|| format!("failed to prepare sandbox for {function_name}"))?;
        Ok(sandbox_path)
    }

    pub async fn remove_from_cache(&self, function_name: &str) {
        self.invoker.remove(function_name);
        debug!("removed cached function runtime state {function_name}");
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Body>> {
        let artifact_path = self.artifact_path(function_name);
        Self::ensure_exists(&artifact_path)?;

        let _sandbox_path = self
            .prepare_sandbox_path(function_name)
            .await
            .with_context(|| format!("failed to prepare sandbox for '{function_name}'"))?;

        let _timer = Timer::new(function_name.to_string());
        let request = build_faasta_request(method, uri, headers, body);
        let response = self
            .invoker
            .invoke(function_name, &artifact_path, request)
            .await
            .with_context(|| format!("worker failed for function '{function_name}'"))?;
        Ok(faasta_response_to_http(response))
    }

    pub fn function_exists(&self, function_name: &str) -> bool {
        self.artifact_path(function_name).exists()
    }
}

pub struct FunctionInvoker {
    runtime: WasmFunctionRuntime,
}

impl FunctionInvoker {
    pub async fn wasm() -> Result<Self> {
        Ok(Self {
            runtime: WasmFunctionRuntime::new().await?,
        })
    }

    async fn invoke(
        &self,
        function_name: &str,
        artifact_path: &Path,
        request: WasmRequest,
    ) -> Result<WasmResponse> {
        self.runtime
            .invoke(function_name, artifact_path, request)
            .await
    }

    fn remove(&self, function_name: &str) {
        self.runtime.remove(function_name);
    }
}

fn build_faasta_request(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> WasmRequest {
    let method_code = match method {
        Method::GET => 0,
        Method::POST => 1,
        Method::PUT => 2,
        Method::DELETE => 3,
        Method::PATCH => 4,
        Method::HEAD => 5,
        Method::OPTIONS => 6,
        _ => 0,
    };

    let mut header_vec = Vec::new();
    for (name, value) in headers.iter() {
        header_vec.push(WireHeader {
            name: name.as_str().to_string(),
            value: value.to_str().unwrap_or("").to_string(),
        });
    }

    let uri_string = uri.to_string();

    WasmRequest {
        method: method_code,
        uri: uri_string,
        headers: header_vec,
        body: body.to_vec(),
    }
}

fn faasta_response_to_http(resp: WasmResponse) -> Response<Body> {
    let mut response = Response::builder()
        .status(resp.status)
        .body(Body::from(resp.body))
        .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap());

    let headers_mut = response.headers_mut();
    for header in resp.headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(header.name.as_bytes()),
            HeaderValue::from_str(header.value.as_str()),
        ) {
            headers_mut.append(name, val);
        }
    }

    response
}

pub fn resolve_function_name(host: Option<&str>, path: &str, base_domain: &str) -> Option<String> {
    if let Some(host) = host {
        let host = host.split(':').next().unwrap_or(host);
        if host.ends_with(base_domain) {
            let parts = host.split('.').collect::<Vec<_>>();
            if parts.len() > 2 {
                let name = parts[0];
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }

    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        trimmed.split('/').next().map(|s| s.to_string())
    }
}

pub fn sanitize_function_name(function_name: &str) -> Option<String> {
    let valid = function_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Some(function_name.to_string())
    } else {
        None
    }
}
