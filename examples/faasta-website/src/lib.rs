use cap_async_std::fs::Dir;
use faasta_macros::faasta;
use faasta_types::{FaastaRequest, FaastaResponse};
use maud::{html, Markup, DOCTYPE};

#[faasta]
pub async fn serve_website(request: FaastaRequest, _dir: Dir) -> FaastaResponse {
    let path = request.uri.as_str();
    let (status, markup) = match path {
        "/" | "/index.html" => (200, serve_home_page()),
        "/docs" => (200, serve_docs_page()),
        "/getting-started" => (200, serve_getting_started_page()),
        _ => (404, serve_not_found_page()),
    };

    html_response(status, markup)
}

fn html_response(status: u16, markup: Markup) -> FaastaResponse {
    FaastaResponse::new(status)
        .header("content-type", "text/html; charset=utf-8")
        .with_body(markup.into_string().into_bytes())
}

fn layout(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (get_styles()) }
            }
            body {
                (content)
            }
        }
    }
}

fn serve_home_page() -> Markup {
    layout(
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
                            p { "Create your function using Rust with the Faasta macro." }
                        }
                        div.step {
                            div.step-number { "2" }
                            h3 { "Build for Native" }
                            p { "Compile your code as a shared library with " code { "cargo faasta build" } }
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
    )
}

