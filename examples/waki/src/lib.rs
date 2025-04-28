use waki::Client;
use waki::{handler, ErrorCode, Request, Response};

// External API proxy endpoint
#[handler]
fn external_api(req: Request) -> Result<Response, ErrorCode> {
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
