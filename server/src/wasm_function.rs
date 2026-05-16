use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail, ensure};
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{Credentials as S3Credentials, Region as S3Region};
use aws_sdk_s3::primitives::ByteStream;
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
use omnia_wasi_sql::{
    Connection as SqlConnection, DataType, Field, Row, SqlDefault, WasiSql, WasiSqlCtx,
    WasiSqlCtxView,
};
use redis::AsyncCommands;
use tokio_postgres::types::ToSql;
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
    keyvalue: KeyValueProvider,
    blobstore: BlobstoreProvider,
    sql: SqlProvider,
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

        let keyvalue = KeyValueProvider::from_env().await?;
        let blobstore = BlobstoreProvider::from_env().await?;
        let sql = SqlProvider::from_env().await?;

        Ok(Self {
            engine,
            linker,
            cache: DashMap::new(),
            keyvalue,
            blobstore,
            sql,
        })
    }

    pub async fn invoke(
        &self,
        function_name: &str,
        artifact_path: &Path,
        request: WasmRequest,
    ) -> Result<WasmResponse> {
        let pre = self.load(function_name, artifact_path)?;
        let tenant = TenantId::new(function_name);
        let sql = self.sql.for_tenant(&tenant).await?;
        let mut store = Store::new(
            &self.engine,
            WasmRequestState::new(
                TenantKeyValue::new(tenant.clone(), self.keyvalue.clone()),
                TenantBlobstore::new(tenant, self.blobstore.clone()),
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
}

struct WasmRequestState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    keyvalue: TenantKeyValue,
    blobstore: TenantBlobstore,
    sql: TenantSql,
}

impl WasmRequestState {
    fn new(keyvalue: TenantKeyValue, blobstore: TenantBlobstore, sql: TenantSql) -> Self {
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

#[derive(Clone, Debug)]
struct TenantId {
    namespace: String,
    hash: String,
}

impl TenantId {
    fn new(function_name: &str) -> Self {
        Self {
            namespace: format!("fn:{function_name}"),
            hash: stable_tenant_hash(function_name),
        }
    }

    fn resource_name(&self, name: &str) -> String {
        if name.is_empty() {
            format!("{}:default", self.namespace)
        } else {
            format!("{}:{name}", self.namespace)
        }
    }

    fn s3_prefix(&self) -> String {
        format!("functions/{}/blob", self.hash)
    }

    fn valkey_prefix(&self, bucket: &str) -> String {
        format!("faasta:{}:kv:{bucket}", self.hash)
    }
}

fn stable_tenant_hash(function_name: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in function_name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

fn guest_resource_name(name: &str) -> String {
    if name.is_empty() {
        "default".to_string()
    } else {
        name.to_string()
    }
}

fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[derive(Clone)]
enum KeyValueProvider {
    Memory(KeyValueDefault),
    Valkey(ValkeyKeyValue),
}

impl std::fmt::Debug for KeyValueProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => formatter.write_str("KeyValueProvider::Memory"),
            Self::Valkey(_) => formatter.write_str("KeyValueProvider::Valkey"),
        }
    }
}

impl KeyValueProvider {
    async fn from_env() -> Result<Self> {
        match env_or_default("FAASTA_KV_BACKEND", "memory").as_str() {
            "memory" => Ok(Self::Memory(KeyValueDefault::connect().await?)),
            "valkey" => Ok(Self::Valkey(ValkeyKeyValue::from_env().await?)),
            other => bail!("unsupported FAASTA_KV_BACKEND '{other}'"),
        }
    }
}

#[derive(Clone)]
enum BlobstoreProvider {
    Memory(BlobstoreDefault),
    S3(S3Blobstore),
}

impl std::fmt::Debug for BlobstoreProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => formatter.write_str("BlobstoreProvider::Memory"),
            Self::S3(_) => formatter.write_str("BlobstoreProvider::S3"),
        }
    }
}

impl BlobstoreProvider {
    async fn from_env() -> Result<Self> {
        match env_or_default("FAASTA_BLOB_BACKEND", "memory").as_str() {
            "memory" => Ok(Self::Memory(BlobstoreDefault::connect().await?)),
            "s3" => Ok(Self::S3(S3Blobstore::from_env().await?)),
            other => bail!("unsupported FAASTA_BLOB_BACKEND '{other}'"),
        }
    }
}

