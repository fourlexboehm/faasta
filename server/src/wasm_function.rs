use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::FutureExt;
use http::{HeaderName, HeaderValue, Method, Request, Uri};
use http_body_util::{BodyExt, Full};
use omnia::{Backend, Host};
use omnia_wasi_blobstore::{
    BlobstoreDefault, Container, ContainerMetadata, ObjectMetadata, WasiBlobstore,
    WasiBlobstoreCtx, WasiBlobstoreCtxView,
};
use omnia_wasi_keyvalue::{
    Bucket, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx, WasiKeyValueCtxView,
};
use omnia_wasi_sql::{SqlDefault, WasiSql, WasiSqlCtxView};
use tracing::debug;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p3::bindings::ServicePre;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::p3::{Request as WasiHttpRequest, WasiHttpCtxView, WasiHttpView};

#[derive(Debug, Clone)]
pub struct WireHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct WasmRequest {
    pub method: u8,
    pub uri: String,
    pub headers: Vec<WireHeader>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct WasmResponse {
    pub status: u16,
    pub headers: Vec<WireHeader>,
    pub body: Vec<u8>,
}

type RequestBody =
    http_body_util::combinators::MapErr<Full<Bytes>, fn(std::convert::Infallible) -> ErrorCode>;

pub struct WasmFunctionRuntime {
    engine: Engine,
    linker: Linker<WasmRequestState>,
    cache: DashMap<String, Arc<ServicePre<WasmRequestState>>>,
    keyvalue: KeyValueDefault,
    blobstore: BlobstoreDefault,
    sql_dir: PathBuf,
}

impl WasmFunctionRuntime {
    pub async fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.wasm_component_model_async(true);
        config.memory_init_cow(true);
        config.cranelift_opt_level(OptLevel::Speed);

        let engine = Engine::new(&config)
            .map_err(|err| anyhow!("failed to create wasmtime engine: {err}"))?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p3::add_to_linker(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI p3 imports to linker: {err}"))?;
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI p2 imports to linker: {err}"))?;
        wasmtime_wasi_http::p3::add_to_linker(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI HTTP p3 imports to linker: {err}"))?;
        <WasiKeyValue as Host<WasmRequestState>>::add_to_linker(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI keyvalue imports to linker: {err}"))?;
        <WasiBlobstore as Host<WasmRequestState>>::add_to_linker(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI blobstore imports to linker: {err}"))?;
        <WasiSql as Host<WasmRequestState>>::add_to_linker(&mut linker)
            .map_err(|err| anyhow!("failed to add WASI SQL imports to linker: {err}"))?;

        let keyvalue = KeyValueDefault::connect().await?;
        let blobstore = BlobstoreDefault::connect().await?;
        let sql_dir = PathBuf::from(
            std::env::var("FAASTA_WASI_SQL_DIR").unwrap_or_else(|_| "./data/wasi-sql".to_string()),
        );
        std::fs::create_dir_all(&sql_dir)
            .with_context(|| format!("failed to create WASI SQL directory {sql_dir:?}"))?;

        Ok(Self {
            engine,
            linker,
            cache: DashMap::new(),
            keyvalue,
            blobstore,
            sql_dir,
        })
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        artifact_path: &Path,
        request: WasmRequest,
    ) -> Result<WasmResponse> {
        let pre = self.load(function_name, artifact_path)?;
        let sql = self.sql_for_function(function_name).await?;
        let mut store = Store::new(
            &self.engine,
            WasmRequestState::new(
                TenantKeyValue::new(tenant_namespace(function_name), self.keyvalue.clone()),
                TenantBlobstore::new(tenant_namespace(function_name), self.blobstore.clone()),
                sql,
            ),
        );
        let request = build_hyper_request(request)?;
        let service = pre
            .instantiate_async(&mut store)
            .await
            .map_err(|err| anyhow!("failed to instantiate WASI HTTP service component: {err}"))?;
        let (wasi_request, request_io) = WasiHttpRequest::from_http(request);

        store
            .run_concurrent(async |accessor| {
                let response = match service.handle(accessor, wasi_request).await? {
                    Ok(response) => response,
                    Err(err) => bail!("guest returned WASI HTTP error: {err:?}"),
                };
                let response =
                    accessor.with(|store| response.into_http(store, async { Ok(()) }))?;
                let (response, ()) =
                    futures_util::try_join!(hyper_response_to_worker(response), async {
                        request_io.await.context("failed to consume request body")
                    },)?;
                Ok(response)
            })
            .await?
    }

    pub fn remove(&self, function_name: &str) {
        self.cache.remove(function_name);
    }

    fn load(
        &self,
        function_name: &str,
        artifact_path: &Path,
    ) -> Result<Arc<ServicePre<WasmRequestState>>> {
        if let Some(entry) = self.cache.get(function_name) {
            return Ok(entry.clone());
        }

        debug!(
            "compiling WASI HTTP component for {function_name} from {}",
            artifact_path.display()
        );
        let component =
            if artifact_path.extension().and_then(|ext| ext.to_str()) == Some("cwasm") {
                // SAFETY: precompiled artifacts are only loaded from the configured functions
                // directory. Wasmtime validates that the artifact matches this engine.
                unsafe { Component::deserialize_file(&self.engine, artifact_path) }
            } else {
                Component::from_file(&self.engine, artifact_path)
            }
            .map_err(|err| {
                anyhow!(
                    "failed to load component {}: {err}",
                    artifact_path.display()
                )
            })?;

        let pre =
            ServicePre::new(self.linker.instantiate_pre(&component).map_err(|err| {
                anyhow!("failed to pre-instantiate WASI HTTP p3 component: {err}")
            })?)
            .map_err(|err| anyhow!("component does not export wasi:http/service world: {err}"))?;
        let pre = Arc::new(pre);
        self.cache.insert(function_name.to_string(), pre.clone());
        Ok(pre)
    }

    async fn sql_for_function(&self, function_name: &str) -> Result<SqlDefault> {
        let path = self.sql_dir.join(format!("{function_name}.sqlite3"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create WASI SQL parent {parent:?}"))?;
        }
        SqlDefault::connect_with(omnia_wasi_sql::default_impl::ConnectOptions {
            database: path.to_string_lossy().into_owned(),
        })
        .await
        .with_context(|| format!("failed to open tenant SQL database for {function_name}"))
    }
}

struct WasmRequestState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    keyvalue: TenantKeyValue,
    blobstore: TenantBlobstore,
    sql: SqlDefault,
}

impl WasmRequestState {
    fn new(keyvalue: TenantKeyValue, blobstore: TenantBlobstore, sql: SqlDefault) -> Self {
        Self {
            wasi: WasiCtx::builder().build(),
            http: WasiHttpCtx::new(),
            table: ResourceTable::new(),
            keyvalue,
            blobstore,
            sql,
        }
    }
}

impl WasiView for WasmRequestState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for WasmRequestState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: Default::default(),
        }
    }
}

impl omnia_wasi_keyvalue::WasiKeyValueView for WasmRequestState {
    fn keyvalue(&mut self) -> WasiKeyValueCtxView<'_> {
        WasiKeyValueCtxView {
            ctx: &mut self.keyvalue,
            table: &mut self.table,
        }
    }
}

