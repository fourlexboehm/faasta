use cap_async_std::fs::Dir;
use faasta_macros::faasta;
use faasta_types::{FaastaRequest, FaastaResponse};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use url::form_urlencoded;

#[derive(Serialize)]
struct ProxyResponse {
    endpoint: String,
    upstream_status: u16,
    upstream_body: serde_json::Value,
}

fn parse_query(uri: &str) -> std::collections::HashMap<String, String> {
    uri.splitn(2, '?')
        .nth(1)
        .map(|query| form_urlencoded::parse(query.as_bytes()).into_owned().collect())
        .unwrap_or_default()
}

fn json_response(status: u16, payload: serde_json::Value) -> FaastaResponse {
    FaastaResponse::new(status)
        .header("content-type", "application/json")
        .with_body(payload.to_string().into_bytes())
}

#[faasta]
pub async fn proxy_httpbin(request: FaastaRequest, _dir: Dir) -> FaastaResponse {
    let FaastaRequest { uri, .. } = request;
    let uri_string = uri.as_str().to_string();
    let params = parse_query(&uri_string);
    let endpoint = params
        .get("endpoint")
        .cloned()
        .unwrap_or_else(|| "get".to_string());

    let url = format!("https://httpbin.org/{endpoint}");
    let client = Client::new();

    let upstream = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return json_response(
                502,
                json!({
                    "error": "failed to reach upstream",
                    "details": err.to_string(),
                }),
            );
        }
    };

    let status = upstream.status().as_u16();
    let body_json = match upstream.json::<serde_json::Value>().await {
        Ok(json) => json,
        Err(err) => {
            return json_response(
                502,
                json!({
                    "error": "failed to decode upstream response",
                    "details": err.to_string(),
                }),
            );
        }
    };

    json_response(
        200,
        json!(ProxyResponse {
            endpoint,
            upstream_status: status,
            upstream_body: body_json,
        }),
    )
}
