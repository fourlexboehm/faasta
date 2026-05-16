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

      .stats {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 12px;
        margin-top: 28px;
        max-width: 690px;
      }

      .stat {
        border-top: 2px solid var(--ink);
        padding-top: 12px;
      }

      .stat-value {
        display: block;
        font-size: 1.4rem;
        font-weight: 780;
        line-height: 1.1;
      }

      .stat-label {
        display: block;
        margin-top: 6px;
        color: var(--muted);
        font-size: 0.88rem;
        line-height: 1.35;
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

      .efficiency {
        display: grid;
        grid-template-columns: minmax(0, 0.95fr) minmax(0, 1.05fr);
        gap: 22px;
        align-items: stretch;
      }

      .process-card,
      .comparison-card {
        border: 1px solid var(--line);
        border-radius: 8px;
        background: var(--panel);
        padding: 22px;
      }

      .process-stack {
        display: grid;
        gap: 10px;
      }

      .process-row {
        display: grid;
        grid-template-columns: 134px 1fr;
        gap: 12px;
        align-items: center;
      }

      .process-label {
        color: var(--muted);
        font-size: 0.86rem;
        line-height: 1.35;
      }

      .runtime-lane {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
        min-height: 42px;
        align-items: center;
        border: 1px solid rgba(25, 33, 29, 0.12);
        border-radius: 6px;
        padding: 8px;
        background: #f8f1df;
      }

      .runtime-pill {
        border-radius: 5px;
        padding: 7px 9px;
        background: #17201c;
        color: #f8f1df;
        font-size: 0.78rem;
        font-weight: 720;
      }

      .runtime-pill.host {
        background: var(--green);
      }

      .runtime-pill.platform {
        background: var(--blue);
      }

      .runtime-pill.heavy {
        background: var(--red);
      }

      .bar-list {
        display: grid;
        gap: 14px;
      }

      .bar-row {
        display: grid;
        gap: 7px;
      }

      .bar-meta {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        color: var(--muted);
        font-size: 0.88rem;
      }

      .bar-track {
        height: 12px;
        border: 1px solid rgba(25, 33, 29, 0.18);
        border-radius: 999px;
        background: #f8f1df;
        overflow: hidden;
      }

      .bar-fill {
        height: 100%;
        border-radius: inherit;
        background: var(--green);
      }

      .bar-fill.medium {
        background: var(--blue);
      }

      .bar-fill.high {
        background: var(--amber);
      }

      .bar-fill.max {
        background: var(--red);
      }

      .wide-card {
        grid-column: 1 / -1;
      }

      .resource-grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 22px;
      }

      .axis-label {
        margin: 0 0 14px;
        color: var(--muted);
        font-size: 0.86rem;
        font-weight: 720;
        text-transform: uppercase;
      }

      .note {
        margin: 16px 0 0;
        color: var(--muted);
        font-size: 0.9rem;
        line-height: 1.55;
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
        .docs,
        .efficiency,
        .stats {
          grid-template-columns: 1fr;
        }

        .process-row {
          grid-template-columns: 1fr;
        }

        .resource-grid {
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
          <a href="#model">Why</a>
          <a href="#efficiency">Cost</a>
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
            <h1>Fast Rust functions without the platform tax.</h1>
            <p class="lead">Faasta gives you the useful parts of serverless without containers, per-service boilerplate, or hand-rolled tenancy. Rust handlers compile to WASIp3 <span class="inline-code">wasi:http/service</span> components, run warm inside Wasmtime, and receive SQL, KV, and blob storage through wasi-cloud-core host capabilities.</p>
            <div class="actions">
              <a class="button" href="#docs">Start with the SDK</a>
              <a class="button secondary" href="#efficiency">Why it is cheaper</a>
            </div>
            <div class="stats" aria-label="Faasta performance and operations">
              <div class="stat">
                <span class="stat-value">Warm reuse</span>
                <span class="stat-label">precompiled components stay ready for low-latency requests</span>
              </div>
              <div class="stat">
                <span class="stat-value">Async I/O</span>
                <span class="stat-label">host calls yield through the Tokio and Wasmtime request path</span>
              </div>
              <div class="stat">
                <span class="stat-value">No containers</span>
                <span class="stat-label">lighter deploys with capability-level isolation</span>
              </div>
            </div>
          </div>
          <aside class="visual" aria-label="Faasta runtime diagram">
            <div class="visual-head"><span>request path</span><span>wasi:http/service</span></div>
            <svg class="component-map" viewBox="0 0 640 520" role="img" aria-label="Faasta routes HTTP requests into a WASIp3 component with host capabilities">
              <rect x="36" y="42" width="568" height="436" rx="10" fill="#fffaf0" stroke="#1b241f" stroke-width="2"/>
              <rect x="64" y="82" width="204" height="72" rx="6" fill="#e8f1ec" stroke="#0e7a56" stroke-width="2"/>
              <text x="92" y="126" fill="#19211d" font-size="18" font-family="ui-monospace, monospace">HTTP request</text>
              <path d="M268 118 L344 118" stroke="#19211d" stroke-width="3"/>
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
            <h2>Why Faasta exists.</h2>
            <p class="section-copy">Most small services should not need their own container image, runtime glue, database tenancy code, object-store naming scheme, and deploy pipeline. Faasta keeps the unit of deployment small: one async Rust handler, one WASIp3 component, and host-managed capabilities for fast startup, predictable isolation, and shared distributed backends.</p>
          </div>
          <div class="grid">
            <article class="tile">
              <div class="stripe"></div>
              <strong>Small deployment unit</strong>
              <p>Application crates depend on <span class="inline-code">faasta</span>. The SDK macro exports the service world, and the CLI hides the WASIp3 build details.</p>
            </article>
            <article class="tile">
              <div class="stripe"></div>
              <strong>Performance-oriented runtime</strong>
              <p>Components are validated, precompiled, cached, and invoked in-process. Async SQL, KV, blob, and response body work can yield instead of blocking the server.</p>
            </article>
            <article class="tile">
              <div class="stripe"></div>
              <strong>Tenant-scoped storage</strong>
              <p>Guests open simple names from the wasi-cloud-core capability APIs. Faasta maps those names to per-function SQL schemas, object prefixes, and key prefixes.</p>
            </article>
          </div>
        </div>
      </section>

      <section id="efficiency">
        <div class="shell">
          <div class="section-head">
            <h2>One runtime, many requests.</h2>
            <p class="section-copy">Faasta gets its cost advantage by amortizing the expensive parts. A single server process owns sockets, TLS, scheduling, Wasmtime engines, compiled component caches, database pools, and capability providers. Individual requests only enter a tenant-scoped component instance and await shared async host services.</p>
          </div>
          <div class="efficiency">
            <article class="process-card">
              <h3>Where the overhead lives</h3>
              <div class="process-stack" aria-label="Runtime overhead comparison">
                <div class="process-row">
                  <div class="process-label">Faasta node</div>
                  <div class="runtime-lane">
                    <span class="runtime-pill host">Tokio</span>
                    <span class="runtime-pill host">Wasmtime</span>
                    <span class="runtime-pill">fn A</span>
                    <span class="runtime-pill">fn B</span>
                    <span class="runtime-pill">fn C</span>
                    <span class="runtime-pill">SQL pool</span>
                  </div>
                </div>
                <div class="process-row">
                  <div class="process-label">Container per service</div>
                  <div class="runtime-lane">
                    <span class="runtime-pill heavy">runtime A</span>
                    <span class="runtime-pill heavy">runtime B</span>
                    <span class="runtime-pill heavy">runtime C</span>
                  </div>
                </div>
                <div class="process-row">
                  <div class="process-label">VM per service</div>
                  <div class="runtime-lane">
                    <span class="runtime-pill heavy">guest OS A</span>
                    <span class="runtime-pill heavy">guest OS B</span>
                    <span class="runtime-pill heavy">guest OS C</span>
                  </div>
                </div>
                <div class="process-row">
                  <div class="process-label">Managed edge/serverless</div>
                  <div class="runtime-lane">
                    <span class="runtime-pill platform">vendor runtime</span>
                    <span class="runtime-pill platform">metered CPU</span>
                    <span class="runtime-pill platform">cold starts</span>
                    <span class="runtime-pill platform">microVMs</span>
                    <span class="runtime-pill platform">bound services</span>
                  </div>
                </div>
              </div>
              <p class="note">The important trick is not magic: keep the platform hot once, then multiplex lots of small isolated components through it.</p>
            </article>
            <article class="comparison-card">
              <h3>Efficiency shape</h3>
              <div class="bar-list" aria-label="Relative overhead chart">
                <div class="bar-row">
                  <div class="bar-meta"><span>Faasta shared WASI runtime</span><span>lowest multi-tenant overhead</span></div>
                  <div class="bar-track"><div class="bar-fill" style="width: 22%"></div></div>
                </div>
                <div class="bar-row">
                  <div class="bar-meta"><span>Cloudflare Workers style isolate</span><span>efficient, but JS can pay GC/JIT overhead</span></div>
                  <div class="bar-track"><div class="bar-fill medium" style="width: 32%"></div></div>
                </div>
                <div class="bar-row">
                  <div class="bar-meta"><span>Lambda style functions</span><span>cold starts plus microVM/runtime init</span></div>
                  <div class="bar-track"><div class="bar-fill medium" style="width: 52%"></div></div>
                </div>
                <div class="bar-row">
                  <div class="bar-meta"><span>Docker service per function</span><span>duplicated runtime memory</span></div>
                  <div class="bar-track"><div class="bar-fill high" style="width: 72%"></div></div>
                </div>
                <div class="bar-row">
                  <div class="bar-meta"><span>VM per function</span><span>duplicated kernel and userspace</span></div>
                  <div class="bar-track"><div class="bar-fill max" style="width: 92%"></div></div>
                </div>
              </div>
              <p class="note">This is an architectural comparison, not a benchmark. Actual cost depends on traffic shape, memory limits, regional networking, storage backends, and how much idle capacity you keep warm.</p>
            </article>
            <article class="comparison-card wide-card">
              <h3>Memory and CPU pressure</h3>
              <div class="resource-grid">
                <div>
                  <p class="axis-label">Memory overhead</p>
                  <div class="bar-list" aria-label="Relative memory overhead chart">
                    <div class="bar-row">
                      <div class="bar-meta"><span>Faasta shared WASI runtime</span><span>engine, pools, and host services shared</span></div>
                      <div class="bar-track"><div class="bar-fill" style="width: 20%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Workers style isolate</span><span>isolate heap per workload</span></div>
                      <div class="bar-track"><div class="bar-fill medium" style="width: 34%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Lambda style function</span><span>reserved memory and microVM envelope</span></div>
                      <div class="bar-track"><div class="bar-fill medium" style="width: 55%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Container per function</span><span>runtime duplicated per image</span></div>
                      <div class="bar-track"><div class="bar-fill high" style="width: 76%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>VM per function</span><span>guest kernel and userspace duplicated</span></div>
                      <div class="bar-track"><div class="bar-fill max" style="width: 94%"></div></div>
                    </div>
                  </div>
                </div>
                <div>
                  <p class="axis-label">CPU overhead</p>
                  <div class="bar-list" aria-label="Relative CPU overhead chart">
                    <div class="bar-row">
                      <div class="bar-meta"><span>Faasta shared WASI runtime</span><span>await points reuse one scheduler</span></div>
                      <div class="bar-track"><div class="bar-fill" style="width: 24%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Workers style isolate</span><span>dispatch plus possible JIT and GC work</span></div>
                      <div class="bar-track"><div class="bar-fill medium" style="width: 40%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Lambda style function</span><span>cold start, runtime bootstrap, metering</span></div>
                      <div class="bar-track"><div class="bar-fill medium" style="width: 60%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>Container per function</span><span>process and runtime duplication</span></div>
                      <div class="bar-track"><div class="bar-fill high" style="width: 74%"></div></div>
                    </div>
                    <div class="bar-row">
                      <div class="bar-meta"><span>VM per function</span><span>scheduler and virtualization layers</span></div>
                      <div class="bar-track"><div class="bar-fill max" style="width: 88%"></div></div>
                    </div>
                  </div>
                </div>
              </div>
              <p class="note">The win is highest for many small I/O-heavy functions: while one request waits on SQL, KV, blobs, or a response body, the same process can keep polling other tenants without starting another container, microVM, or guest OS.</p>
            </article>
          </div>
        </div>
      </section>

      <section id="docs">
        <div class="shell">
          <div class="section-head">
            <h2>Usage</h2>
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
            <p class="section-copy">Faasta keeps guest code stable while swapping host providers behind the wasi-cloud-core capability contracts. Local development can stay simple; multi-node deployments point every server at the same Postgres, Garage/S3, and Valkey services.</p>
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