#[derive(Clone)]
enum SqlProvider {
    Sqlite { dir: PathBuf },
    Postgres(PostgresSqlProvider),
}

impl SqlProvider {
    async fn from_env() -> Result<Self> {
        match env_or_default("FAASTA_SQL_BACKEND", "sqlite").as_str() {
            "sqlite" => {
                let dir = PathBuf::from(env_or_default("FAASTA_WASI_SQL_DIR", "./data/wasi-sql"));
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("failed to create WASI SQL directory {dir:?}"))?;
                Ok(Self::Sqlite { dir })
            }
            "postgres" => Ok(Self::Postgres(PostgresSqlProvider::from_env().await?)),
            other => bail!("unsupported FAASTA_SQL_BACKEND '{other}'"),
        }
    }

    async fn for_tenant(&self, tenant: &TenantId) -> Result<TenantSql> {
        match self {
            Self::Sqlite { dir } => {
                let path = dir.join(format!("{}.sqlite3", tenant.hash));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create WASI SQL parent {parent:?}"))?;
                }
                let sql = SqlDefault::connect_with(omnia_wasi_sql::default_impl::ConnectOptions {
                    database: path.to_string_lossy().into_owned(),
                })
                .await
                .with_context(|| format!("failed to open tenant SQL database {}", tenant.hash))?;
                Ok(TenantSql::Sqlite(sql))
            }
            Self::Postgres(provider) => Ok(TenantSql::Postgres(provider.for_tenant(tenant).await?)),
        }
    }
}

#[derive(Clone)]
enum TenantSql {
    Sqlite(SqlDefault),
    Postgres(PostgresTenantSql),
}

impl std::fmt::Debug for TenantSql {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(_) => formatter.write_str("TenantSql::Sqlite"),
            Self::Postgres(sql) => formatter
                .debug_tuple("TenantSql::Postgres")
                .field(sql)
                .finish(),
        }
    }
}

impl WasiSqlCtx for TenantSql {
    fn open(&self, name: String) -> omnia::FutureResult<Arc<dyn SqlConnection>> {
        match self {
            Self::Sqlite(sql) => sql.open(name),
            Self::Postgres(sql) => sql.open(name),
        }
    }
}

#[derive(Clone)]
struct TenantKeyValue {
    tenant: TenantId,
    inner: KeyValueProvider,
}

impl std::fmt::Debug for TenantKeyValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantKeyValue")
            .field("tenant", &self.tenant)
            .finish_non_exhaustive()
    }
}

impl TenantKeyValue {
    fn new(tenant: TenantId, inner: KeyValueProvider) -> Self {
        Self { tenant, inner }
    }
}

impl WasiKeyValueCtx for TenantKeyValue {
    fn open_bucket(&self, identifier: String) -> omnia::FutureResult<Arc<dyn Bucket>> {
        let guest_name = guest_resource_name(&identifier);
        let host_name = self.tenant.resource_name(&identifier);
        let valkey_prefix = self.tenant.valkey_prefix(&guest_name);
        let inner = self.inner.clone();
        async move {
            match inner {
                KeyValueProvider::Memory(memory) => {
                    let bucket = memory.open_bucket(host_name).await?;
                    Ok(Arc::new(TenantBucket { guest_name, bucket }) as Arc<dyn Bucket>)
                }
                KeyValueProvider::Valkey(valkey) => Ok(Arc::new(ValkeyBucket {
                    guest_name,
                    prefix: valkey_prefix,
                    valkey,
                }) as Arc<dyn Bucket>),
            }
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
    tenant: TenantId,
    inner: BlobstoreProvider,
}

impl std::fmt::Debug for TenantBlobstore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantBlobstore")
            .field("tenant", &self.tenant)
            .finish_non_exhaustive()
    }
}

impl TenantBlobstore {
    fn new(tenant: TenantId, inner: BlobstoreProvider) -> Self {
        Self { tenant, inner }
    }

    fn host_name(&self, name: &str) -> String {
        self.tenant.resource_name(name)
    }
}