impl omnia_wasi_blobstore::WasiBlobstoreView for WasmRequestState {
    fn blobstore(&mut self) -> WasiBlobstoreCtxView<'_> {
        WasiBlobstoreCtxView {
            ctx: &mut self.blobstore,
            table: &mut self.table,
        }
    }
}

impl omnia_wasi_sql::WasiSqlView for WasmRequestState {
    fn sql(&mut self) -> WasiSqlCtxView<'_> {
        WasiSqlCtxView {
            ctx: &mut self.sql,
            table: &mut self.table,
        }
    }
}

fn tenant_namespace(function_name: &str) -> String {
    format!("fn:{function_name}")
}

fn tenant_resource_name(namespace: &str, name: &str) -> String {
    if name.is_empty() {
        format!("{namespace}:default")
    } else {
        format!("{namespace}:{name}")
    }
}

#[derive(Clone)]
struct TenantKeyValue {
    namespace: String,
    inner: KeyValueDefault,
}

impl std::fmt::Debug for TenantKeyValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantKeyValue")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl TenantKeyValue {
    fn new(namespace: String, inner: KeyValueDefault) -> Self {
        Self { namespace, inner }
    }
}

impl WasiKeyValueCtx for TenantKeyValue {
    fn open_bucket(&self, identifier: String) -> omnia::FutureResult<Arc<dyn Bucket>> {
        let guest_name = if identifier.is_empty() {
            "default".to_string()
        } else {
            identifier.clone()
        };
        let host_name = tenant_resource_name(&self.namespace, &identifier);
        let inner = self.inner.clone();
        async move {
            let bucket = inner.open_bucket(host_name).await?;
            Ok(Arc::new(TenantBucket { guest_name, bucket }) as Arc<dyn Bucket>)
        }
        .boxed()
    }
}

#[derive(Clone, Debug)]
struct TenantBucket {
    guest_name: String,
    bucket: Arc<dyn Bucket>,
}

impl Bucket for TenantBucket {
    fn name(&self) -> &'static str {
        Box::leak(self.guest_name.clone().into_boxed_str())
    }

    fn get(&self, key: String) -> omnia::FutureResult<Option<Vec<u8>>> {
        self.bucket.get(key)
    }

    fn set(&self, key: String, value: Vec<u8>) -> omnia::FutureResult<()> {
        self.bucket.set(key, value)
    }

    fn delete(&self, key: String) -> omnia::FutureResult<()> {
        self.bucket.delete(key)
    }

    fn exists(&self, key: String) -> omnia::FutureResult<bool> {
        self.bucket.exists(key)
    }

    fn keys(&self) -> omnia::FutureResult<Vec<String>> {
        self.bucket.keys()
    }
}

