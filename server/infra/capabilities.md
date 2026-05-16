# WASI Capabilities Infra Notes

Faasta now links `wasi:keyvalue`, `wasi:sql`, and `wasi:blobstore` into the Wasmtime component linker.

Backends:

- SQL defaults to Omnia's SQLite-backed `wasi:sql` provider. Each function gets its own SQLite file under `FAASTA_WASI_SQL_DIR`; default is `./data/wasi-sql/{tenant_hash}.sqlite3`.
- SQL can use Postgres with `FAASTA_SQL_BACKEND=postgres` and `FAASTA_SQL_POSTGRES_DSN`. Faasta creates one schema per function and sets `search_path` per operation.
- KV defaults to Omnia's in-memory `wasi:keyvalue` provider. KV can use Valkey with `FAASTA_KV_BACKEND=valkey` and `FAASTA_KV_VALKEY_URL`.
- Blobstore defaults to Omnia's in-memory `wasi:blobstore` provider. Blobstore can use Garage or another S3-compatible service with `FAASTA_BLOB_BACKEND=s3`.

Tenanting model:

- Guests open simple names such as `default`, `cache`, or `uploads`.
- Faasta rewrites those names before they reach the provider, using a stable tenant hash or `fn:{function_name}:{resource_name}`.
- SQL does not expose a shared connection. The host creates a per-function SQLite database or Postgres schema.
- Blob objects are stored below `functions/{tenant_hash}/blob/{container}/{object}` in S3.
- Valkey keys are stored below `faasta:{tenant_hash}:kv:{bucket}:{key}` with a per-bucket key index.
- Guest code does not receive tenant IDs, DB paths, bucket prefixes, or storage credentials.

Environment:

- `FAASTA_SQL_BACKEND=sqlite|postgres`
- `FAASTA_SQL_POSTGRES_DSN=postgres://...`
- `FAASTA_SQL_POSTGRES_POOL_SIZE=16`
- `FAASTA_BLOB_BACKEND=memory|s3`
- `FAASTA_BLOB_S3_ENDPOINT=http://garage:3900`
- `FAASTA_BLOB_S3_ACCESS_KEY=...`
- `FAASTA_BLOB_S3_SECRET_KEY=...`
- `FAASTA_BLOB_S3_BUCKET=faasta`
- `FAASTA_BLOB_S3_REGION=garage`
- `FAASTA_KV_BACKEND=memory|valkey`
- `FAASTA_KV_VALKEY_URL=redis://valkey:6379`

Multi-node deployment:

- Run every Faasta server with the same Postgres, Garage/S3, and Valkey configuration.
- Keep SQLite/memory only for local development and single-node testing.
- Treat Valkey as cache-first persistent state; critical transactional data belongs in SQL.
