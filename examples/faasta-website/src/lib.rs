use faasta::http::Html;

#[faasta::handler]
async fn handle() -> faasta::Result<Html<String>> {
    Ok(Html(page().to_string()))
}

fn page() -> &'static str {
    r###"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Faasta - WASIp3 functions for Rust</title>
    <meta name="description" content="Faasta runs Rust functions as WASIp3 HTTP components with simple injected SQL, KV, and blob storage.">
    <style>
      :root {
        color-scheme: light;
        --paper: #f6f3ea;
        --ink: #19211d;
        --muted: #5d6761;
        --line: #d8d0c0;
        --panel: #fffcf4;
        --green: #0e7a56;
        --red: #b84232;
        --blue: #2d5b8a;
        --amber: #b7791f;
        --code: #17201c;
      }

      * {
        box-sizing: border-box;
      }

      html {
        scroll-behavior: smooth;
      }

      body {
        margin: 0;
        min-width: 320px;
        background: var(--paper);
        color: var(--ink);
        font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }

      a {
        color: inherit;
      }

      .shell {
        width: min(1180px, calc(100% - 40px));
        margin: 0 auto;
      }

      .topbar {
        position: sticky;
        top: 0;
        z-index: 10;
        border-bottom: 1px solid rgba(25, 33, 29, 0.12);
        background: rgba(246, 243, 234, 0.92);
        backdrop-filter: blur(16px);
      }

      .nav {
        display: flex;
        align-items: center;
        justify-content: space-between;
        min-height: 64px;
        gap: 20px;
      }

      .brand {
        display: flex;
        align-items: center;
        gap: 10px;
        font-weight: 760;
        letter-spacing: 0;
        text-decoration: none;
      }

      .mark {
        width: 28px;
        height: 28px;
        border: 2px solid var(--ink);
        display: grid;
        place-items: center;
        font-size: 0.75rem;
        line-height: 1;
      }

      .links {
        display: flex;
        gap: 18px;
        color: var(--muted);
        font-size: 0.95rem;
      }

      .links a {
        text-decoration: none;
      }

      .hero {
        min-height: calc(100vh - 64px);
        display: grid;
        align-items: center;
        padding: 72px 0 56px;
        border-bottom: 1px solid var(--line);
      }

      .hero-grid {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(360px, 0.82fr);
        gap: 48px;
        align-items: center;
      }

      .eyebrow {
        margin: 0 0 18px;
        color: var(--green);
        font-size: 0.82rem;
        font-weight: 760;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }

      h1 {
        margin: 0;
        max-width: 820px;
        font-size: clamp(3.2rem, 8.7vw, 8.9rem);
        line-height: 0.86;
        letter-spacing: 0;
      }

      .lead {
        margin: 24px 0 0;
        max-width: 690px;
        color: #354039;
        font-size: clamp(1.1rem, 2vw, 1.45rem);
        line-height: 1.55;
      }

      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 34px;
      }

      .button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 44px;
        padding: 0 18px;
        border: 1px solid var(--ink);
        border-radius: 6px;
        background: var(--ink);
        color: #fffaf0;
        font-weight: 720;
        text-decoration: none;
      }

      .button.secondary {
        background: transparent;
        color: var(--ink);
      }

      .visual {
        border: 1px solid #1c2822;
        border-radius: 8px;
        background: var(--panel);
        box-shadow: 12px 12px 0 rgba(25, 33, 29, 0.14);
        overflow: hidden;
      }

      .visual-head {
        display: flex;
        justify-content: space-between;
        padding: 12px 14px;
        border-bottom: 1px solid var(--line);
        color: var(--muted);
        font-size: 0.8rem;
      }

      .component-map {
        display: block;
        width: 100%;
        height: auto;
        background: #fbf7ed;
      }

      section {
        padding: 84px 0;
        border-bottom: 1px solid var(--line);
      }

      .section-head {
        display: grid;
        grid-template-columns: 0.62fr 1fr;
        gap: 40px;
        align-items: end;
        margin-bottom: 34px;
      }

      h2 {
        margin: 0;
        font-size: clamp(2rem, 4.4vw, 4.7rem);
        line-height: 0.95;
        letter-spacing: 0;
      }

      .section-copy {
        margin: 0;
        color: var(--muted);
        font-size: 1.05rem;
        line-height: 1.65;
      }

      .grid {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 16px;
      }

      .tile {
        min-height: 220px;
        border: 1px solid var(--line);
        border-radius: 8px;
        padding: 22px;
        background: var(--panel);
      }

      .tile strong {
        display: block;
        margin-bottom: 12px;
        font-size: 1.15rem;
      }

      .tile p {
        margin: 0;
        color: var(--muted);
        line-height: 1.62;
      }

      .stripe {
        height: 4px;
        width: 64px;
        margin-bottom: 20px;
        background: var(--green);
      }

      .tile:nth-child(2) .stripe {
        background: var(--blue);
      }

      .tile:nth-child(3) .stripe {
        background: var(--red);
      }

      .docs {
        display: grid;
        grid-template-columns: minmax(260px, 0.55fr) minmax(0, 1fr);
        gap: 22px;
      }

      .rail {
        border: 1px solid var(--line);
        border-radius: 8px;
        background: var(--panel);
        padding: 18px;
        align-self: start;
      }

      .rail a {
        display: block;
        padding: 10px 0;
        border-bottom: 1px solid rgba(25, 33, 29, 0.1);
        color: var(--muted);
        text-decoration: none;
      }

      .rail a:last-child {
        border-bottom: 0;
      }

      .doc-stack {
        display: grid;
        gap: 16px;
      }

      .doc {
        border: 1px solid var(--line);
        border-radius: 8px;
        background: var(--panel);
        padding: 22px;
      }

      .doc h3 {
        margin: 0 0 14px;
        font-size: 1.2rem;
      }

      pre {
        margin: 0;
        overflow: auto;
        border-radius: 8px;
        background: var(--code);
        color: #f8f1df;
        padding: 18px;
        font-size: 0.92rem;
        line-height: 1.55;
      }

      code {
        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      }

      .inline-code {
        padding: 0.13rem 0.34rem;
        border: 1px solid rgba(25, 33, 29, 0.12);
        border-radius: 4px;
        background: rgba(255, 252, 244, 0.72);
      }

      .matrix {
        width: 100%;
        border-collapse: collapse;
        overflow: hidden;
        border: 1px solid var(--line);
        border-radius: 8px;
        background: var(--panel);
      }

      th,
      td {
        padding: 14px 16px;
        border-bottom: 1px solid var(--line);
        text-align: left;
        vertical-align: top;
      }

      th {
        color: var(--muted);
        font-size: 0.82rem;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }

      tr:last-child td {
        border-bottom: 0;
      }

      .footer {
        padding: 42px 0;
        color: var(--muted);
      }

      @media (max-width: 860px) {
        .shell {
          width: min(100% - 28px, 1180px);
        }

        .links {
          display: none;
        }

        .hero {
          min-height: auto;
          padding-top: 44px;
        }

        .hero-grid,
        .section-head,
        .docs {
          grid-template-columns: 1fr;
        }

        .grid {
          grid-template-columns: 1fr;
        }

        .visual {
          box-shadow: 7px 7px 0 rgba(25, 33, 29, 0.14);
        }

        section {
          padding: 58px 0;
        }
      }
    </style>
  </head>
  <body>
    <header class="topbar">
      <nav class="shell nav" aria-label="Primary">
        <a class="brand" href="#top"><span class="mark">F</span><span>Faasta</span></a>
        <div class="links">
          <a href="#model">Model</a>
          <a href="#docs">Docs</a>
          <a href="#storage">Storage</a>
          <a href="#deploy">Deploy</a>
        </div>
      </nav>
    </header>

    <main id="top">
      <section class="hero">
        <div class="shell hero-grid">
          <div>
            <p class="eyebrow">Rust functions as WASIp3 components</p>
            <h1>Ship small async services without building a platform.</h1>
            <p class="lead">Faasta packages Rust handlers as <span class="inline-code">wasi:http/service</span> components, runs them with Wasmtime, and injects tenant-scoped SQL, KV, and blob storage at the host boundary.</p>
            <div class="actions">
              <a class="button" href="#docs">Start with the SDK</a>
              <a class="button secondary" href="#storage">Storage model</a>
            </div>
          </div>
          <aside class="visual" aria-label="Faasta runtime diagram">
            <div class="visual-head"><span>request path</span><span>wasi:http/service</span></div>
            <svg class="component-map" viewBox="0 0 640 520" role="img" aria-label="Faasta routes HTTP requests into a WASIp3 component with host capabilities">
              <rect x="36" y="42" width="568" height="436" rx="10" fill="#fffaf0" stroke="#1b241f" stroke-width="2"/>
              <rect x="72" y="82" width="180" height="72" rx="6" fill="#e8f1ec" stroke="#0e7a56" stroke-width="2"/>
              <text x="95" y="126" fill="#19211d" font-size="24" font-family="ui-monospace, monospace">HTTP request</text>
              <path d="M252 118 L344 118" stroke="#19211d" stroke-width="3"/>
              <path d="M332 106 L348 118 L332 130" fill="none" stroke="#19211d" stroke-width="3"/>
              <rect x="352" y="74" width="216" height="88" rx="6" fill="#eef3fb" stroke="#2d5b8a" stroke-width="2"/>
              <text x="382" y="111" fill="#19211d" font-size="22" font-family="ui-monospace, monospace">#[faasta::</text>
              <text x="392" y="138" fill="#19211d" font-size="22" font-family="ui-monospace, monospace">handler]</text>
              <rect x="92" y="222" width="456" height="104" rx="8" fill="#17201c"/>
              <text x="122" y="263" fill="#f8f1df" font-size="21" font-family="ui-monospace, monospace">async fn handle(sql, kv, blobs)</text>
              <text x="122" y="294" fill="#9fe1bf" font-size="21" font-family="ui-monospace, monospace">  -&gt; Result&lt;Html&lt;String&gt;&gt;</text>
              <path d="M186 326 L186 378 M320 326 L320 378 M454 326 L454 378" stroke="#19211d" stroke-width="3"/>
              <rect x="96" y="382" width="112" height="54" rx="6" fill="#f0eadf" stroke="#b7791f" stroke-width="2"/>
              <rect x="264" y="382" width="112" height="54" rx="6" fill="#f0eadf" stroke="#b84232" stroke-width="2"/>
              <rect x="424" y="382" width="112" height="54" rx="6" fill="#f0eadf" stroke="#0e7a56" stroke-width="2"/>
              <text x="136" y="416" fill="#19211d" font-size="20" font-family="ui-monospace, monospace">SQL</text>
              <text x="305" y="416" fill="#19211d" font-size="20" font-family="ui-monospace, monospace">KV</text>
              <text x="455" y="416" fill="#19211d" font-size="20" font-family="ui-monospace, monospace">Blobs</text>
            </svg>
          </aside>
        </div>
      </section>

      <section id="model">
        <div class="shell">
          <div class="section-head">
            <h2>The current model is component-first.</h2>
            <p class="section-copy">No native shared libraries, no user-managed process wrappers. User code exports a WASIp3 HTTP service through the SDK macro, and the host owns routing, capability wiring, tenant isolation, and distributed backends.</p>
          </div>
          <div class="grid">
            <article class="tile">
              <div class="stripe"></div>
              <strong>One dependency</strong>
              <p>Application crates depend on <span class="inline-code">faasta</span>. The macro, WASIp3 bindings, and capability adapters stay behind the SDK.</p>
            </article>
            <article class="tile">
              <div class="stripe"></div>
              <strong>Async host calls</strong>
              <p>Handlers are async. SQL, KV, blob, and response body work can yield back to the Wasmtime/Tokio request path instead of blocking the server.</p>
            </article>
            <article class="tile">
              <div class="stripe"></div>
              <strong>Tenant-scoped storage</strong>
              <p>Guests open simple names. Faasta maps those names to per-function SQL schemas, object prefixes, and key prefixes.</p>
            </article>
          </div>
        </div>
      </section>

      <section id="docs">
        <div class="shell">
          <div class="section-head">
            <h2>Latest usage</h2>
            <p class="section-copy">The CLI wraps the component build. Most projects should start with <span class="inline-code">cargo faasta new</span>, edit the generated handler, then build and deploy with the same commands.</p>
          </div>
          <div class="docs">
            <aside class="rail" aria-label="Documentation sections">
              <a href="#new-project">New project</a>
              <a href="#html-handler">Return HTML</a>
              <a href="#json-handler">Return JSON</a>
              <a href="#capabilities">Use capabilities</a>
            </aside>
            <div class="doc-stack">
              <article class="doc" id="new-project">
                <h3>Create and deploy</h3>
                <pre><code>cargo install cargo-faasta