impl WasiBlobstoreCtx for TenantBlobstore {
    fn create_container(&self, name: String) -> omnia::FutureResult<Arc<dyn Container>> {
        let guest_name = guest_resource_name(&name);
        let host_name = self.host_name(&name);
        let inner = self.inner.clone();
        let tenant = self.tenant.clone();
        async move {
            match inner {
                BlobstoreProvider::Memory(memory) => {
                    let container = memory.create_container(host_name).await?;
                    Ok(Arc::new(TenantContainer {
                        guest_name,
                        container,
                    }) as Arc<dyn Container>)
                }
                BlobstoreProvider::S3(s3) => {
                    let container = S3Container::new(s3, tenant, guest_name);
                    container.ensure_marker().await?;
                    Ok(Arc::new(container) as Arc<dyn Container>)
                }
            }
        }
        .boxed()
    }

    fn get_container(&self, name: String) -> omnia::FutureResult<Arc<dyn Container>> {
        let guest_name = guest_resource_name(&name);
        let host_name = self.host_name(&name);
        let inner = self.inner.clone();
        let tenant = self.tenant.clone();
        async move {
            match inner {
                BlobstoreProvider::Memory(memory) => {
                    let container = memory.get_container(host_name).await?;
                    Ok(Arc::new(TenantContainer {
                        guest_name,
                        container,
                    }) as Arc<dyn Container>)
                }
                BlobstoreProvider::S3(s3) => {
                    let container = S3Container::new(s3, tenant, guest_name);
                    ensure!(container.exists().await?, "container not found");
                    Ok(Arc::new(container) as Arc<dyn Container>)
                }
            }
        }
        .boxed()
    }

    fn delete_container(&self, name: String) -> omnia::FutureResult<()> {
        let host_name = self.host_name(&name);
        let guest_name = guest_resource_name(&name);
        let inner = self.inner.clone();
        let tenant = self.tenant.clone();
        async move {
            match inner {
                BlobstoreProvider::Memory(memory) => memory.delete_container(host_name).await,
                BlobstoreProvider::S3(s3) => S3Container::new(s3, tenant, guest_name).clear().await,
            }
        }
        .boxed()
    }

    fn container_exists(&self, name: String) -> omnia::FutureResult<bool> {
        let host_name = self.host_name(&name);
        let guest_name = guest_resource_name(&name);
        let inner = self.inner.clone();
        let tenant = self.tenant.clone();
        async move {
            match inner {
                BlobstoreProvider::Memory(memory) => memory.container_exists(host_name).await,
                BlobstoreProvider::S3(s3) => {
                    S3Container::new(s3, tenant, guest_name).exists().await
                }
            }
        }
        .boxed()
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

#[derive(Clone)]
struct ValkeyKeyValue {
    client: redis::Client,
}

impl std::fmt::Debug for ValkeyKeyValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ValkeyKeyValue")
    }
}

impl ValkeyKeyValue {
    async fn from_env() -> Result<Self> {
        let url = env_or_default("FAASTA_KV_VALKEY_URL", "redis://127.0.0.1:6379");
        let client = redis::Client::open(url.clone())
            .with_context(|| format!("failed to create Valkey client for {url}"))?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .with_context(|| format!("failed to connect to Valkey at {url}"))?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .context("failed to ping Valkey")?;
        Ok(Self { client })
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .context("failed to open Valkey connection")
    }
}

#[derive(Clone)]
struct ValkeyBucket {
    guest_name: String,
    prefix: String,
    valkey: ValkeyKeyValue,
}

impl std::fmt::Debug for ValkeyBucket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValkeyBucket")
            .field("guest_name", &self.guest_name)
            .finish_non_exhaustive()
    }
}

impl ValkeyBucket {
    fn data_key(&self, key: &str) -> String {
        format!("{}:data:{key}", self.prefix)
    }

    fn index_key(&self) -> String {
        format!("{}:keys", self.prefix)
    }
}

