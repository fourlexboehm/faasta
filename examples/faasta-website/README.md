# Faasta Website

This example is the current Faasta documentation/marketing site as a WASIp3-facing Faasta component.

It uses the public SDK surface:

```rust
use faasta::http::Html;

#[faasta::handler]
async fn handle() -> faasta::Result<Html<String>> {
    Ok(Html(page().to_string()))
}
```

The page documents the latest workflow:

- `cargo faasta new`
- `cargo faasta build`
- `cargo faasta deploy`
- `faasta::http::{Html, Json}`
- injected `Kv`, `Sql`, and `Blobs`
