use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use axum::body::Body;
use bytes::Bytes;
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use dashmap::DashMap;
use faasta_types::{
    FaastaFuture, FaastaRequest, FaastaResponse, Header,
    stabby::alloc::{string::String as StableString, vec::Vec as StableVec},
};
use http::{HeaderMap, Method, Response, Uri, header::HeaderName, header::HeaderValue};
use libloading::{Library, Symbol};
use once_cell::sync::OnceCell;
use tokio::fs;
use tracing::debug;

use crate::github_auth::GitHubAuth;
use crate::kvm_guest;
use crate::metrics::Timer;
use crate::storage;

pub static SERVER: OnceCell<Arc<FaastaServer>> = OnceCell::new();

#[allow(improper_ctypes_definitions)]
type HandleRequestFn = unsafe extern "C" fn(FaastaRequest, Dir) -> FaastaFuture;

struct LoadedFunction {
    _library: Arc<Library>,
    handle_fn: HandleRequestFn,
    hits: AtomicUsize,
}

impl LoadedFunction {
    fn new(library: Arc<Library>, handle_fn: HandleRequestFn) -> Self {
        Self {
            _library: library,
            handle_fn,
            hits: AtomicUsize::new(0),
        }
    }

    fn handle(&self) -> HandleRequestFn {
        self.hits.fetch_add(1, Ordering::Relaxed);
        self.handle_fn
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }
}

pub struct FaastaServer {
    pub base_domain: String,
    pub functions_dir: PathBuf,
    cache_root: PathBuf,
    sandbox_root: PathBuf,
    pub github_auth: GitHubAuth,
    loaded: DashMap<String, Arc<LoadedFunction>>,
    max_cached_functions: usize,
}

impl FaastaServer {
    pub async fn new(base_domain: String, functions_dir: PathBuf) -> Result<Self> {
        kvm_guest::ensure_linked();

        ensure_dir(&functions_dir).await.with_context(|| {
            format!(
                "failed to create functions directory at {:?}",
                functions_dir
            )
        })?;

        let cache_root = functions_dir.join("cache");
        ensure_dir(&cache_root)
            .await
            .with_context(|| format!("failed to create cache directory at {:?}", cache_root))?;

        let sandbox_root = functions_dir.join("sandbox");
        ensure_dir(&sandbox_root)
            .await
            .with_context(|| format!("failed to create sandbox directory at {:?}", sandbox_root))?;

        let github_auth = GitHubAuth::new().await?;

        Ok(Self {
            base_domain,
            functions_dir,
            cache_root,
            sandbox_root,
            github_auth,
            loaded: DashMap::new(),
            max_cached_functions: 512,
        })
    }

    fn symbol_name(function_name: &str) -> String {
        function_symbol_name(function_name)
    }

