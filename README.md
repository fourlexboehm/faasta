# Faasta: a Faster FaaS Platform

Faasta is a cutting-edge Function-as-a-Service (FaaS) platform designed for exceptional speed and efficiency. With **cold start times under 1ms** and a **memory overhead of less than 1KB**, Faasta runs native Linux shared libraries under strong per-request isolation.

## Key Features

- Builds your Rust handlers into **native x86_64 Linux shared libraries** (`.so`)
- Uses the `#[faasta]` macro and strongly typed request/response primitives
- Provides **secure per-request isolation** with [kvmserver](./kvmserver/README.md)
- Achieves **ultra-fast cold starts** without the overhead of traditional containerization
- **Self-hostable** with simple setup - run your own Faasta instance anywhere
- Powered by **KVM + TinyKVM** for fast reset and isolation
- Includes a **free hosted instance** at [faasta.lol](https://faasta.lol)

---

## Getting Started

Install the Faasta CLI:
```bash
cargo install cargo-faasta
```

Create a new Faasta project:
```bash
cargo faasta init
# or
cargo faasta new my-function
```

Build your function:
```bash
cargo faasta build
```

Login with your GitHub account:
```bash
cargo faasta login
```

Deploy your function:
```bash
cargo faasta deploy
```

Your function will be available at `https://your-function-name.faasta.lol`

## Runtime Model (KVM Server)

Faasta runs functions as native Linux code and isolates requests using `kvmserver`:

- The CLI builds a `.so` artifact (`x86_64-unknown-linux-gnu`)
- The server launches functions through `kvmserver`
- `kvmserver` uses TinyKVM snapshots for fast request isolation and reset

For self-hosting, see:
- [KVM server docs](./kvmserver/README.md)
- [Faasta server setup](./server/README.md)

## Self-Hosting

Faasta is fully self-hostable. The provided systemd setup runs:

- `/opt/faasta/faasta-server` (control plane / routing)
- `/opt/faasta/kvmserver` (request execution runtime)

⚠️ Experimental Status
Faasta is currently experimental. There will be breaking changes that will interupt service on the faasta.lol instance.