#[derive(Clone)]
struct TenantBlobstore {
    namespace: String,
    inner: BlobstoreDefault,
}

impl std::fmt::Debug for TenantBlobstore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantBlobstore")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl TenantBlobstore {
    fn new(namespace: String, inner: BlobstoreDefault) -> Self {
        Self { namespace, inner }
    }

    fn host_name(&self, name: &str) -> String {
        tenant_resource_name(&self.namespace, name)
    }
}

impl WasiBlobstoreCtx for TenantBlobstore {
    fn create_container(&self, name: String) -> omnia::FutureResult<Arc<dyn Container>> {
        let guest_name = if name.is_empty() {
            "default".to_string()
        } else {
            name.clone()
        };
        let host_name = self.host_name(&name);
        let inner = self.inner.clone();
        async move {
            let container = inner.create_container(host_name).await?;
            Ok(Arc::new(TenantContainer {
                guest_name,
                container,
            }) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn get_container(&self, name: String) -> omnia::FutureResult<Arc<dyn Container>> {
        let guest_name = if name.is_empty() {
            "default".to_string()
        } else {
            name.clone()
        };
        let host_name = self.host_name(&name);
        let inner = self.inner.clone();
        async move {
            let container = inner.get_container(host_name).await?;
            Ok(Arc::new(TenantContainer {
                guest_name,
                container,
            }) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn delete_container(&self, name: String) -> omnia::FutureResult<()> {
        self.inner.delete_container(self.host_name(&name))
    }

    fn container_exists(&self, name: String) -> omnia::FutureResult<bool> {
        self.inner.container_exists(self.host_name(&name))
    }
}

#[derive(Clone, Debug)]
struct TenantContainer {
    guest_name: String,
    container: Arc<dyn Container>,
}

impl Container for TenantContainer {
    fn name(&self) -> anyhow::Result<String> {
        Ok(self.guest_name.clone())
    }

    fn info(&self) -> anyhow::Result<ContainerMetadata> {
        let mut info = self.container.info()?;
        info.name = self.guest_name.clone();
        Ok(info)
    }

    fn get_data(&self, name: String, start: u64, end: u64) -> omnia::FutureResult<Option<Vec<u8>>> {
        self.container.get_data(name, start, end)
    }

    fn write_data(&self, name: String, data: Vec<u8>) -> omnia::FutureResult<()> {
        self.container.write_data(name, data)
    }

    fn list_objects(&self) -> omnia::FutureResult<Vec<String>> {
        self.container.list_objects()
    }

    fn delete_object(&self, name: String) -> omnia::FutureResult<()> {
        self.container.delete_object(name)
    }

    fn has_object(&self, name: String) -> omnia::FutureResult<bool> {
        self.container.has_object(name)
    }

    fn object_info(&self, name: String) -> omnia::FutureResult<ObjectMetadata> {
        let guest_container = self.guest_name.clone();
        let container = self.container.clone();
        async move {
            let mut info = container.object_info(name).await?;
            info.container = guest_container;
            Ok(info)
        }
        .boxed()
    }
}

fn build_hyper_request(request: WasmRequest) -> Result<Request<RequestBody>> {
    let mut builder = Request::builder()
        .method(method_from_wire(request.method))
        .uri(request.uri.parse::<Uri>().context("invalid request URI")?);

    let headers = builder
        .headers_mut()
        .context("failed to prepare request headers")?;
    for header in request.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(header.name.as_bytes()),
            HeaderValue::from_str(&header.value),
        ) {
            headers.append(name, value);
        }
    }

    builder
        .body(
            Full::new(Bytes::from(request.body))
                .map_err(infallible_to_error_code as fn(std::convert::Infallible) -> ErrorCode),
        )
        .context("failed to build request")
}

fn infallible_to_error_code(never: std::convert::Infallible) -> ErrorCode {
    match never {}
}

fn method_from_wire(method: u8) -> Method {
    match method {
        0 => Method::GET,
        1 => Method::POST,
        2 => Method::PUT,
        3 => Method::DELETE,
        4 => Method::PATCH,
        5 => Method::HEAD,
        6 => Method::OPTIONS,
        _ => Method::GET,
    }
}

async fn hyper_response_to_worker<B>(response: hyper::Response<B>) -> Result<WasmResponse>
where
    B: http_body::Body<Data = Bytes>,
    B::Error: std::fmt::Debug,
{
    let (parts, body) = response.into_parts();
    let body = body
        .collect()
        .await
        .map_err(|err| anyhow::anyhow!("failed to read WASI response body: {err:?}"))?
        .to_bytes()
        .to_vec();

    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|value| WireHeader {
                name: name.as_str().to_string(),
                value: value.to_string(),
            })
        })
        .collect();

    Ok(WasmResponse {
        status: parts.status.as_u16(),
        headers,
        body,
    })
}
