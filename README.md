# Faasta: a Faster FaaS Platform

Faasta is a cutting-edge Function-as-a-Service (FaaS) platform designed for exceptional speed and efficiency. With **cold start times under 1ms** and a **memory overhead of less than 1KB**, Faasta delivers unparalleled performance through its modern WebAssembly architecture.

## Key Features

- Runs your code as **WebAssembly modules** using the WASI P2 standard
- Leverages **WASIHTTP** for high-performance HTTP request handling
- Provides **secure isolation** between functions through WebAssembly's sandboxed execution model
- Achieves **ultra-fast cold starts** without the overhead of traditional containerization
- **Self-hostable** with simple setup - run your own Faasta instance anywhere
- **Standards-compliant** with WASI P2 and WASIHTTP, making your functions portable
- Powered by **Wasmtime** for efficient WebAssembly execution
- Includes a **free hosted instance** at [faasta.xyz](https://faasta.xyz)

---

Your code is compiled to WebAssembly with **strict safety requirements**, ensuring:
1. 🔒 Secure execution within the WebAssembly sandbox
2. ✅ Whitelisted dependencies
3. 🌐 Standards-compliant implementation using WASI P2 and WASIHTTP

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

Build your function for WebAssembly:
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

Your function will be available at `https://your-function-name.faasta.xyz`

## WASI P2 and WASIHTTP

Faasta implements the WebAssembly System Interface (WASI) Preview 2 specification and the WASIHTTP standard to enable:

- Standardized HTTP request and response handling
- Component-based architecture for better modularity
- Consistent interface for interacting with the host system
- Portable functions that can run on any WASI P2 compatible runtime

Because Faasta uses these open standards, your functions are not locked to a specific platform and can be hosted anywhere that supports these standards.

## Self-Hosting

Faasta is fully self-hostable. You can run your own instance of the Faasta server to host your functions on your own infrastructure.

⚠️ Experimental Status
Faasta is currently experimental. There will be breaking changes that will interupt service on the faasta.xyz instance.

