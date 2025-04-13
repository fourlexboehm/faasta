use waki::{handler, ErrorCode, Request, Response};

#[handler]
fn hello(req: Request) -> Result<Response, ErrorCode> {
    // Extract query parameters
    let query = req.query();
    let name = query.get("name").unwrap_or(&"World".to_string());
    
    // Extract path
    let path = req.uri().path();
    
    // Get headers
    let headers = req.headers();
    let user_agent = headers.get("user-agent").unwrap_or("Unknown");
    
    // Build response with HTML
    Response::builder()
        .header("content-type", "text/html")
        .body(format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>WASI HTTP Example</title>
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
        <p>This function is running on WASI Preview 2 HTTP with subdomain routing!</p>
    </div>
</body>
</html>"#,
            name, path, user_agent
        ))
        .build()
}