impl Bucket for ValkeyBucket {
    fn name(&self) -> &'static str {
        Box::leak(self.guest_name.clone().into_boxed_str())
    }

    fn get(&self, key: String) -> omnia::FutureResult<Option<Vec<u8>>> {
        let bucket = self.clone();
        async move {
            let mut conn = bucket.valkey.connection().await?;
            let value = conn.get(bucket.data_key(&key)).await?;
            Ok(value)
        }
        .boxed()
    }

    fn set(&self, key: String, value: Vec<u8>) -> omnia::FutureResult<()> {
        let bucket = self.clone();
        async move {
            let mut conn = bucket.valkey.connection().await?;
            let _: () = conn.set(bucket.data_key(&key), value).await?;
            let _: usize = conn.sadd(bucket.index_key(), key).await?;
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, key: String) -> omnia::FutureResult<()> {
        let bucket = self.clone();
        async move {
            let mut conn = bucket.valkey.connection().await?;
            let _: usize = conn.del(bucket.data_key(&key)).await?;
            let _: usize = conn.srem(bucket.index_key(), key).await?;
            Ok(())
        }
        .boxed()
    }

    fn exists(&self, key: String) -> omnia::FutureResult<bool> {
        let bucket = self.clone();
        async move {
            let mut conn = bucket.valkey.connection().await?;
            let exists = conn.exists(bucket.data_key(&key)).await?;
            Ok(exists)
        }
        .boxed()
    }

    fn keys(&self) -> omnia::FutureResult<Vec<String>> {
        let bucket = self.clone();
        async move {
            let mut conn = bucket.valkey.connection().await?;
            let keys = conn.smembers(bucket.index_key()).await?;
            Ok(keys)
        }
        .boxed()
    }
}

#[derive(Clone)]
struct S3Blobstore {
    client: S3Client,
    bucket: String,
}

impl std::fmt::Debug for S3Blobstore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3Blobstore")
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl S3Blobstore {
    async fn from_env() -> Result<Self> {
        let endpoint = std::env::var("FAASTA_BLOB_S3_ENDPOINT")
            .context("FAASTA_BLOB_S3_ENDPOINT is required for FAASTA_BLOB_BACKEND=s3")?;
        let access_key = std::env::var("FAASTA_BLOB_S3_ACCESS_KEY")
            .context("FAASTA_BLOB_S3_ACCESS_KEY is required for FAASTA_BLOB_BACKEND=s3")?;
        let secret_key = std::env::var("FAASTA_BLOB_S3_SECRET_KEY")
            .context("FAASTA_BLOB_S3_SECRET_KEY is required for FAASTA_BLOB_BACKEND=s3")?;
        let bucket = env_or_default("FAASTA_BLOB_S3_BUCKET", "faasta");
        let region = env_or_default("FAASTA_BLOB_S3_REGION", "garage");

        let config = aws_sdk_s3::config::Builder::new()
            .endpoint_url(endpoint)
            .credentials_provider(S3Credentials::new(
                access_key, secret_key, None, None, "faasta",
            ))
            .region(S3Region::new(region))
            .force_path_style(true)
            .build();
        let client = S3Client::from_conf(config);
        client
            .head_bucket()
            .bucket(&bucket)
            .send()
            .await
            .with_context(|| format!("failed to access S3 bucket {bucket}"))?;
        Ok(Self { client, bucket })
    }
}

#[derive(Clone)]
struct S3Container {
    store: S3Blobstore,
    tenant: TenantId,
    guest_name: String,
}

impl std::fmt::Debug for S3Container {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3Container")
            .field("guest_name", &self.guest_name)
            .field("tenant", &self.tenant)
            .finish()
    }
}

impl S3Container {
    fn new(store: S3Blobstore, tenant: TenantId, guest_name: String) -> Self {
        Self {
            store,
            tenant,
            guest_name,
        }
    }

    fn prefix(&self) -> String {
        format!("{}/{}/", self.tenant.s3_prefix(), self.guest_name)
    }

    fn marker_key(&self) -> String {
        format!("{}.container", self.prefix())
    }

    fn object_key(&self, name: &str) -> String {
        format!("{}{}", self.prefix(), name.trim_start_matches('/'))
    }

    async fn ensure_marker(&self) -> Result<()> {
        self.store
            .client
            .put_object()
            .bucket(&self.store.bucket)
            .key(self.marker_key())
            .body(ByteStream::from_static(b""))
            .send()
            .await
            .context("failed to create S3 container marker")?;
        Ok(())
    }

