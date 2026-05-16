use faasta::http::Html;

#[faasta::handler]
async fn handle() -> faasta::Result<Html<String>> {
    Ok(Html(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Faasta WASIp3</title>
    <style>
      :root {
        color-scheme: light dark;
        font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      body {
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: #f7f3ea;
        color: #1d2521;
      }
      main {
        width: min(720px, calc(100vw - 40px));
      }
      h1 {
        margin: 0 0 12px;
        font-size: clamp(2rem, 8vw, 4.75rem);
        line-height: 0.95;
        letter-spacing: 0;
      }
      p {
        margin: 0;
        max-width: 54ch;
        font-size: 1.08rem;
        line-height: 1.65;
      }
      code {
        padding: 0.125rem 0.3rem;
        border-radius: 4px;
        background: rgba(29, 37, 33, 0.08);
      }
    </style>
  </head>
  <body>
    <main>
      <h1>Hello from Faasta</h1>
      <p>This page is returned by an async Rust function exported as a WASIp3 <code>wasi:http/service</code> component with <code>#[faasta::handler]</code>.</p>
    </main>
  </body>
</html>"#
            .to_string(),
    ))
}
