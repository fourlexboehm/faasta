use anyhow::{Result, anyhow};
use cyper::Client as HttpClient;
use oauth2::http as oauth_http;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, HttpRequest, HttpResponse,
    RedirectUrl, Scope, TokenResponse, TokenUrl, basic::BasicClient,
};
use serde::Deserialize;
use std::{net::SocketAddr, str::FromStr};
use tiny_http::{Response, Server};
use url::Url;

// GitHub OAuth app details
const DEFAULT_CLIENT_ID: &str = "Iv23lik79igmHPi63dO1";
const DEFAULT_CLIENT_SECRET: &str = "2a10cd3c2465622a1649b766e574f15eb9211eb7";
const REDIRECT_PORT: u16 = 9876;

type GithubOAuthClient = BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

// Test mode flag
static TEST_MODE: AtomicBool = AtomicBool::new(false);
static TEST_USERNAME: Mutex<Option<String>> = Mutex::new(None);
static TEST_TOKEN: Mutex<Option<String>> = Mutex::new(None);

#[cfg(test)]
/// Enable test mode with given username and token.
pub fn enable_test_mode(username: String, token: String) {
    TEST_MODE.store(true, Ordering::Relaxed);
    *TEST_USERNAME.lock().unwrap() = Some(username);
    *TEST_TOKEN.lock().unwrap() = Some(token);
}

/// Get the test mode status and credentials
fn get_test_data() -> (bool, Option<String>, Option<String>) {
    (
        TEST_MODE.load(Ordering::Relaxed),
        TEST_USERNAME.lock().unwrap().clone(),
        TEST_TOKEN.lock().unwrap().clone(),
    )
}

/// Get client ID from environment or use default
fn get_client_id() -> String {
    std::env::var("FAASTA_GITHUB_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string())
}

/// Get client secret from environment or use default
fn get_client_secret() -> String {
    std::env::var("FAASTA_GITHUB_CLIENT_SECRET")
        .unwrap_or_else(|_| DEFAULT_CLIENT_SECRET.to_string())
}

// Structure to hold user info from GitHub API
#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

/// Performs the GitHub OAuth flow and returns the username and token
pub async fn github_oauth_flow() -> Result<(String, String)> {
    // Check if we're in test mode
    let (is_test_mode, test_username, test_token) = get_test_data();
    if is_test_mode {
        if let (Some(username), Some(token)) = (test_username, test_token) {
            println!("Using test credentials");
            return Ok((username, format!("Bearer {token}")));
        }
    }

    // Set up the OAuth2 client
    let github_client = get_oauth_client()?;

    // Generate the authorization URL
    let (authorize_url, csrf_state) = github_client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("user:email".to_string()))
        .url();

    // Start the redirect server
    let server = start_redirect_server()?;

    // Open the browser to authenticate the user
    println!("Opening browser for GitHub authentication...");
    println!("Authorization URL: {authorize_url}");
    if let Err(e) = open::that(authorize_url.to_string()) {
        return Err(anyhow!("Failed to open browser: {}", e));
    }

    // Wait for the callback from GitHub
    println!("Waiting for GitHub authentication...");
    let auth_code = wait_for_callback(server, &csrf_state)?;

    // Exchange the authorization code for a token
    println!("Exchanging authorization code for token...");
    let token = match github_client
        .exchange_code(AuthorizationCode::new(auth_code))
        .request_async(&cyper_async_http_client)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            println!("Error exchanging code for token: {e:?}");
            return Err(anyhow!(
                "Failed to exchange authorization code for token: {}",
                e
            ));
        }
    };

    // Get the access token as a string
    let access_token = token.access_token().secret();

    // Get the user's GitHub info using the token
    println!("Getting GitHub user information...");
    let username = get_github_username(access_token).await?;

    Ok((username, format!("Bearer {access_token}")))
}