    async fn exists(&self) -> Result<bool> {
        let listed = self
            .store
            .client
            .list_objects_v2()
            .bucket(&self.store.bucket)
            .prefix(self.prefix())
            .max_keys(1)
            .send()
            .await
            .context("failed to list S3 container")?;
        Ok(listed.key_count().unwrap_or_default() > 0)
    }

    async fn clear(&self) -> Result<()> {
        for object in self.list_objects().await? {
            self.delete_object(object).await?;
        }
        let _ = self
            .store
            .client
            .delete_object()
            .bucket(&self.store.bucket)
            .key(self.marker_key())
            .send()
            .await;
        Ok(())
    }
}

impl Container for S3Container {
    fn name(&self) -> Result<String> {
        Ok(self.guest_name.clone())
    }

    fn info(&self) -> Result<ContainerMetadata> {
        Ok(ContainerMetadata {
            name: self.guest_name.clone(),
            created_at: 0,
        })
    }

    fn get_data(&self, name: String, start: u64, end: u64) -> omnia::FutureResult<Option<Vec<u8>>> {
        let container = self.clone();
        async move {
            let mut request = container
                .store
                .client
                .get_object()
                .bucket(&container.store.bucket)
                .key(container.object_key(&name));
            if !(start == 0 && end == u64::MAX) {
                request = request.range(format!("bytes={start}-{end}"));
            }
            match request.send().await {
                Ok(output) => {
                    let bytes = output.body.collect().await?.into_bytes().to_vec();
                    Ok(Some(bytes))
                }
                Err(err) if err.to_string().contains("NoSuchKey") => Ok(None),
                Err(err) => Err(err).context("failed to read S3 object"),
            }
        }
        .boxed()
    }

    fn write_data(&self, name: String, data: Vec<u8>) -> omnia::FutureResult<()> {
        let container = self.clone();
        async move {
            container.ensure_marker().await?;
            container
                .store
                .client
                .put_object()
                .bucket(&container.store.bucket)
                .key(container.object_key(&name))
                .body(ByteStream::from(data))
                .send()
                .await
                .context("failed to write S3 object")?;
            Ok(())
        }
        .boxed()
    }

    fn list_objects(&self) -> omnia::FutureResult<Vec<String>> {
        let container = self.clone();
        async move {
            let prefix = container.prefix();
            let mut objects = Vec::new();
            let mut continuation = None;
            loop {
                let output = container
                    .store
                    .client
                    .list_objects_v2()
                    .bucket(&container.store.bucket)
                    .prefix(&prefix)
                    .set_continuation_token(continuation)
                    .send()
                    .await
                    .context("failed to list S3 objects")?;
                for object in output.contents() {
                    let Some(key) = object.key() else {
                        continue;
                    };
                    if key.ends_with(".container") {
                        continue;
                    }
                    if let Some(name) = key.strip_prefix(&prefix) {
                        objects.push(name.to_string());
                    }
                }
                if output.is_truncated().unwrap_or(false) {
                    continuation = output.next_continuation_token().map(ToString::to_string);
                } else {
                    break;
                }
            }
            Ok(objects)
        }
        .boxed()
    }

    fn delete_object(&self, name: String) -> omnia::FutureResult<()> {
        let container = self.clone();
        async move {
            container
                .store
                .client
                .delete_object()
                .bucket(&container.store.bucket)
                .key(container.object_key(&name))
                .send()
                .await
                .context("failed to delete S3 object")?;
            Ok(())
        }
        .boxed()
    }

    fn has_object(&self, name: String) -> omnia::FutureResult<bool> {
        let container = self.clone();
        async move {
            match container
                .store
                .client
                .head_object()
                .bucket(&container.store.bucket)
                .key(container.object_key(&name))
                .send()
                .await
            {
                Ok(_) => Ok(true),
                Err(err) if err.to_string().contains("NotFound") => Ok(false),
                Err(err) => Err(err).context("failed to stat S3 object"),
            }
        }
        .boxed()
    }