cargo faasta new hello-site
cd hello-site
cargo faasta build
cargo faasta deploy</code></pre>
              </article>

              <article class="doc" id="html-handler">
                <h3>HTML response</h3>
                <pre><code>use faasta::http::Html;

#[faasta::handler]
async fn handle() -&gt; faasta::Result&lt;Html&lt;String&gt;&gt; {
    Ok(Html("&lt;h1&gt;Hello from Faasta&lt;/h1&gt;".to_string()))
}</code></pre>
              </article>

              <article class="doc" id="json-handler">
                <h3>JSON response</h3>
                <pre><code>use faasta::http::Json;
use serde::Serialize;

#[derive(Serialize)]
struct Output {
    ok: bool,
}

#[faasta::handler]
async fn handle() -&gt; faasta::Result&lt;Json&lt;Output&gt;&gt; {
    Ok(Json(Output { ok: true }))
}</code></pre>
              </article>

              <article class="doc" id="capabilities">
                <h3>Injected storage</h3>
                <pre><code>use faasta::{blob::Blobs, http::Json, kv::Kv, sql::Sql};

#[faasta::handler]
async fn handle(kv: Kv, sql: Sql, blobs: Blobs) -&gt; faasta::Result&lt;Json&lt;Output&gt;&gt; {
    kv.bucket("cache").set("last", "hello").await?;
    sql.exec("CREATE TABLE IF NOT EXISTS hits (message TEXT)", ()).await?;
    blobs.container("uploads").create_if_missing().await?;
    Ok(Json(Output { ok: true }))
}</code></pre>
              </article>
            </div>
          </div>
        </div>
      </section>

      <section id="storage">
        <div class="shell">
          <div class="section-head">
            <h2>Backends without new guest APIs.</h2>
            <p class="section-copy">Faasta keeps guest code stable while swapping host providers. Local development can stay simple; multi-node deployments point every server at the same Postgres, Garage/S3, and Valkey services.</p>
          </div>
          <table class="matrix">
            <thead>
              <tr>
                <th>Capability</th>
                <th>Local default</th>
                <th>Distributed backend</th>
                <th>Environment</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>SQL</td>
                <td>Per-function SQLite files</td>
                <td>Postgres schema per function</td>
                <td><code>FAASTA_SQL_BACKEND=postgres</code></td>
              </tr>
              <tr>
                <td>Blob</td>
                <td>In-memory provider</td>
                <td>Garage or S3-compatible object storage</td>
                <td><code>FAASTA_BLOB_BACKEND=s3</code></td>
              </tr>
              <tr>
                <td>KV</td>
                <td>In-memory provider</td>
                <td>Valkey with tenant prefixes</td>
                <td><code>FAASTA_KV_BACKEND=valkey</code></td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section id="deploy">
        <div class="shell">
          <div class="section-head">
            <h2>Build wraps the target details.</h2>
            <p class="section-copy">Faasta targets WASIp3 components. The Rust target is still evolving, so the CLI owns the target name and artifact path. Application docs should teach the Faasta workflow instead of asking users to memorize component build internals.</p>
          </div>
          <pre><code>cargo faasta build
# wraps the WASIp3 component build

cargo faasta deploy
# uploads target/wasm32-wasip3/release/&lt;crate&gt;.wasm by default</code></pre>
        </div>
      </section>
    </main>

    <footer class="footer">
      <div class="shell">Faasta is moving all-in on WASI components, async handlers, and host-managed capabilities.</div>
    </footer>
  </body>
</html>"###
}
