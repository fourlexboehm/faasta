# Faasta: The Fastest FaaS Platform in the World

Faasta is a cutting-edge Function-as-a-Service (FaaS) platform designed for exceptional speed and efficiency. With **cold start times under 1ms** and a **memory overhead of less than 1KB**, Faasta achieves unparalleled performance by:

- Compiling your code as a **dynamic library**.
- Loading it at runtime without relying on containerization.
- Enforcing **in-process isolation** between functions using static analysis to prevent memory sharing.

---

A minimal example of a faasta function is as follows:
```rust
#[faasta]
async fn handler() -> Response<Body> {
    Response::new(Body::from("Hello World"))
}
```
Your code is compiled with **strict safety requirements**, ensuring:
1.	🚫 No unsafe code
2. 	✅ Whitelisted dependencies
3. 	🚫 No std library

 In place of the standard library, Faasta grants access to Cap-Std, a capability-based standard library that provides an isolated environment to access system resources.
```rust
#[faasta]
async fn handler() -> Response<Body> {
    Response::new(Body::from("Hello World"))
}
```


⚠️ Experimental Status
Faasta is currently highly experimental. Avoid including sensitive information or credentials in your code.

⚠️ DoS Protection
Faasta’s primary focus is on ensuring safe execution of your code, with little emphasis on preventing Denial of Service (DoS) attacks at this stage.