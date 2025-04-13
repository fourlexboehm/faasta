# FaaStA Server WASI Development Guidelines

## Build Commands
- Build project: `cargo build`
- Build release: `cargo build --release`
- Run server: `cargo run`
- Run single test: `cargo test test_name`
- Run all tests: `cargo test`
- Lint code: `cargo clippy`
- Format code: `cargo fmt`

## Code Style
- **Imports**: Group by std lib → external crates → project modules
- **Error Handling**: Use anyhow::Result with context(); prefer ? over match/unwrap
- **Naming**: CamelCase for types/traits; snake_case for vars/functions; ALL_CAPS for constants
- **Documentation**: Use rustdoc /// comments for public items
- **Types**: Strong typing with explicit Result types; avoid unwrap() in production code
- **Formatting**: Follow rustfmt conventions with 4-space indentation
- **Concurrency**: Use async/await with proper error propagation

## Repository Structure
- `src/main.rs`: Entry point
- `examples/`: Example code showing API usage