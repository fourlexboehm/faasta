use std::collections::HashMap;

use cap_async_std::fs::Dir;
use faasta_macros::faasta;
use faasta_types::{FaastaRequest, FaastaResponse};
use serde::Serialize;
use serde_json::json;
use url::form_urlencoded;

#[derive(Serialize)]
struct RetirementParams {
    years_until_retirement: f64,
    current_savings: f64,
    annual_contribution: f64,
    expected_return_rate: f64,
}

fn parse_query(uri: &str) -> HashMap<String, String> {
    uri.splitn(2, '?')
        .nth(1)
        .map(|query| form_urlencoded::parse(query.as_bytes()).into_owned().collect())
        .unwrap_or_default()
}

fn params_from_query(query: HashMap<String, String>) -> RetirementParams {
    let get = |key: &str, default: f64| -> f64 {
        query
            .get(key)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(default)
    };

    RetirementParams {
        years_until_retirement: get("years", 25.0),
        current_savings: get("savings", 50_000.0),
        annual_contribution: get("contribution", 12_000.0),
        expected_return_rate: get("return", 0.06),
    }
}

fn projected_balance(params: &RetirementParams) -> f64 {
    let mut balance = params.current_savings;
    for _ in 0..params.years_until_retirement as usize {
        balance *= 1.0 + params.expected_return_rate;
        balance += params.annual_contribution;
    }
    balance
}

fn build_html(params: &RetirementParams, future_balance: f64) -> String {
    let summary = json!({
        "years_until_retirement": params.years_until_retirement,
        "current_savings": params.current_savings,
        "annual_contribution": params.annual_contribution,
        "expected_return_rate": params.expected_return_rate,
        "projected_balance": future_balance,
    });

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8" />
    <title>Retirement Planner</title>
    <style>
        body {{
            font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
            margin: 0;
            padding: 40px;
            background: linear-gradient(135deg, #1e3c72 0%, #2a5298 100%);
            color: #0f172a;
        }}
        .card {{
            max-width: 840px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.95);
            border-radius: 18px;
            padding: 36px;
            box-shadow: 0 24px 60px rgba(15, 23, 42, 0.25);
        }}
        h1 {{
            margin-top: 0;
            font-size: 2.25rem;
            color: #1e3a8a;
        }}
        .grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 18px;
            margin: 32px 0;
        }}
        .tile {{
            background: linear-gradient(135deg, #67e8f9 0%, #7c3aed 100%);
            color: white;
            padding: 24px;
            border-radius: 14px;
            box-shadow: 0 16px 32px rgba(124, 58, 237, 0.35);
        }}
        .tile strong {{
            display: block;
            font-size: 2rem;
            margin-bottom: 6px;
        }}
        .summary {{
            background: #eef2ff;
            border-radius: 14px;
            padding: 20px;
            font-family: monospace;
            overflow-x: auto;
        }}
    </style>
</head>
<body>
    <div class="card">
        <h1>Retirement Planner</h1>
        <p>Update the query string to explore different scenarios. Try <code>?years=30&return=0.07&contribution=15000</code>.</p>
        <div class="grid">
            <div class="tile">
                <span>Years Remaining</span>
                <strong>{years:.0}</strong>
            </div>
            <div class="tile">
                <span>Current Savings</span>
                <strong>${current:.0}</strong>
            </div>
            <div class="tile">
                <span>Annual Contribution</span>
                <strong>${contribution:.0}</strong>
            </div>
            <div class="tile">
                <span>Projected Balance</span>
                <strong>${projected:.0}</strong>
            </div>
        </div>
        <h2>Raw Numbers</h2>
        <div class="summary">{summary}</div>
    </div>
</body>
</html>"#,
        years = params.years_until_retirement,
        current = params.current_savings,
        contribution = params.annual_contribution,
        projected = future_balance,
        summary = serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".to_string()),
    )
}

#[faasta]
pub async fn retirement_planner(request: FaastaRequest, _dir: Dir) -> FaastaResponse {
    let uri_string = request.uri.as_str().to_string();
    let params = params_from_query(parse_query(&uri_string));
    let projected_balance = projected_balance(&params);
    let html = build_html(&params, projected_balance);

    FaastaResponse::new(200)
        .header("content-type", "text/html; charset=utf-8")
        .with_body(html.into_bytes())
}
