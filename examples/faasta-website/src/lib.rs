use maud::{html, Markup, DOCTYPE};
use spin_sdk::http::{IntoResponse, Response};
use spin_sdk::http_component;

/// Faasta website HTTP component
#[http_component]
fn serve_website(req: http::Request<()>) -> anyhow::Result<impl IntoResponse> {
    // Get the requested path
    let path = req.uri().path();
    
    // Serve appropriate content based on path
    match path {
        "/" | "/index.html" => serve_home_page(),
        "/docs" => serve_docs_page(),
        "/getting-started" => serve_getting_started_page(),
        _ => serve_not_found_page(),
    }
}

/// Serve the home page
fn serve_home_page() -> anyhow::Result<Response> {
    let markup = layout(
        "Faasta - Ultra-Fast FaaS Platform",
        html! {
            section.hero {
                div.container {
                    h1 { "Function-as-a-Service at Lightning Speed" }
                    p.subtitle { "Cold start times under 1ms. Memory overhead less than 1KB." }
                    div.cta-buttons {
                        a.btn.primary href="/getting-started" { "Get Started" }
                        a.btn.secondary href="/docs" { "Documentation" }
                    }
                    div.stats {
                        div.stat {
                            div.stat-value { "<1ms" }
                            div.stat-label { "Cold Start" }
                        }
                        div.stat {
                            div.stat-value { "<1KB" }
                            div.stat-label { "Memory Overhead" }
                        }
                        div.stat {
                            div.stat-value { "WASI P2" }
                            div.stat-label { "Standard" }
                        }
                    }
                }
            }

            section.features {
                div.container {
                    h2 { "Key Features" }
                    div.feature-grid {
                        div.feature {
                            div.feature-icon { "⚡" }
                            h3 { "WebAssembly Powered" }
                            p { "Run your code as WebAssembly modules using the WASI P2 standard for maximum performance." }
                        }
                        div.feature {
                            div.feature-icon { "🔒" }
                            h3 { "Secure Isolation" }
                            p { "Functions run in WebAssembly's sandboxed execution model for strong security guarantees." }
                        }
                        div.feature {
                            div.feature-icon { "🚀" }
                            h3 { "Ultra-Fast Cold Starts" }
                            p { "Start your functions in under 1ms without the overhead of traditional containerization." }
                        }
                        div.feature {
                            div.feature-icon { "🏠" }
                            h3 { "Self-Hostable" }
                            p { "Run your own Faasta instance anywhere with simple setup instructions." }
                        }
                        div.feature {
                            div.feature-icon { "📊" }
                            h3 { "Minimal Overhead" }
                            p { "Less than 1KB memory overhead per function, optimized for efficiency." }
                        }
                        div.feature {
                            div.feature-icon { "🌐" }
                            h3 { "Standards Compliant" }
                            p { "Based on WASI Preview 2 and wasi-http, making your functions portable across platforms." }
                        }
                    }
                }
            }

            section.how-it-works {
                div.container {
                    h2 { "How Faasta Works" }
                    div.steps {
                        div.step {
                            div.step-number { "1" }
                            h3 { "Write Your Function" }
                            p { "Create your function using Rust with the WASI HTTP standard" }
                        }
                        div.step {
                            div.step-number { "2" }
                            h3 { "Build For WebAssembly" }
                            p { "Compile your code to WebAssembly using " code { "cargo faasta build" } }
                        }
                        div.step {
                            div.step-number { "3" }
                            h3 { "Deploy" }
                            p { "Deploy to Faasta with " code { "cargo faasta deploy" } }
                        }
                        div.step {
                            div.step-number { "4" }
                            h3 { "Profit!" }
                            p { "Access your function at " code { "your-function.faasta.xyz" } }
                        }
                    }
                }
            }

            section.cta {
                div.container {
                    h2 { "Ready to Get Started?" }
                    p { "Deploy your first function in minutes with our simple getting started guide." }
                    a.btn.primary href="/getting-started" { "Get Started Now" }
                }
            }
        }
    );

    Ok(response_html(markup))
}

