use waki::Client;
use waki::{handler, ErrorCode, Request, Response};

// Single handler function that routes based on path
#[handler]
fn handle_request(req: Request) -> Result<Response, ErrorCode> {
    // Get path from request
    let path = req.path();
    
    // Route the request based on path
    if path == "/api" || path == "/api/" {
        handle_external_api(req)
    } else {
        // Default to retirement calculator
        handle_retirement_calculator(req)
    }
}

// External API proxy endpoint
fn handle_external_api(req: Request) -> Result<Response, ErrorCode> {
    // Get the API endpoint from query parameters
    let query = req.query();
    let default_endpoint = "get".to_string();
    let endpoint = query.get("endpoint").unwrap_or(&default_endpoint);

    // Create a new HTTP client
    let client = Client::new();

    // Build the URL for the external API
    let url = format!("https://httpbin.org/{}", endpoint);

    // Make the request to the external API
    let external_response = match client.get(&url).send() {
        Ok(resp) => resp,
        Err(_) => {
            return Response::builder()
                .status_code(500) // Internal Server Error
                .body("Failed to connect to external API")
                .build()
                .map_err(|_| ErrorCode::InternalError(None));
        }
    };

    // Get the status code from the external response
    let status_code = external_response.status_code();

    // Get the body from the external response
    let body = match external_response.body() {
        Ok(body) => body,
        Err(_) => {
            return Response::builder()
                .status_code(500) // Internal Server Error
                .body("Failed to read response body from external API")
                .build()
                .map_err(|_| ErrorCode::InternalError(None));
        }
    };

    // Return the response from the external API
    Response::builder()
        .status_code(status_code)
        .body(body)
        .build()
        .map_err(|_| ErrorCode::InternalError(None))
}