    fn object_info(&self, name: String) -> omnia::FutureResult<ObjectMetadata> {
        let container = self.clone();
        async move {
            let output = container
                .store
                .client
                .head_object()
                .bucket(&container.store.bucket)
                .key(container.object_key(&name))
                .send()
                .await
                .context("failed to stat S3 object")?;
            Ok(ObjectMetadata {
                name,
                container: container.guest_name,
                created_at: output
                    .last_modified()
                    .map(|dt| dt.secs().max(0) as u64)
                    .unwrap_or_default(),
                size: output.content_length().unwrap_or_default().max(0) as u64,
            })
        }
        .boxed()
    }
}

#[derive(Clone)]
struct PostgresSqlProvider {
    pool: deadpool_postgres::Pool,
}

impl PostgresSqlProvider {
    async fn from_env() -> Result<Self> {
        let dsn = std::env::var("FAASTA_SQL_POSTGRES_DSN")
            .context("FAASTA_SQL_POSTGRES_DSN is required for FAASTA_SQL_BACKEND=postgres")?;
        let config = dsn
            .parse::<tokio_postgres::Config>()
            .context("failed to parse Postgres DSN")?;
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(
                env_or_default("FAASTA_SQL_POSTGRES_POOL_SIZE", "16")
                    .parse()
                    .context("invalid FAASTA_SQL_POSTGRES_POOL_SIZE")?,
            )
            .build()
            .context("failed to build Postgres pool")?;
        pool.get()
            .await
            .context("failed to connect to Postgres")?
            .simple_query("SELECT 1")
            .await
            .context("failed to ping Postgres")?;
        Ok(Self { pool })
    }

    async fn for_tenant(&self, tenant: &TenantId) -> Result<PostgresTenantSql> {
        let schema = format!("faasta_fn_{}", tenant.hash);
        let client = self
            .pool
            .get()
            .await
            .context("failed to get Postgres client")?;
        client
            .batch_execute(&format!(
                "CREATE SCHEMA IF NOT EXISTS {};",
                quote_pg_ident(&schema)
            ))
            .await
            .with_context(|| format!("failed to prepare Postgres schema {schema}"))?;
        Ok(PostgresTenantSql {
            pool: self.pool.clone(),
            schema,
        })
    }
}

#[derive(Clone)]
struct PostgresTenantSql {
    pool: deadpool_postgres::Pool,
    schema: String,
}

impl std::fmt::Debug for PostgresTenantSql {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresTenantSql")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl WasiSqlCtx for PostgresTenantSql {
    fn open(&self, _name: String) -> omnia::FutureResult<Arc<dyn SqlConnection>> {
        let connection = PostgresTenantConnection {
            pool: self.pool.clone(),
            schema: self.schema.clone(),
        };
        async move { Ok(Arc::new(connection) as Arc<dyn SqlConnection>) }.boxed()
    }
}

#[derive(Clone, Debug)]
struct PostgresTenantConnection {
    pool: deadpool_postgres::Pool,
    schema: String,
}

impl SqlConnection for PostgresTenantConnection {
    fn query(&self, query: String, params: Vec<DataType>) -> omnia::FutureResult<Vec<Row>> {
        let connection = self.clone();
        async move {
            validate_single_statement(&query)?;
            let query = rewrite_qmark_params(&query);
            let values = postgres_params(params);
            let refs = values
                .iter()
                .map(|value| value.as_ref() as &(dyn ToSql + Sync))
                .collect::<Vec<_>>();
            let mut client = connection
                .pool
                .get()
                .await
                .context("failed to get Postgres client")?;
            let transaction = client
                .transaction()
                .await
                .context("failed to start transaction")?;
            transaction
                .batch_execute(&format!(
                    "SET LOCAL search_path TO {}, public;",
                    quote_pg_ident(&connection.schema)
                ))
                .await
                .context("failed to set tenant search_path")?;
            let rows = transaction
                .query(&query, &refs)
                .await
                .context("failed to execute Postgres query")?;
            let rows = rows
                .iter()
                .enumerate()
                .map(|(index, row)| postgres_row_to_wasi(index, row))
                .collect::<Result<Vec<_>>>()?;
            transaction
                .commit()
                .await
                .context("failed to commit query")?;
            Ok(rows)
        }
        .boxed()
    }