/// Serve the documentation page
fn serve_docs_page() -> anyhow::Result<Response> {
    let markup = layout(
        "Documentation - Faasta",
        html! {
            section.docs-header {
                div.container {
                    h1 { "Faasta Documentation" }
                    p { "Learn how to build, deploy, and manage your functions with Faasta" }
                }
            }

            section.docs-content {
                div.container {
                    div.docs-grid {
                        div.sidebar {
                            h3 { "Contents" }
                            ul {
                                li { a href="#overview" { "Overview" } }
                                li { a href="#wasi-p2" { "WASI P2 and WASIHTTP" } }
                                li { a href="#architecture" { "Architecture" } }
                                li { a href="#functions" { "Writing Functions" } }
                                li { a href="#self-hosting" { "Self-Hosting" } }
                            }
                        }
                        div.main-content {
                            div id="overview" class="docs-section" {
                                h2 { "Overview" }
                                p { "Faasta is a cutting-edge Function-as-a-Service (FaaS) platform designed for exceptional speed and efficiency. With cold start times under 1ms and a memory overhead of less than 1KB, Faasta delivers unparalleled performance through its modern WebAssembly architecture." }
                                
                                p { "Key features include:" }
                                ul {
                                    li { "WebAssembly-powered execution" }
                                    li { "WASI P2 and WASIHTTP standards compliance" }
                                    li { "Ultra-fast cold starts" }
                                    li { "Secure function isolation" }
                                    li { "Self-hosting capabilities" }
                                }
                            }

                            div id="wasi-p2" class="docs-section" {
                                h2 { "WASI P2 and WASIHTTP" }
                                p { "Faasta implements the WebAssembly System Interface (WASI) Preview 2 specification and the WASIHTTP standard to enable:" }
                                
                                ul {
                                    li { "Standardized HTTP request and response handling" }
                                    li { "Component-based architecture for better modularity" }
                                    li { "Consistent interface for interacting with the host system" }
                                    li { "Portable functions that can run on any WASI P2 compatible runtime" }
                                }
                                
                                p { "Because Faasta uses these open standards, your functions are not locked to a specific platform and can be hosted anywhere that supports these standards." }
                            }

                            div id="architecture" class="docs-section" {
                                h2 { "Architecture" }
                                p { "Faasta's architecture consists of:" }
                                
                                ul {
                                    li {
                                        strong { "Runtime Engine:" }
                                        " Powered by Wasmtime for efficient WebAssembly execution"
                                    }
                                    li {
                                        strong { "API Gateway:" }
                                        " Handles routing HTTP requests to the appropriate functions"
                                    }
                                    li {
                                        strong { "Function Store:" }
                                        " Manages WebAssembly module storage and retrieval"
                                    }
                                    li {
                                        strong { "Authentication:" }
                                        " Secures your functions with GitHub authentication"
                                    }
                                }
                            }

                            div id="functions" class="docs-section" {
                                h2 { "Writing Functions" }
                                p { "Faasta functions are written in Rust and compiled to WebAssembly. Here's a basic example:" }
                                
                                pre {
                                    code {
r#"use spin_sdk::http::{IntoResponse, Response};
use spin_sdk::http_component;

#[http_component]
fn hello_world(req: http::Request<()>) -> anyhow::Result<impl IntoResponse> {
    let name = req.uri().query()
        .unwrap_or("")
        .split('&')
        .find_map(|pair| {
            let mut parts = pair.split('=');
            if parts.next() == Some("name") {
                parts.next()
            } else {
                None
            }
        })
        .unwrap_or("World");

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body(format!("Hello, {}!", name))
        .build())
}"# }
                                }
                            }

                            div id="self-hosting" class="docs-section" {
                                h2 { "Self-Hosting" }
                                p { "Faasta is fully self-hostable. You can run your own instance of the Faasta server to host your functions on your own infrastructure." }
                                
                                p { "Follow these steps to self-host Faasta:" }
                                ol {
                                    li { "Clone the Faasta repository" }
                                    li { "Build the server with " code { "cargo build --release" } }
                                    li { "Configure your server with the provided configuration files" }
                                    li { "Start the server with " code { "cargo run --release" } }
                                    li { "Deploy functions to your self-hosted instance" }
                                }
                            }
                        }
                    }
                }
            }
        }
    );

    Ok(response_html(markup))
}

