# WASI Capabilities Infra Notes

Faasta now links `wasi:keyvalue`, `wasi:sql`, and `wasi:blobstore` into the Wasmtime component linker.

Current local backends:

- SQL: Omnia's SQLite-backed `wasi:sql` provider. Each function gets its own SQLite file under `FAASTA_WASI_SQL_DIR`; default is `./data/wasi-sql/{function}.sqlite3`.
- KV: Omnia's in-memory `wasi:keyvalue` provider, with host-side bucket names prefixed by function namespace.
- Blobstore: Omnia's in-memory `wasi:blobstore` provider, with host-side container names prefixed by function namespace.

Tenanting model:

- Guests open simple names such as `default`, `cache`, or `uploads`.
- Faasta rewrites those names before they reach the provider, using `fn:{function_name}:{resource_name}`.
- SQL does not expose a shared connection. The host creates a per-function SQLite provider before each invocation.
- Guest code does not receive tenant IDs, DB paths, bucket prefixes, or storage credentials.

Production direction for multiple Faasta servers:

- SQL: use an external managed or clustered SQL database if guests need SQL semantics. SQLite is fine for one box and local development, but it is not the right shared write backend across multiple Faasta servers.
- Blobstore: use S3-compatible object storage. Garage is a good self-hosted candidate because it is distributed and S3-compatible, but Faasta still needs a blobstore host adapter that maps `wasi:blobstore` calls onto S3/Garage APIs.
- KV: use a distributed KV/database such as FoundationDB, etcd, Redis-compatible storage, or a SQL-backed key-value table. Faasta should expose the same `wasi:keyvalue` guest API while swapping the host backend.

Recommended order:

1. Keep the Omnia default providers to prove guest compatibility.
2. Make SQL durable immediately via `FAASTA_WASI_SQL_DATABASE`.
3. Add a SQLite-backed `wasi:keyvalue` provider for one-box durability.
4. Add S3/Garage-backed `wasi:blobstore`.
5. Add distributed KV once the multi-node deployment story is concrete.