// Retirement calculator endpoint
fn handle_retirement_calculator(_req: Request) -> Result<Response, ErrorCode> {
    // HTML content for the retirement calculator
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Retirement Calculator</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1e3c72 0%, #2a5298 100%);
            min-height: 100vh;
            padding: 20px;
            color: #333;
        }
        
        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.95);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
        }
        
        h1 {
            color: #1e3c72;
            margin-bottom: 30px;
            text-align: center;
            font-size: 2.5em;
        }
        
        .input-section {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 20px;
            margin-bottom: 40px;
            padding: 30px;
            background: #f8f9fa;
            border-radius: 15px;
        }
        
        .input-group {
            display: flex;
            flex-direction: column;
        }
        
        label {
            font-weight: 600;
            margin-bottom: 8px;
            color: #2a5298;
        }
        
        input {
            padding: 12px 16px;
            border: 2px solid #e0e0e0;
            border-radius: 8px;
            font-size: 16px;
            transition: all 0.3s ease;
        }
        
        input:focus {
            outline: none;
            border-color: #2a5298;
            box-shadow: 0 0 0 3px rgba(42, 82, 152, 0.1);
        }
        
        .results {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
            margin-bottom: 40px;
        }
        
        .result-card {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            border-radius: 15px;
            text-align: center;
            transform: translateY(0);
            transition: transform 0.3s ease;
        }
        
        .result-card:hover {
            transform: translateY(-5px);
        }
        
        .result-label {
            font-size: 14px;
            opacity: 0.9;
            margin-bottom: 10px;
        }
        
        .result-value {
            font-size: 36px;
            font-weight: 700;
            margin-bottom: 5px;
        }
        
        .result-detail {
            font-size: 14px;
            opacity: 0.8;
        }
        
        .chart-container {
            background: white;
            padding: 30px;
            border-radius: 15px;
            box-shadow: 0 5px 20px rgba(0, 0, 0, 0.1);
            margin-bottom: 30px;
        }
        
        canvas {
            max-width: 100%;
            height: 400px;
        }
        
        .breakdown {
            background: #f8f9fa;
            padding: 30px;
            border-radius: 15px;
            margin-top: 30px;
        }
        
        .breakdown h3 {
            color: #1e3c72;
            margin-bottom: 20px;
        }
        
        .breakdown-item {
            display: flex;
            justify-content: space-between;
            padding: 15px 0;
            border-bottom: 1px solid #e0e0e0;
        }
        
        .breakdown-item:last-child {
            border-bottom: none;
        }
        
        .error {
            background: #fee;
            color: #c33;
            padding: 20px;
            border-radius: 10px;
            margin: 20px 0;
            text-align: center;
            font-weight: 600;
        }
        
        .info-box {
            background: #e3f2fd;
            border-left: 4px solid #2196f3;
            padding: 20px;
            margin: 20px 0;
            border-radius: 5px;
        }
        
        .info-box h4 {
            color: #1976d2;
            margin-bottom: 10px;
        }
        
        .info-box p {
            color: #555;
            line-height: 1.6;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🏖️ Retirement Calculator</h1>
        
        <div class="input-section">
            <div class="input-group">
                <label for="currentAge">Current Age</label>
                <input type="number" id="currentAge" value="30" min="18" max="59">
            </div>
            <div class="input-group">
                <label for="weeklyContribution">Weekly Retirement Contribution ($)</label>
                <input type="number" id="weeklyContribution" value="500" min="0" step="50">
            </div>
            <div class="input-group">
                <label for="currentSuper">Current Super Balance ($)</label>
                <input type="number" id="currentSuper" value="50000" min="0" step="1000">
            </div>
            <div class="input-group">
                <label for="currentNonSuper">Current Non-Super Investments ($)</label>
                <input type="number" id="currentNonSuper" value="10000" min="0" step="1000">
            </div>
        </div>
        
        <div id="results"></div>
        
        <div class="chart-container">
            <canvas id="wealthChart"></canvas>
        </div>
        
        <div id="breakdown"></div>
        
        <div class="info-box">
            <h4>How this calculator works:</h4>
            <p>1. First, we calculate how much you need in super at age 60 to last until 85 ($50k/year spending)</p>
            <p>2. Your contributions go to super (5.5% return) until this target is reached</p>
            <p>3. Once super is sufficient, contributions go to non-super investments (5% return)</p>
            <p>4. You can retire when your non-super investments can sustain you until age 60</p>
        </div>
    </div>
    
    <script src="https://cdnjs.cloudflare.com/ajax/libs/Chart.js/4.4.0/chart.umd.js"></script>
    <script>
        let wealthChart = null;
        
        function calculateRetirement() {
            const currentAge = parseInt(document.getElementById('currentAge').value);
            const weeklyContribution = parseFloat(document.getElementById('weeklyContribution').value);
            const currentSuper = parseFloat(document.getElementById('currentSuper').value);
            const currentNonSuper = parseFloat(document.getElementById('currentNonSuper').value);
            
            const annualContribution = weeklyContribution * 52;
            const annualSpending = 50000;
            const superReturn = 0.055;
            const nonSuperReturn = 0.05;
            
            // Calculate required super balance at 60 to last until 85
            let requiredSuperAt60 = 0;
            let balance = 0;
            for (let year = 0; year < 25; year++) {
                requiredSuperAt60 += annualSpending / Math.pow(1 + superReturn, year);
            }
            
            // Simulate year by year
            let superBalance = currentSuper;
            let nonSuperBalance = currentNonSuper;
            let retirementAge = null;
            let yearData = [];
            let superSufficient = false;
            let superSufficientAge = null;
            
            for (let age = currentAge; age <= 85; age++) {
                // Check if super will be sufficient at 60
                let projectedSuperAt60 = superBalance * Math.pow(1 + superReturn, 60 - age);
                
                if (!superSufficient && projectedSuperAt60 >= requiredSuperAt60) {
                    superSufficient = true;
                    superSufficientAge = age;
                }
                
                // Add contributions if still working
                if (!retirementAge) {
                    if (superSufficient) {
                        nonSuperBalance += annualContribution;
                    } else {
                        superBalance += annualContribution;
                    }
                }
                
                // Apply returns
                superBalance *= (1 + superReturn);
                nonSuperBalance *= (1 + nonSuperReturn);
                
                // Check if can retire (non-super can last until 60)
                if (!retirementAge && age < 60 && superSufficient) {
                    let yearsUntil60 = 60 - age;
                    let requiredNonSuper = 0;
                    let tempBalance = 0;
                    
                    for (let y = 0; y < yearsUntil60; y++) {
                        requiredNonSuper += annualSpending / Math.pow(1 + nonSuperReturn, y);
                    }
                    
                    if (nonSuperBalance >= requiredNonSuper) {
                        retirementAge = age;
                    }
                }
                
                // Spending in retirement
                if (retirementAge) {
                    if (age < 60) {
                        nonSuperBalance -= annualSpending;
                        if (nonSuperBalance < 0) nonSuperBalance = 0;
                    } else {
                        superBalance -= annualSpending;
                        if (superBalance < 0) superBalance = 0;
                    }
                }
                
                yearData.push({
                    age: age,
                    super: Math.round(superBalance),
                    nonSuper: Math.round(nonSuperBalance),
                    total: Math.round(superBalance + nonSuperBalance)
                });
            }
            
            return {
                retirementAge: retirementAge,
                superSufficientAge: superSufficientAge,
                requiredSuperAt60: requiredSuperAt60,
                yearData: yearData,
                finalSuper: superBalance,
                finalNonSuper: nonSuperBalance
            };
        }
        
        function formatCurrency(amount) {
            return new Intl.NumberFormat('en-US', {
                style: 'currency',
                currency: 'USD',
                minimumFractionDigits: 0,
                maximumFractionDigits: 0
            }).format(amount);
        }
        
        function updateChart(data) {
            const ctx = document.getElementById('wealthChart').getContext('2d');
            
            if (wealthChart) {
                wealthChart.destroy();
            }
            
            wealthChart = new Chart(ctx, {
                type: 'line',
                data: {
                    labels: data.yearData.map(d => d.age),
                    datasets: [
                        {
                            label: 'Super Balance',
                            data: data.yearData.map(d => d.super),
                            borderColor: '#2196f3',
                            backgroundColor: 'rgba(33, 150, 243, 0.1)',
                            fill: true,
                            tension: 0.4
                        },
                        {
                            label: 'Non-Super Balance',
                            data: data.yearData.map(d => d.nonSuper),
                            borderColor: '#4caf50',
                            backgroundColor: 'rgba(76, 175, 80, 0.1)',
                            fill: true,
                            tension: 0.4
                        },
                        {
                            label: 'Total Wealth',
                            data: data.yearData.map(d => d.total),
                            borderColor: '#9c27b0',
                            backgroundColor: 'rgba(156, 39, 176, 0.1)',
                            fill: false,
                            borderWidth: 3,
                            tension: 0.4
                        }
                    ]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    plugins: {
                        title: {
                            display: true,
                            text: 'Wealth Projection Over Time',
                            font: { size: 18 }
                        },
                        legend: {
                            position: 'top'
                        }
                    },
                    scales: {
                        x: {
                            title: {
                                display: true,
                                text: 'Age'
                            }
                        },
                        y: {
                            title: {
                                display: true,
                                text: 'Balance ($)'
                            },
                            ticks: {
                                callback: function(value) {
                                    return formatCurrency(value);
                                }
                            }
                        }
                    }
                }
            });
        }
        
        function displayResults() {
            const results = calculateRetirement();
            const currentAge = parseInt(document.getElementById('currentAge').value);
            
            let resultsHTML = '<div class="results">';
            
            if (results.retirementAge) {
                resultsHTML += `
                    <div class="result-card" style="background: linear-gradient(135deg, #11998e 0%, #38ef7d 100%);">
                        <div class="result-label">You can retire at age</div>
                        <div class="result-value">${results.retirementAge}</div>
                        <div class="result-detail">In ${results.retirementAge - currentAge} years</div>
                    </div>
                `;
            } else {
                resultsHTML += `
                    <div class="result-card" style="background: linear-gradient(135deg, #eb3349 0%, #f45c43 100%);">
                        <div class="result-label">Cannot retire before 60</div>
                        <div class="result-value">Need More</div>
                        <div class="result-detail">Increase contributions</div>
                    </div>
                `;
            }
            
            resultsHTML += `
                <div class="result-card">
                    <div class="result-label">Super Target for Ages 60-85</div>
                    <div class="result-value">${formatCurrency(results.requiredSuperAt60)}</div>
                    <div class="result-detail">Reached at age ${results.superSufficientAge || 'Never'}</div>
                </div>
                <div class="result-card" style="background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);">
                    <div class="result-label">Final Balance at 85</div>
                    <div class="result-value">${formatCurrency(results.finalSuper + results.finalNonSuper)}</div>
                    <div class="result-detail">Super: ${formatCurrency(results.finalSuper)}</div>
                </div>
            `;
            
            resultsHTML += '</div>';
            
            if (!results.retirementAge) {
                resultsHTML += '<div class="error">You need to increase your weekly contributions to retire before 60. Your super alone will support you from 60-85.</div>';
            }
            
            document.getElementById('results').innerHTML = resultsHTML;
            
            // Breakdown
            let breakdownHTML = '<div class="breakdown"><h3>📊 Detailed Breakdown</h3>';
            
            if (results.retirementAge) {
                const yearsToRetirement = results.retirementAge - currentAge;
                const totalContributions = weeklyContribution.value * 52 * yearsToRetirement;
                
                breakdownHTML += `
                    <div class="breakdown-item">
                        <span>Years until retirement:</span>
                        <strong>${yearsToRetirement} years</strong>
                    </div>
                    <div class="breakdown-item">
                        <span>Total contributions until retirement:</span>
                        <strong>${formatCurrency(totalContributions)}</strong>
                    </div>
                    <div class="breakdown-item">
                        <span>Years in retirement:</span>
                        <strong>${85 - results.retirementAge} years</strong>
                    </div>
                    <div class="breakdown-item">
                        <span>Super sufficient at age:</span>
                        <strong>${results.superSufficientAge}</strong>
                    </div>
                `;
            }
            
            breakdownHTML += '</div>';
            document.getElementById('breakdown').innerHTML = breakdownHTML;
            
            updateChart(results);
        }
        
        // Event listeners
        document.querySelectorAll('input').forEach(input => {
            input.addEventListener('input', displayResults);
        });
        
        // Initial calculation
        displayResults();
    </script>
</body>
</html>"#;

    // Return the HTML response
    Response::builder()
        .status_code(200)
        .header("Content-Type", "text/html")
        .body(html)
        .build()
        .map_err(|_| ErrorCode::InternalError(None))
}
