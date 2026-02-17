# FaaSTa HTTPS API Documentation

This document describes the HTTPS API endpoints available in the FaaSTa service.

All API endpoints are versioned with a `/v1/` prefix. Any request to paths that begin with `/v1/` but don't match valid endpoints will return a 403 Forbidden response.

## Authentication

All API endpoints require authentication using a GitHub authentication token. The token must be provided in the `Authorization` header with the `Bearer` prefix:

```
Authorization: Bearer your_github_token
```

## Endpoints

### Publish Function

Publishes a native Linux shared library function to the FaaSTa service.

**URL**: `/v1/publish/{function_name}`

**Path Parameters**:
- `function_name` (required): The name for the function. Must contain only alphanumeric characters, hyphens, and underscores.

**Method**: `POST`

**Headers**:
- `Authorization` (required): Your GitHub authentication token with `Bearer` prefix.
- `Content-Type`: `application/octet-stream` (recommended)

**Body**: Raw shared library binary file (`.so`, built for `x86_64-unknown-linux-gnu`)

**Success Response**:
- **Status Code**: 200 OK
- **Content**: JSON object with the following structure:
  ```json
  {
    "success": true,
    "message": "Function 'function-name' published successfully"
  }
  ```

**Error Responses**:
- **Status Code**: 400 Bad Request
  - Empty artifact file
  - Invalid function name
  - Artifact file too large
- **Status Code**: 401 Unauthorized
  - Missing Authorization header
  - Empty Authorization token
  - Invalid GitHub authentication token
- **Status Code**: 403 Forbidden
  - A function with this name already exists and belongs to another user
  - Maximum project limit reached
  - Invalid API endpoint (when accessing invalid paths with the v1 prefix)
- **Status Code**: 500 Internal Server Error
  - Server-side error

**Example Usage**:
```bash
curl -X POST "https://faasta.lol/v1/publish/my-function" \
  -H "Authorization: Bearer your_github_token" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@/path/to/your/libmy_function.so"
```

## Testing the API

You can use the provided test script in the examples directory to test the HTTPS API:

```bash
./examples/test_publish_https.sh -n my-function -t your_github_token -f path/to/libmy_function.so
```

For more details on the script options:

```bash
./examples/test_publish_https.sh --help
```

## Error Format

All errors are returned as JSON objects with the following structure:

```json
{
  "success": false,
  "error": "Description of the error"
}
```

## Function Usage After Publishing

After successfully publishing a function, it will be available at:
- `https://function-name.faasta.lol`
- `https://faasta.lol/function-name`

Replace `function-name` with the name you provided when publishing the function.
