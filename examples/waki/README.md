
This directory contains examples demonstrating the use of the [waki](https://crates.io/crates/waki) library for both HTTP client and server applications.


The server example demonstrates a WebAssembly-based HTTP server using the waki library. 


To build the server example:

```bash
cargo faasta build 
```

The resulting `.wasm` file will be in `target/wasm32-wasi/release/http_server.wasm`.

- Path: `/external_api`
- Method: GET
- Query Parameters: `endpoint` (optional)
- Description: Makes a request to an external API (httpbin.org) and returns the response.
- Example: `GET /external_api?endpoint=ip` → Returns your IP address from httpbin.org
- Example: `GET /external_api?endpoint=user-agent` → Returns your user agent from httpbin.org
