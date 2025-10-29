use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
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
    pub metadata_db: sled::Db,
    pub base_domain: String,
    pub functions_dir: PathBuf,
    sandbox_root: PathBuf,
    pub github_auth: GitHubAuth,
    loaded: DashMap<String, Arc<LoadedFunction>>,
    max_cached_functions: usize,
}

impl FaastaServer {
    pub async fn new(
        metadata_db: sled::Db,
        base_domain: String,
        functions_dir: PathBuf,
    ) -> Result<Self> {
        kvm_guest::ensure_linked();

        if !functions_dir.exists() {
            fs::create_dir_all(&functions_dir).await.with_context(|| {
                format!(
                    "failed to create functions directory at {:?}",
                    functions_dir
                )
            })?;
        }

        let sandbox_root = functions_dir.join("sandbox");
        fs::create_dir_all(&sandbox_root)
            .await
            .with_context(|| format!("failed to create sandbox directory at {:?}", sandbox_root))?;

        let github_auth = GitHubAuth::new(metadata_db.clone()).await?;

        Ok(Self {
            metadata_db,
            base_domain,
            functions_dir,
            sandbox_root,
            github_auth,
            loaded: DashMap::new(),
            max_cached_functions: 512,
        })
    }

    pub fn artifact_path(&self, function_name: &str) -> PathBuf {
        self.functions_dir.join(format!("{function_name}.so"))
    }

    fn symbol_name(function_name: &str) -> String {
        function_symbol_name(function_name)
    }

    fn ensure_exists(path: &Path) -> Result<()> {
        if !path.exists() {
            bail!("function artifact missing at {}", path.display());
        }
        Ok(())
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

        let artifact_path = self.artifact_path(function_name);
        Self::ensure_exists(&artifact_path)?;

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
        fs::create_dir_all(&sandbox_path)
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
        self.artifact_path(function_name).exists()
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
