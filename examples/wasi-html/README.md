# Faasta WASIp3 HTML Example

This example returns an HTML page from a Faasta handler:

```rust
use faasta::http::Html;

#[faasta::handler]
async fn handle() -> faasta::Result<Html<String>> {
    Ok(Html("<h1>Hello from Faasta</h1>".to_string()))
}
```

The SDK macro exports the function as WASIp3 `wasi:http/service`. Faasta tooling should build the component artifact and hide temporary Rust target details from users.
