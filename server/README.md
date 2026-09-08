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

## Deployment

The server runs as a normal process (no KVM wrapper):

```bash
faasta-server \
  --listen-addr 0.0.0.0:443 \
  --http-listen-addr 0.0.0.0:80 \
  --base-domain faasta.lol \
  --db-path ./data/db \
  --functions-path ./functions
```

For a systemd host with hourly GitHub release updates, use the units and scripts under [infra/](infra/):

```bash
sudo ./infra/setup-faasta-autoupdate.sh
```

That installs `faasta.service`, downloads the latest `build-N` `faasta-server` binary into `/opt/faasta`, and enables the updater timer.
