use spin_sdk::http::{IntoResponse, Response};
use spin_sdk::http_component;

/// A simple Spin HTTP component.
#[http_component]
fn hello_world(req: http::Request<()>) -> anyhow::Result<impl IntoResponse> {
    // Extract query parameters
    let query = req.uri().query().unwrap_or("");
    let name = query
        .split('&')
        .find_map(|pair| {
            let mut parts = pair.split('=');
            if parts.next() == Some("name") {
                parts.next()
            } else {
                None
            }
        })
        .unwrap_or("World");

    // Extract path from the URL
    let path = req.uri().path();

    // Get user agent
    let user_agent = req
        .headers()
        .get(http::header::USER_AGENT)
        .map(|h| h.to_str().unwrap_or("Unknown"))
        .unwrap_or("Unknown");

    // Build response with HTML
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Faasta HTTP Example</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; line-height: 1.6; }}
        h1 {{ color: #333; }}
        .info {{ background-color: #f5f5f5; padding: 15px; border-radius: 5px; }}
        .highlight {{ color: #0066cc; font-weight: bold; }}
    </style>
</head>
<body>
    <h1>Hello, {}!</h1>
    <div class="info">
        <p>You accessed path: <span class="highlight">{}</span></p>
        <p>Your User-Agent: <span class="highlight">{}</span></p>
        <p>This function is running on Faasta with subdomain routing!</p>
    </div>
</body>
</html>"#,
        name, path, user_agent
    );

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/html")
        .body(html)
        .build())
}