fn serve_docs_page() -> Markup {
    layout(
        "Faasta Documentation",
        html! {
            section.docs {
                div.container {
                    h1 { "Faasta Documentation" }
                    p.subtitle { "Learn everything you need to know about building, deploying, and managing Faasta functions." }

                    div.content-grid {
                        article {
                            h2 { "Quick Start" }
                            ol {
                                li { "Install the CLI: " code { "cargo install cargo-faasta" } }
                                li { "Initialize a new project: " code { "cargo faasta init" } }
                                li { "Build your function: " code { "cargo faasta build" } }
                                li { "Deploy: " code { "cargo faasta deploy" } }
                            }

                            h2 { "Function Development" }
                            p { "Faasta functions are normal Rust async functions decorated with the " code { "#[faasta]" } " macro." }

                            h3 { "Handler Signature" }
                            pre { (r#"
#[faasta]
async fn handler(request: FaastaRequest, dir: Dir) -> FaastaResponse {
    // your code here
}
"#) }
                        }

                        aside.sidebar {
                            h3 { "Resources" }
                            ul {
                                li { a href="https://github.com/fourlexboehm/faasta" { "GitHub Repository" } }
                                li { a href="https://faasta.xyz" { "Hosted Instance" } }
                                li { a href="https://docs.rs/faasta-types" { "Type Documentation" } }
                            }
                        }
                    }
                }
            }
        }
    )
}

fn serve_getting_started_page() -> Markup {
    layout(
        "Getting Started with Faasta",
        html! {
            section.getting-started {
                div.container {
                    h1 { "Getting Started" }
                    p.subtitle { "Build and deploy your first Faasta function in minutes." }

                    div.steps-list {
                        div.step {
                            h2 { "1. Install the CLI" }
                            pre { "cargo install cargo-faasta" }
                        }
                        div.step {
                            h2 { "2. Initialize a Project" }
                            pre { "cargo faasta init" }
                        }
                        div.step {
                            h2 { "3. Build Locally" }
                            pre { "cargo faasta build" }
                        }
                        div.step {
                            h2 { "4. Deploy" }
                            pre { "cargo faasta deploy" }
                        }
                        div.step {
                            h2 { "5. Invoke" }
                            pre { "curl https://your-function.faasta.xyz" }
                        }
                    }

                    div.tip {
                        strong { "Tip:" }
                        span { " Use query parameters like " code { "?name=Alex" } " in the example project to personalize responses." }
                    }
                }
            }
        }
    )
}

fn serve_not_found_page() -> Markup {
    layout(
        "Page Not Found",
        html! {
            section.not-found {
                div.container {
                    h1 { "404" }
                    p.subtitle { "The page you're looking for couldn't be found." }
                    a.btn.primary href="/" { "Return Home" }
                }
            }
        }
    )
}

fn get_styles() -> Markup {
    html! {
        (r#"
body {
    font-family: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    margin: 0;
    background: #0f172a;
    color: #e2e8f0;
}

.hero {
    background: radial-gradient(circle at top, rgba(56, 189, 248, 0.15) 0%, transparent 45%),
                radial-gradient(circle at bottom, rgba(167, 139, 250, 0.12) 0%, transparent 50%),
                linear-gradient(135deg, #1e293b 0%, #0f172a 100%);
    padding: 120px 0 80px;
    position: relative;
    overflow: hidden;
}

.hero::before {
    content: "";
    position: absolute;
    inset: 0;
    background: radial-gradient(circle at top, rgba(56, 189, 248, 0.2) 0%, transparent 55%);
    opacity: 0.7;
}

.hero .container {
    position: relative;
    z-index: 1;
}

.container {
    max-width: 1080px;
    margin: 0 auto;
    padding: 0 24px;
}

h1, h2, h3 {
    margin: 0;
    color: #f8fafc;
}

.subtitle {
    color: #cbd5f5;
    font-size: 20px;
    margin: 24px 0 32px;
    max-width: 620px;
}

.cta-buttons {
    display: flex;
    gap: 16px;
    margin-bottom: 48px;
}

.btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 14px 28px;
    border-radius: 999px;
    font-weight: 600;
    text-decoration: none;
    transition: transform 0.2s ease, box-shadow 0.2s ease;
}

.btn.primary {
    background: linear-gradient(135deg, #38bdf8 0%, #6366f1 100%);
    color: white;
    box-shadow: 0 15px 35px rgba(99, 102, 241, 0.35);
}

.btn.secondary {
    background: rgba(148, 163, 184, 0.1);
    color: #e2e8f0;
    border: 1px solid rgba(148, 163, 184, 0.3);
}

.btn:hover {
    transform: translateY(-2px);
}

.stats {
    display: flex;
    gap: 24px;
    flex-wrap: wrap;
    margin-top: 36px;
}

.stat {
    background: rgba(148, 163, 184, 0.08);
    border-radius: 18px;
    padding: 24px;
    min-width: 160px;
}

.stat-label {
    color: #94a3b8;
    font-size: 14px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
}

.stat-value {
    font-size: 32px;
    font-weight: 700;
    margin-top: 8px;
    color: #fafafa;
}

.features {
    background: rgba(15, 23, 42, 0.85);
    padding: 80px 0;
}

.feature-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 24px;
}

.feature {
    background: rgba(148, 163, 184, 0.08);
    border-radius: 18px;
    padding: 24px;
}

.feature-icon {
    font-size: 36px;
    margin-bottom: 12px;
}

.how-it-works {
    padding: 80px 0;
}

.steps {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 24px;
}

.step {
    background: rgba(148, 163, 184, 0.08);
    border-radius: 18px;
    padding: 24px;
    border: 1px solid rgba(99, 102, 241, 0.18);
}

.step-number {
    width: 40px;
    height: 40px;
    border-radius: 999px;
    background: linear-gradient(135deg, #38bdf8 0%, #6366f1 100%);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    margin-bottom: 16px;
}

.cta {
    padding: 80px 0;
    text-align: center;
    background: radial-gradient(circle at center, rgba(99, 102, 241, 0.12) 0%, transparent 60%);
}

.docs, .getting-started, .not-found {
    padding: 80px 0;
}

.content-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    gap: 40px;
}

.sidebar {
    background: rgba(148, 163, 184, 0.08);
    border-radius: 18px;
    padding: 24px;
}

.sidebar ul {
    list-style: none;
    padding: 0;
    margin: 0;
}

.sidebar li {
    margin-bottom: 12px;
}

.sidebar a {
    color: #93c5fd;
    text-decoration: none;
}

pre {
    background: rgba(15, 23, 42, 0.85);
    border: 1px solid rgba(148, 163, 184, 0.2);
    border-radius: 12px;
    padding: 16px;
    overflow-x: auto;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 14px;
}

.tip {
    margin-top: 32px;
    background: rgba(34, 211, 238, 0.1);
    border: 1px solid rgba(34, 211, 238, 0.3);
    border-radius: 12px;
    padding: 16px;
}

.not-found {
    text-align: center;
}

.not-found h1 {
    font-size: 5rem;
}

@media (max-width: 768px) {
    .stats {
        flex-direction: column;
    }

    .cta-buttons {
        flex-direction: column;
    }

    .content-grid {
        grid-template-columns: 1fr;
    }
}
"#)
    }
}
