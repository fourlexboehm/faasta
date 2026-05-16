# Faasta

Faasta is a Function-as-a-Service platform for Rust functions compiled as WASI HTTP components.

## Key Features

- Build Rust handlers as WASIp3-facing `wasi:http/service` components
- Use one application dependency: `faasta`
- Return JSON or HTML with `faasta::http::{Json, Html}`
- Inject SQL, KV, and blob storage with `Sql`, `Kv`, and `Blobs`
- Run components in-process with Wasmtime
- Self-host with Postgres, Garage/S3, and Valkey for distributed storage

## Example

```rust
use faasta::http::Html;

#[faasta::handler]
async fn handle() -> faasta::Result<Html<String>> {
    Ok(Html("<h1>Hello from Faasta</h1>".to_string()))
}
```

## Workflow

```bash
cargo faasta new my-function
cd my-function
cargo faasta build
cargo faasta deploy
```

`cargo faasta build` wraps the WASIp3 component build so application projects do not need to know the Rust target or artifact layout.

For self-hosting and storage configuration, see [server/README.md](./server/README.md) and [server/infra/capabilities.md](./server/infra/capabilities.md).
