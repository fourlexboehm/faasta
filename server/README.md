# Faasta Server

This directory contains the Faasta server. The server accepts and runs WASI HTTP component artifacts for Faasta functions.

## Runtime

- Functions are uploaded as `.wasm` WASI HTTP components.
- The server loads components with Wasmtime and invokes the WASIp3 `wasi:http/service` entrypoint.
- WASI capabilities are provided by the host and tenant-scoped per function.

## Storage Capabilities

- SQL defaults to per-function SQLite and can use Postgres for multi-node deployments.
- Blob storage defaults to memory and can use S3-compatible storage such as Garage.
- KV defaults to memory and can use Valkey.

See [infra/capabilities.md](infra/capabilities.md) for backend configuration.