/// Serve the getting started page
fn serve_getting_started_page() -> anyhow::Result<Response> {
    let markup = layout(
        "Getting Started - Faasta",
        html! {
            section.getting-started-header {
                div.container {
                    h1 { "Getting Started with Faasta" }
                    p { "Follow this guide to deploy your first function in minutes" }
                }
            }

            section.getting-started-content {
                div.container {
                    div.step-card {
                        div.step-number { "1" }
                        h2 { "Install the Faasta CLI" }
                        p { "Start by installing the Faasta CLI tool using cargo:" }
                        pre { code { "cargo install cargo-faasta" } }
                        p { "This will install the " code { "cargo-faasta" } " subcommand that you'll use to create, build, and deploy your functions." }
                    }

                    div.step-card {
                        div.step-number { "2" }
                        h2 { "Login with GitHub" }
                        p { "Authenticate with your GitHub account to deploy functions:" }
                        pre { code { "cargo faasta login" } }
                        p { "This will open a browser window to authenticate with GitHub. Once authenticated, you'll be able to deploy functions to Faasta." }
                    }

                    div.step-card {
                        div.step-number { "3" }
                        h2 { "Create a New Function" }
                        p { "Create a new Faasta function project:" }
                        pre { code { "cargo faasta new my-function" } }
                        p { "This will generate a new directory with a basic Faasta function template. Alternatively, you can initialize an existing directory:" }
                        pre { code {
                            "mkdir my-function\ncd my-function\ncargo faasta init"
                        } }
                    }

                    div.step-card {
                        div.step-number { "4" }
                        h2 { "Customize Your Function" }
                        p { "Edit the " code { "src/lib.rs" } " file to customize your function:" }
                        pre { code {
r#"use spin_sdk::http::{IntoResponse, Response};
use spin_sdk::http_component;

#[http_component]
fn hello_world(req: http::Request<()>) -> anyhow::Result<impl IntoResponse> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello from Faasta!")
        .build())
}"# } }
                    }

                    div.step-card {
                        div.step-number { "5" }
                        h2 { "Build Your Function" }
                        p { "Build your function for WebAssembly:" }
                        pre { code { "cargo faasta build" } }
                        p { "This compiles your Rust code to a WebAssembly module that can be deployed to Faasta." }
                    }

                    div.step-card {
                        div.step-number { "6" }
                        h2 { "Deploy Your Function" }
                        p { "Deploy your function to Faasta:" }
                        pre { code { "cargo faasta deploy" } }
                        p { "Once deployed, your function will be available at " code { "https://my-function.faasta.xyz" } " (where \"my-function\" is the name of your function)." }
                    }

                    div.step-card.next-steps {
                        h2 { "Next Steps" }
                        p { "Now that you've deployed your first function, you can:" }
                        ul {
                            li { "Explore the " a href="/docs" { "documentation" } " to learn more about Faasta's features" }
                            li { "Check out example functions for inspiration" }
                            li { "Try integrating with other services using HTTP requests" }
                            li { "Set up your own self-hosted Faasta instance" }
                        }
                    }
                }
            }
        }
    );

    Ok(response_html(markup))
}

/// Serve a 404 Not Found page
fn serve_not_found_page() -> anyhow::Result<Response> {
    let markup = layout(
        "404 Not Found - Faasta",
        html! {
            section.not-found {
                div.container {
                    h1 { "404" }
                    h2 { "Page Not Found" }
                    p { "The page you are looking for doesn't exist or has been moved." }
                    a.btn.primary href="/" { "Go Home" }
                }
            }
        }
    );

    Ok(Response::builder()
        .status(404)
        .header("content-type", "text/html")
        .body(markup.into_string())
        .build())
}

