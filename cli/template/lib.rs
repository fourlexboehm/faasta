use std::collections::HashMap;

use cap_async_std::fs::Dir;
use faasta_macros::faasta;
use faasta_types::{FaastaRequest, FaastaResponse};
use serde::Serialize;
use serde_json::json;
use url::form_urlencoded;

#[derive(Serialize)]
struct EchoResponse {
    greeting: String,
    method: String,
    uri: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Option<String>,
}

fn method_name(code: u8) -> &'static str {
    match code {
        0 => "GET",
        1 => "POST",
        2 => "PUT",
        3 => "DELETE",
        4 => "PATCH",
        5 => "HEAD",
        6 => "OPTIONS",
        _ => "UNKNOWN",
    }
}

fn parse_query(uri: &str) -> HashMap<String, String> {
    uri.splitn(2, '?')
        .nth(1)
        .map(|query| form_urlencoded::parse(query.as_bytes()).into_owned().collect())
        .unwrap_or_default()
}

#[faasta]
pub async fn hello_world(request: FaastaRequest, _dir: Dir) -> FaastaResponse {
    let FaastaRequest {
        method,
        uri,
        headers,
        body,
    } = request;

    let uri_string = uri.as_str().to_string();
    let query = parse_query(&uri_string);
    let headers_map: HashMap<String, String> = headers
        .iter()
        .map(|header| (
            header.name.as_str().to_string(),
            header.value.as_str().to_string(),
        ))
        .collect();

    let body_text = {
        let bytes: Vec<u8> = body.iter().copied().collect();
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    };

    let payload = EchoResponse {
        greeting: format!("Hello, {}!", query.get("name").cloned().unwrap_or_else(|| "World".to_string())),
        method: method_name(method).to_string(),
        uri: uri_string,
        query,
        headers: headers_map,
        body: body_text.clone(),
    };

    FaastaResponse::new(200)
        .header("content-type", "application/json")
        .with_body(json!(payload).to_string().into_bytes())
}
