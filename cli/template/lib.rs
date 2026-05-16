use faasta::http::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HelloResponse {
    message: &'static str,
}

#[faasta::handler]
async fn handle() -> faasta::Result<Json<HelloResponse>> {
    Ok(Json(HelloResponse {
        message: "Hello from Faasta",
    }))
}
