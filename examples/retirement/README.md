# Retirement Planner Example

This example renders an HTML retirement projection page from query parameters.

## Build

```bash
cargo faasta build
```

The default artifact path is:

`target/x86_64-unknown-linux-gnu/release/libretirement.so`

## Deploy

```bash
cargo faasta deploy
```

## Behavior

Pass query parameters to tune the projection:

- `years` (default `25`)
- `savings` (default `50000`)
- `contribution` (default `12000`)
- `return` (default `0.06`)

Example:

`GET /?years=30&return=0.07&contribution=15000`