/// Common layout for all pages
fn layout(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="UTF-8";
                meta name="viewport" content="width=device-width, initial-scale=1.0";
                title { (title) }
                style { (get_css_styles()) }
            }
            body {
                header {
                    div.container {
                        div.logo {
                            h1 { "Faasta" }
                            span.tagline { "A Faster FaaS Platform" }
                        }
                        nav {
                            ul {
                                li { a href="/" class=(if title.contains("Ultra-Fast") { "active" } else { "" }) { "Home" } }
                                li { a href="/docs" class=(if title.contains("Documentation") { "active" } else { "" }) { "Documentation" } }
                                li { a href="/getting-started" class=(if title.contains("Getting Started") { "active" } else { "" }) { "Getting Started" } }
                                li { a href="https://github.com/fourlexboehm/faasta" target="_blank" { "GitHub" } }
                            }
                        }
                    }
                }

                // Main content
                (content)

                footer {
                    div.container {
                        div.footer-content {
                            div.footer-logo {
                                h2 { "Faasta" }
                                p { "A Faster FaaS Platform" }
                            }
                            div.footer-links {
                                h3 { "Resources" }
                                ul {
                                    li { a href="/docs" { "Documentation" } }
                                    li { a href="/getting-started" { "Getting Started" } }
                                    li { a href="https://github.com/yourusername/faasta" { "GitHub" } }
                                }
                            }
                            div.footer-links {
                                h3 { "Community" }
                                ul {
                                    li { a href="#" { "Discord" } }

                                    li { a href="#" { "Blog" } }
                                }
                            }
                        }
                        div.footer-bottom {
                            p { "© 2025 Faasta Project. All rights reserved." }
                            p { "Powered by WebAssembly and WASI Preview 2." }
                        }
                    }
                }
            }
        }
    }
}

/// Helper function to create HTML responses
fn response_html(markup: Markup) -> Response {
    Response::builder()
        .status(200)
        .header("content-type", "text/html")
        .body(markup.into_string())
        .build()
}

