use omnia_sdk::{BlobStore, StateStore, TableStore, anyhow};
use omnia_wasi_sql::DataType;
use serde::Serialize;
use wasip3::http::types::{ErrorCode, Fields, Request, Response};
use wasip3::{wit_bindgen, wit_future, wit_stream};

wasip3::http::service::export!(CapabilitiesHttp);

#[derive(Clone, Debug)]
struct CapabilitiesProvider;

impl StateStore for CapabilitiesProvider {}
impl TableStore for CapabilitiesProvider {}
impl BlobStore for CapabilitiesProvider {}

struct CapabilitiesHttp;

#[derive(Debug, Serialize)]
struct CapabilityResponse {
    message: String,
    previous_message: Option<String>,
    kv_roundtrip: Option<String>,
    sql_rows: usize,
    blob_bytes: usize,
    blob_objects: Vec<String>,
}

impl wasip3::exports::http::handler::Guest for CapabilitiesHttp {
    async fn handle(_request: Request) -> Result<Response, ErrorCode> {
        match run_capabilities().await {
            Ok(response) => json_response(200, &response),
            Err(err) => json_response(
                500,
                &serde_json::json!({
                    "error": err.to_string(),
                }),
            ),
        }
    }
}

async fn run_capabilities() -> anyhow::Result<CapabilityResponse> {
    let provider = CapabilitiesProvider;
    let message = "hello from faasta wasi capabilities".to_string();

    let previous_message = StateStore::get(&provider, "last-message")
        .await?
        .and_then(|bytes| String::from_utf8(bytes).ok());

    provider
        .set("last-message", message.as_bytes(), None)
        .await?;

    let kv_roundtrip = StateStore::get(&provider, "last-message")
        .await?
        .and_then(|bytes| String::from_utf8(bytes).ok());

    provider
        .exec(
            "default".to_string(),
            "CREATE TABLE IF NOT EXISTS capability_hits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message TEXT NOT NULL
            )"
            .to_string(),
            Vec::new(),
        )
        .await?;

    provider
        .exec(
            "default".to_string(),
            "INSERT INTO capability_hits(message) VALUES (?)".to_string(),
            vec![DataType::Str(Some(message.clone()))],
        )
        .await?;

    let rows = provider
        .query(
            "default".to_string(),
            "SELECT id, message FROM capability_hits ORDER BY id DESC LIMIT 10".to_string(),
            Vec::new(),
        )
        .await?;

    let container = "capability-demo";
    if !provider.container_exists(container).await? {
        provider.create_container(container).await?;
    }

    provider
        .put(container, "last-message.txt", message.as_bytes())
        .await?;

    let blob = BlobStore::get(&provider, container, "last-message.txt")
        .await?
        .unwrap_or_default();
    let blob_objects = provider.list(container).await?;

    Ok(CapabilityResponse {
        message,
        previous_message,
        kv_roundtrip,
        sql_rows: rows.len(),
        blob_bytes: blob.len(),
        blob_objects,
    })
}

fn json_response<T>(status: u16, value: &T) -> Result<Response, ErrorCode>
where
    T: Serialize,
{
    let body = serde_json::to_vec(value)
        .map_err(|err| ErrorCode::InternalError(Some(format!("serializing response: {err}"))))?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|err| ErrorCode::InternalError(Some(format!("setting header: {err:?}"))))?;
    headers
        .set("content-length", &[body.len().to_string().into_bytes()])
        .map_err(|err| ErrorCode::InternalError(Some(format!("setting header: {err:?}"))))?;

    let (mut body_tx, body_rx) = wit_stream::new();
    let (body_result_tx, body_result_rx) = wit_future::new(|| Ok(None));
    let (response, _response_result) = Response::new(headers, Some(body_rx), body_result_rx);
    response
        .set_status_code(status)
        .map_err(|()| ErrorCode::InternalError(Some("setting status code".to_string())))?;
    drop(body_result_tx);

    wit_bindgen::spawn(async move {
        let remaining = body_tx.write_all(body).await;
        assert!(remaining.is_empty());
    });

    Ok(response)
}