/// Create an OAuth client for GitHub
fn get_oauth_client() -> Result<GithubOAuthClient> {
    let redirect_url = format!("http://localhost:{REDIRECT_PORT}/oauth/callback");
    println!("Redirect URL: {redirect_url}");

    Ok(BasicClient::new(ClientId::new(get_client_id()))
        .set_client_secret(ClientSecret::new(get_client_secret()))
        .set_auth_uri(AuthUrl::new(
            "https://github.com/login/oauth/authorize".to_string(),
        )?)
        .set_token_uri(TokenUrl::new(
            "https://github.com/login/oauth/access_token".to_string(),
        )?)
        .set_redirect_uri(RedirectUrl::new(redirect_url)?))
}

/// Starts a local HTTP server to receive the OAuth redirect
fn start_redirect_server() -> Result<Server> {
    let addr = SocketAddr::from_str(&format!("127.0.0.1:{REDIRECT_PORT}"))?;
    let server = Server::http(addr).map_err(|e| anyhow!("Failed to start server: {}", e))?;
    Ok(server)
}

/// Waits for and processes the OAuth callback
fn wait_for_callback(server: Server, csrf_state: &CsrfToken) -> Result<String> {
    // Wait for the callback from GitHub
    let req = server.recv()?;

    // Parse the request URL to extract the code and state
    let url_str = format!("http://localhost{}", req.url());
    let url = Url::parse(&url_str)?;

    // Extract query parameters
    let mut code = None;
    let mut state = None;

    for (key, value) in url.query_pairs() {
        if key == "code" {
            code = Some(value.to_string());
        } else if key == "state" {
            state = Some(value.to_string());
        }
    }

    // Verify the state to prevent CSRF attacks
    if state.as_deref() != Some(csrf_state.secret()) {
        // Send an error response to the browser
        let error_html = "<html><body><h1>Authentication Error</h1><p>Invalid state parameter. This could be a CSRF attack.</p></body></html>";
        req.respond(Response::from_string(error_html))?;

        return Err(anyhow!("Invalid OAuth state"));
    }

    // Check for the code and respond appropriately
    match code {
        Some(code_value) => {
            // Send a success response to the browser
            let success_html = "<h1>Authentication Successful!</h1><p>You can now close this window and return to the terminal.</p>";
            req.respond(Response::from_string(success_html))?;

            Ok(code_value)
        }
        None => {
            // Send an error response for missing code
            let error_html =
                "<h1>Authentication Error</h1><p>No authorization code received from GitHub.</p>";
            req.respond(Response::from_string(error_html))?;

            Err(anyhow!("No authorization code received"))
        }
    }
}

/// Gets the GitHub username from the user's profile
async fn get_github_username(token: &str) -> Result<String> {
    let user: GitHubUser = HttpClient::new()
        .get("https://api.github.com/user")?
        .header("User-Agent", "faasta-cli")?
        .header("Authorization", format!("Bearer {token}"))?
        .send()
        .await?
        .json()
        .await?;

    Ok(user.login)
}

async fn cyper_async_http_client(
    request: HttpRequest,
) -> std::result::Result<HttpResponse, cyper::Error> {
    let method = request.method().clone();

    let mut outbound_headers = http::HeaderMap::new();
    for (name, value) in request.headers().iter() {
        outbound_headers.append(name.clone(), value.clone());
    }

    let response = HttpClient::new()
        .request(method, request.uri().to_string())?
        .headers(outbound_headers)
        .body(request.body().clone())
        .send()
        .await?;

    let mut inbound_headers = oauth_http::HeaderMap::new();
    for (name, value) in response.headers().iter() {
        inbound_headers.append(name.clone(), value.clone());
    }

    let status_code = oauth_http::StatusCode::from_u16(response.status().as_u16())
        .expect("response status code should be valid");
    let body = response.bytes().await?.to_vec();

    let mut response_builder = oauth_http::Response::builder().status(status_code);
    {
        let headers = response_builder
            .headers_mut()
            .expect("builder should be valid");
        *headers = inbound_headers;
    }

    Ok(response_builder.body(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[compio::test]
    async fn test_oauth_flow_with_test_mode() {
        // Set up test mode
        enable_test_mode("test_user".to_string(), "test_token".to_string());

        // Run the OAuth flow
        let result = github_oauth_flow().await;

        // Check the result
        assert!(result.is_ok());
        let (username, token) = result.unwrap();
        assert_eq!(username, "test_user");
        assert_eq!(token, "Bearer test_token");
    }
}