/// Get the CSS styles for the website
fn get_css_styles() -> String {
    r#"
/* Base Styles */
:root {
    --primary: #4f46e5;
    --primary-dark: #4338ca;
    --secondary: #0ea5e9;
    --dark: #1f2937;
    --light: #f9fafb;
    --gray: #6b7280;
    --success: #10b981;
    --warning: #f59e0b;
    --danger: #ef4444;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: 'Segoe UI', Arial, Helvetica, sans-serif;
    line-height: 1.6;
    color: var(--dark);
    background-color: var(--light);
}

.container {
    width: 100%;
    max-width: 1200px;
    margin: 0 auto;
    padding: 0 20px;
}

a {
    color: var(--primary);
    text-decoration: none;
    transition: color 0.3s;
}

a:hover {
    color: var(--primary-dark);
}

h1, h2, h3, h4, h5, h6 {
    margin-bottom: 1rem;
    line-height: 1.2;
}

/* Header Styles */
header {
    background-color: white;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
    padding: 1rem 0;
    position: sticky;
    top: 0;
    z-index: 1000;
}

header .container {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.logo {
    display: flex;
    flex-direction: column;
}

.logo h1 {
    font-size: 1.8rem;
    font-weight: 700;
    color: var(--primary);
    margin-bottom: 0;
}

.tagline {
    font-size: 0.9rem;
    color: var(--gray);
}

nav ul {
    display: flex;
    list-style: none;
    gap: 2rem;
}

nav a {
    color: var(--dark);
    font-weight: 500;
    padding: 0.5rem 0;
    position: relative;
}

nav a::after {
    content: '';
    position: absolute;
    bottom: 0;
    left: 0;
    width: 0;
    height: 2px;
    background-color: var(--primary);
    transition: width 0.3s;
}

nav a:hover::after,
nav a.active::after {
    width: 100%;
}

nav a.active {
    color: var(--primary);
}

/* Hero Section */
.hero {
    padding: 6rem 0;
    background: linear-gradient(135deg, #4f46e5 0%, #0ea5e9 100%);
    color: white;
    text-align: center;
}

.hero h1 {
    font-size: 3rem;
    font-weight: 800;
    margin-bottom: 1rem;
}

.subtitle {
    font-size: 1.5rem;
    margin-bottom: 2rem;
    opacity: 0.9;
}

.cta-buttons {
    display: flex;
    justify-content: center;
    gap: 1rem;
    margin-bottom: 3rem;
}

.btn {
    display: inline-block;
    padding: 0.75rem 2rem;
    border-radius: 5px;
    font-weight: 600;
    transition: all 0.3s;
    text-align: center;
}

.btn.primary {
    background-color: white;
    color: var(--primary);
}

.btn.primary:hover {
    background-color: rgba(255, 255, 255, 0.9);
    transform: translateY(-2px);
}

.btn.secondary {
    background-color: rgba(255, 255, 255, 0.1);
    color: white;
    border: 1px solid rgba(255, 255, 255, 0.2);
}

.btn.secondary:hover {
    background-color: rgba(255, 255, 255, 0.2);
    transform: translateY(-2px);
}

.stats {
    display: flex;
    justify-content: center;
    gap: 4rem;
}

.stat {
    text-align: center;
}

.stat-value {
    font-size: 2.5rem;
    font-weight: 700;
    margin-bottom: 0.5rem;
}

.stat-label {
    font-size: 1rem;
    opacity: 0.8;
}

/* Features Section */
.features {
    padding: 5rem 0;
    background-color: white;
}

.features h2 {
    text-align: center;
    font-size: 2.5rem;
    margin-bottom: 3rem;
}

.feature-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 2rem;
}

.feature {
    padding: 2rem;
    border-radius: 10px;
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05);
    transition: transform 0.3s, box-shadow 0.3s;
}

.feature:hover {
    transform: translateY(-5px);
    box-shadow: 0 8px 15px rgba(0, 0, 0, 0.1);
}

.feature-icon {
    font-size: 2.5rem;
    margin-bottom: 1rem;
}

.feature h3 {
    font-size: 1.3rem;
    margin-bottom: 1rem;
}

/* How It Works Section */
.how-it-works {
    padding: 5rem 0;
    background-color: #f3f4f6;
}

.how-it-works h2 {
    text-align: center;
    font-size: 2.5rem;
    margin-bottom: 3rem;
}

.steps {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 2rem;
}

.step {
    text-align: center;
    padding: 2rem;
    position: relative;
}

.step-number {
    width: 50px;
    height: 50px;
    background-color: var(--primary);
    color: white;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.5rem;
    font-weight: 700;
    margin: 0 auto 1rem;
}

.step h3 {
    font-size: 1.3rem;
    margin-bottom: 1rem;
}

/* CTA Section */
.cta {
    padding: 5rem 0;
    background: linear-gradient(135deg, #0ea5e9 0%, #4f46e5 100%);
    color: white;
    text-align: center;
}

.cta h2 {
    font-size: 2.5rem;
    margin-bottom: 1rem;
}

.cta p {
    font-size: 1.2rem;
    margin-bottom: 2rem;
    opacity: 0.9;
}

/* Footer Styles */
footer {
    background-color: var(--dark);
    color: white;
    padding: 4rem 0 2rem;
}

.footer-content {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 2rem;
    margin-bottom: 3rem;
}

.footer-logo h2 {
    font-size: 1.8rem;
    color: white;
    margin-bottom: 0.5rem;
}

.footer-logo p {
    color: rgba(255, 255, 255, 0.7);
}

.footer-links h3 {
    font-size: 1.2rem;
    margin-bottom: 1.5rem;
}

.footer-links ul {
    list-style: none;
}

.footer-links li {
    margin-bottom: 0.8rem;
}

.footer-links a {
    color: rgba(255, 255, 255, 0.7);
    transition: color 0.3s;
}

.footer-links a:hover {
    color: white;
}

.footer-bottom {
    border-top: 1px solid rgba(255, 255, 255, 0.1);
    padding-top: 2rem;
    text-align: center;
    color: rgba(255, 255, 255, 0.7);
    font-size: 0.9rem;
}

.footer-bottom p {
    margin-bottom: 0.5rem;
}

/* Documentation Page Styles */
.docs-header {
    padding: 4rem 0;
    background: linear-gradient(135deg, #4f46e5 0%, #0ea5e9 100%);
    color: white;
    text-align: center;
}

.docs-header h1 {
    font-size: 2.5rem;
    margin-bottom: 1rem;
}

.docs-content {
    padding: 4rem 0;
}

.docs-grid {
    display: grid;
    grid-template-columns: 250px 1fr;
    gap: 2rem;
}

.sidebar {
    position: sticky;
    top: 100px;
    height: fit-content;
}

.sidebar h3 {
    font-size: 1.2rem;
    margin-bottom: 1.5rem;
}

.sidebar ul {
    list-style: none;
}

.sidebar li {
    margin-bottom: 0.8rem;
}

.docs-section {
    margin-bottom: 3rem;
}

.docs-section h2 {
    font-size: 1.8rem;
    margin-bottom: 1.5rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid #e5e7eb;
}

.docs-section p, .docs-section ul, .docs-section ol {
    margin-bottom: 1.5rem;
}

.docs-section ul, .docs-section ol {
    padding-left: 1.5rem;
}

.docs-section li {
    margin-bottom: 0.5rem;
}

.docs-section pre {
    background-color: #f3f4f6;
    padding: 1.5rem;
    border-radius: 5px;
    overflow-x: auto;
    margin-bottom: 1.5rem;
}

.docs-section code {
    font-family: 'Consolas', 'Liberation Mono', monospace;
}

/* Getting Started Page Styles */
.getting-started-header {
    padding: 4rem 0;
    background: linear-gradient(135deg, #4f46e5 0%, #0ea5e9 100%);
    color: white;
    text-align: center;
}

.getting-started-header h1 {
    font-size: 2.5rem;
    margin-bottom: 1rem;
}

.getting-started-content {
    padding: 4rem 0;
}

.step-card {
    background-color: white;
    border-radius: 10px;
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05);
    padding: 2rem;
    margin-bottom: 2rem;
    position: relative;
}

.step-card .step-number {
    position: absolute;
    top: -20px;
    left: 2rem;
    width: 40px;
    height: 40px;
    background-color: var(--primary);
    color: white;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.2rem;
    font-weight: 700;
}

.step-card h2 {
    font-size: 1.5rem;
    margin-bottom: 1rem;
}

.step-card p {
    margin-bottom: 1rem;
}

.step-card pre {
    background-color: #f3f4f6;
    padding: 1rem;
    border-radius: 5px;
    overflow-x: auto;
    margin-bottom: 1rem;
}

.step-card code {
    font-family: 'Consolas', 'Liberation Mono', monospace;
}

.next-steps {
    background-color: #f0f9ff;
    border-left: 4px solid var(--secondary);
}

.next-steps ul {
    padding-left: 1.5rem;
}

.next-steps li {
    margin-bottom: 0.5rem;
}

/* 404 Page Styles */
.not-found {
    padding: 8rem 0;
    text-align: center;
}

.not-found h1 {
    font-size: 6rem;
    font-weight: 800;
    color: var(--primary);
    margin-bottom: 1rem;
}

.not-found h2 {
    font-size: 2rem;
    margin-bottom: 1.5rem;
}

.not-found p {
    font-size: 1.2rem;
    margin-bottom: 2rem;
    color: var(--gray);
}

.not-found .btn.primary {
    background-color: var(--primary);
    color: white;
}

.not-found .btn.primary:hover {
    background-color: var(--primary-dark);
}

/* Responsive Styles */
@media (max-width: 768px) {
    header .container {
        flex-direction: column;
        gap: 1rem;
    }
    
    nav ul {
        gap: 1rem;
    }
    
    .hero h1 {
        font-size: 2.5rem;
    }
    
    .subtitle {
        font-size: 1.2rem;
    }
    
    .stats {
        flex-direction: column;
        gap: 2rem;
    }
    
    .docs-grid {
        grid-template-columns: 1fr;
    }
    
    .sidebar {
        position: static;
        margin-bottom: 2rem;
    }
}"#.to_string()
}