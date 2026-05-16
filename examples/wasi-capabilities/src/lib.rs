use faasta::blob::Blobs;
use faasta::http::Json;
use faasta::kv::Kv;
use faasta::sql::Sql;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct CapabilityResponse {
    message: String,
    previous_message: Option<String>,
    kv_roundtrip: Option<String>,
    sql_rows: usize,
    blob_bytes: usize,
    blob_objects: Vec<String>,
}

#[faasta::handler]
async fn handle(kv: Kv, sql: Sql, blobs: Blobs) -> faasta::Result<Json<CapabilityResponse>> {
    let message = "hello from faasta wasi capabilities".to_string();
    let cache = kv.bucket("cache");

    let previous_message = cache
        .get("last-message")
        .await?
        .and_then(|bytes| String::from_utf8(bytes).ok());

    cache.set("last-message", message.as_bytes()).await?;

    let kv_roundtrip = cache
        .get("last-message")
        .await?
        .and_then(|bytes| String::from_utf8(bytes).ok());

    sql.exec(
        "CREATE TABLE IF NOT EXISTS capability_hits (
            message TEXT NOT NULL
        )",
        (),
    )
    .await?;

    sql.exec(
        "INSERT INTO capability_hits(message) VALUES (?)",
        (message.clone(),),
    )
    .await?;

    let rows = sql
        .query("SELECT message FROM capability_hits LIMIT 10", ())
        .await?;

    let container = blobs.container("capability-demo");
    container.create_if_missing().await?;
    container
        .put("last-message.txt", message.as_bytes())
        .await?;

    let blob = container.get("last-message.txt").await?.unwrap_or_default();
    let blob_objects = container.list().await?;

    Ok(Json(CapabilityResponse {
        message,
        previous_message,
        kv_roundtrip,
        sql_rows: rows.len(),
        blob_bytes: blob.len(),
        blob_objects,
    }))
}