    fn exec(&self, query: String, params: Vec<DataType>) -> omnia::FutureResult<u32> {
        let connection = self.clone();
        async move {
            validate_single_statement(&query)?;
            let query = rewrite_qmark_params(&query);
            let values = postgres_params(params);
            let refs = values
                .iter()
                .map(|value| value.as_ref() as &(dyn ToSql + Sync))
                .collect::<Vec<_>>();
            let mut client = connection
                .pool
                .get()
                .await
                .context("failed to get Postgres client")?;
            let transaction = client
                .transaction()
                .await
                .context("failed to start transaction")?;
            transaction
                .batch_execute(&format!(
                    "SET LOCAL search_path TO {}, public;",
                    quote_pg_ident(&connection.schema)
                ))
                .await
                .context("failed to set tenant search_path")?;
            let count = transaction
                .execute(&query, &refs)
                .await
                .context("failed to execute Postgres statement")?;
            transaction
                .commit()
                .await
                .context("failed to commit statement")?;
            Ok(count.min(u64::from(u32::MAX)) as u32)
        }
        .boxed()
    }
}

fn quote_pg_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn validate_single_statement(query: &str) -> Result<()> {
    let mut in_string = false;
    let mut chars = query.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                if in_string && chars.peek() == Some(&'\'') {
                    let _ = chars.next();
                } else {
                    in_string = !in_string;
                }
            }
            ';' if !in_string && chars.any(|rest| !rest.is_whitespace()) => {
                bail!("multi-statement SQL is not allowed")
            }
            _ => {}
        }
    }
    Ok(())
}

fn rewrite_qmark_params(query: &str) -> String {
    let mut rewritten = String::with_capacity(query.len());
    let mut in_string = false;
    let mut index = 1usize;
    let mut chars = query.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                rewritten.push(ch);
                if in_string && chars.peek() == Some(&'\'') {
                    rewritten.push(chars.next().unwrap_or('\''));
                } else {
                    in_string = !in_string;
                }
            }
            '?' if !in_string => {
                rewritten.push('$');
                rewritten.push_str(&index.to_string());
                index += 1;
            }
            _ => rewritten.push(ch),
        }
    }
    rewritten
}

fn postgres_params(params: Vec<DataType>) -> Vec<Box<dyn ToSql + Sync + Send>> {
    params
        .into_iter()
        .map(|param| match param {
            DataType::Boolean(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Int32(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Int64(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Uint32(value) => {
                Box::new(value.map(i64::from)) as Box<dyn ToSql + Sync + Send>
            }
            DataType::Uint64(value) => Box::new(value.map(|v| v.min(i64::MAX as u64) as i64))
                as Box<dyn ToSql + Sync + Send>,
            DataType::Float(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Double(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Str(value)
            | DataType::Date(value)
            | DataType::Time(value)
            | DataType::Timestamp(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
            DataType::Binary(value) => Box::new(value) as Box<dyn ToSql + Sync + Send>,
        })
        .collect()
}

fn postgres_row_to_wasi(index: usize, row: &tokio_postgres::Row) -> Result<Row> {
    let fields = row
        .columns()
        .iter()
        .enumerate()
        .map(|(column_index, column)| {
            let value = match *column.type_() {
                tokio_postgres::types::Type::BOOL => {
                    DataType::Boolean(row.try_get::<_, Option<bool>>(column_index)?)
                }
                tokio_postgres::types::Type::INT2 | tokio_postgres::types::Type::INT4 => {
                    DataType::Int32(row.try_get::<_, Option<i32>>(column_index)?)
                }
                tokio_postgres::types::Type::INT8 => {
                    DataType::Int64(row.try_get::<_, Option<i64>>(column_index)?)
                }
                tokio_postgres::types::Type::FLOAT4 => {
                    DataType::Float(row.try_get::<_, Option<f32>>(column_index)?)
                }
                tokio_postgres::types::Type::FLOAT8 => {
                    DataType::Double(row.try_get::<_, Option<f64>>(column_index)?)
                }
                tokio_postgres::types::Type::BYTEA => {
                    DataType::Binary(row.try_get::<_, Option<Vec<u8>>>(column_index)?)
                }
                _ => DataType::Str(row.try_get::<_, Option<String>>(column_index)?),
            };
            Ok(Field {
                name: column.name().to_string(),
                value,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Row {
        index: index.to_string(),
        fields,
    })
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