    fn artifact_version(bytes: &[u8]) -> String {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    fn materialized_artifact_dir(&self, function_name: &str) -> PathBuf {
        self.cache_root.join(function_name)
    }

    fn materialized_artifact_path_for_version(
        &self,
        function_name: &str,
        artifact_version: &str,
    ) -> PathBuf {
        self.materialized_artifact_dir(function_name)
            .join(artifact_version)
            .join("artifact.so")
    }

    async fn materialize_artifact(
        &self,
        function_name: &str,
        artifact_bytes: &[u8],
    ) -> Result<PathBuf> {
        let artifact_version = Self::artifact_version(artifact_bytes);
        let artifact_path =
            self.materialized_artifact_path_for_version(function_name, &artifact_version);

        if artifact_path.exists() {
            return Ok(artifact_path);
        }

        let artifact_dir = artifact_path
            .parent()
            .context("materialized artifact path missing parent")?;
        ensure_dir(artifact_dir).await.with_context(|| {
            format!(
                "failed to create private artifact directory {}",
                artifact_dir.display()
            )
        })?;

        let temp_path = artifact_dir.join("artifact.so.tmp");
        fs::write(&temp_path, artifact_bytes)
            .await
            .with_context(|| format!("failed to write private artifact {}", temp_path.display()))?;
        fs::rename(&temp_path, &artifact_path)
            .await
            .with_context(|| {
                format!(
                    "failed to finalize private artifact materialization {}",
                    artifact_path.display()
                )
            })?;

        Ok(artifact_path)
    }

    async fn load_artifact_path(&self, function_name: &str) -> Result<PathBuf> {
        let artifact_bytes = storage::get_artifact(function_name)?
            .ok_or_else(|| anyhow::anyhow!("function artifact missing for {function_name}"))?;
        self.materialize_artifact(function_name, &artifact_bytes)
            .await
    }

    fn evict_if_needed(&self) {
        if self.loaded.len() <= self.max_cached_functions {
            return;
        }

        if let Some(entry) = self.loaded.iter().min_by_key(|guard| guard.value().hits()) {
            let key = entry.key().to_string();
            debug!("evicting cached function {key}");
            self.loaded.remove(&key);
        }
    }

    async fn load_function(&self, function_name: &str) -> Result<Arc<LoadedFunction>> {
        if let Some(handle) = self.loaded.get(function_name) {
            return Ok(handle.clone());
        }

        let artifact_path = self.load_artifact_path(function_name).await?;

        // Safety: the library is trusted to export the expected symbol.
        let library = unsafe {
            Library::new(&artifact_path)
                .with_context(|| format!("failed to load library {}", artifact_path.display()))?
        };
        let library = Arc::new(library);
        let symbol_name = Self::symbol_name(function_name);

        let symbol: Symbol<HandleRequestFn> = unsafe {
            library.get(symbol_name.as_bytes()).with_context(|| {
                format!(
                    "function symbol '{symbol_name}' missing in {}",
                    artifact_path.display()
                )
            })?
        };
        let handle_fn = *symbol;

        let loaded = Arc::new(LoadedFunction::new(library.clone(), handle_fn));
        self.loaded
            .insert(function_name.to_string(), loaded.clone());
        self.evict_if_needed();
        Ok(loaded)
    }

    pub async fn prepare_sandbox(&self, function_name: &str) -> Result<Dir> {
        let sandbox_path = self.sandbox_root.join(function_name);
        ensure_dir(&sandbox_path)
            .await
            .with_context(|| format!("failed to prepare sandbox for {function_name}"))?;

        Dir::open_ambient_dir(&sandbox_path, ambient_authority())
            .await
            .with_context(|| format!("failed to open sandbox dir {}", sandbox_path.display()))
    }

    pub async fn remove_from_cache(&self, function_name: &str) {
        self.loaded.remove(function_name);
        debug!("removed cached function {function_name}");
    }

    pub async fn invalidate_function(&self, function_name: &str) {
        self.remove_from_cache(function_name).await;

        let function_cache_dir = self.materialized_artifact_dir(function_name);
        match fs::remove_dir_all(&function_cache_dir).await {
            Ok(()) => {
                debug!("removed private artifact cache for {function_name}");
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                debug!(
                    "failed to remove private artifact cache {}: {}",
                    function_cache_dir.display(),
                    err
                );
            }
        }
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Body>> {
        let handle = self
            .load_function(function_name)
            .await
            .with_context(|| format!("failed to load function '{function_name}'"))?;

        let sandbox = self
            .prepare_sandbox(function_name)
            .await
            .with_context(|| format!("failed to prepare sandbox for '{function_name}'"))?;

        let _timer = Timer::new(function_name.to_string());
        let request = build_faasta_request(method, uri, headers, body);
        let future = unsafe { (handle.handle())(request, sandbox) };
        let response = future.await;
        Ok(faasta_response_to_http(response))
    }

    pub fn function_exists(&self, function_name: &str) -> bool {
        storage::artifact_exists(function_name).unwrap_or(false)
    }
}

async fn ensure_dir(path: &Path) -> Result<()> {
    match fs::create_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn build_faasta_request(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> FaastaRequest {
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

    let mut header_vec: StableVec<Header> = StableVec::new();
    for (name, value) in headers.iter() {
        let stable_name = StableString::from(name.as_str());
        let stable_value = StableString::from(value.to_str().unwrap_or(""));
        header_vec.push(Header {
            name: stable_name,
            value: stable_value,
        });
    }

    let stable_body: StableVec<u8> = body.into_iter().collect();
    let uri_string = uri.to_string();

    FaastaRequest {
        method: method_code,
        uri: StableString::from(uri_string.as_str()),
        headers: header_vec,
        body: stable_body,
    }
}

fn faasta_response_to_http(resp: FaastaResponse) -> Response<Body> {
    let body_bytes: Vec<u8> = resp.body.iter().copied().collect();
    let mut response = Response::builder()
        .status(resp.status)
        .body(Body::from(body_bytes))
        .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap());

    let headers_mut = response.headers_mut();
    for header in resp.headers.iter() {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(header.name.as_str().as_bytes()),
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

pub fn function_symbol_name(function_name: &str) -> String {
    let sanitized = function_name.replace('-', "_");
    format!("dy_{sanitized}")
}

#[cfg(test)]
mod tests {
    use super::FaastaServer;
    use std::path::PathBuf;

    #[test]
    fn materialized_artifact_path_is_versioned_and_private() {
        let server = FaastaServer {
            base_domain: "faasta.lol".to_string(),
            functions_dir: PathBuf::from("/tmp/functions"),
            cache_root: PathBuf::from("/tmp/functions/cache"),
            sandbox_root: PathBuf::from("/tmp/functions/sandbox"),
            github_auth: crate::github_auth::GitHubAuth,
            loaded: dashmap::DashMap::new(),
            max_cached_functions: 1,
        };

        let bytes = b"library-bytes";
        let version = FaastaServer::artifact_version(bytes);
        let path = server.materialized_artifact_path_for_version("demo", &version);

        assert_eq!(
            path,
            PathBuf::from(format!("/tmp/functions/cache/demo/{version}/artifact.so"))
        );
    }
}
