# Faasta WASIp3 Capabilities Example

This component exercises the host capabilities wired into `faasta-server` using the `faasta` SDK:

- `faasta::kv::Kv`
- `faasta::sql::Sql`
- `faasta::blob::Blobs`
- `#[faasta::handler]` as the WASIp3 HTTP service entrypoint

Once the component is built by Faasta tooling, publish it to Faasta:

```sh
cargo faasta deploy --server https://faasta.lol --artifact-path ./target/faasta/faasta_wasi_capabilities.wasm --function-name capabilities
curl -X POST https://capabilities.faasta.lol/capabilities \
  -H 'content-type: application/json' \
  -d '{"message":"hello durable-ish wasi"}'
```

The guest source is WASIp3-facing. The SDK macro exports `wasi:http/service`, and Faasta tooling is responsible for hiding the current Rust compiler target details when it builds the component artifact.

The current server wiring uses:

- key-value through in-memory storage by default, or Valkey with `FAASTA_KV_BACKEND=valkey`
- blobstore through in-memory storage by default, or Garage/S3 with `FAASTA_BLOB_BACKEND=s3`
- SQL through per-function SQLite files by default, or Postgres schemas with `FAASTA_SQL_BACKEND=postgres`

The guest code stays the same across those host backends; Faasta injects the tenant-specific SQL schema, blob prefix, and KV prefix.
