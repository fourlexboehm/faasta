# Faasta WASI Capabilities Example

This component exercises the host capabilities wired into `faasta-server`:

- `wasi:keyvalue` through `omnia-sdk::StateStore`
- `wasi:sql` through `omnia-sdk::TableStore`
- `wasi:blobstore` through `omnia-sdk::BlobStore`
- `wasi:http/service` as the request entrypoint

Build with a component-aware Rust/WASI workflow, then publish the produced component to Faasta.

```sh
cargo component build --release
cargo faasta deploy --server https://faasta.lol ./target/wasm32-wasip2/release/faasta_wasi_capabilities.wasm capabilities
curl -X POST https://capabilities.faasta.lol/capabilities \
  -H 'content-type: application/json' \
  -d '{"message":"hello durable-ish wasi"}'
```

The current server wiring uses:

- in-memory key-value
- in-memory blobstore
- SQLite for SQL at `FAASTA_WASI_SQL_DATABASE`, defaulting to `./data/wasi-sql.sqlite3`

That proves the guest API surface. Durable/distributed KV and blobstore backends are the next infrastructure step.
