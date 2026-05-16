use serde::Serialize;
use wasip3::http::types::{ErrorCode, Fields, Response};
use wasip3::{wit_bindgen, wit_future, wit_stream};

pub struct Html<T>(pub T);
pub struct Json<T>(pub T);

pub trait IntoResponse {
    fn into_response(self) -> Result<Response, ErrorCode>;
}

impl<T> Html<T> {
    pub fn with_status(self, status: u16) -> ResponseWithStatus<Self> {
        ResponseWithStatus {
            status,
            response: self,
        }
    }
}

impl<T> Json<T> {
    pub fn with_status(self, status: u16) -> ResponseWithStatus<Self> {
        ResponseWithStatus {
            status,
            response: self,
        }
    }
}

pub struct ResponseWithStatus<T> {
    status: u16,
    response: T,
}

impl<T> IntoResponse for Html<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Result<Response, ErrorCode> {
        body_response(200, "text/html; charset=utf-8", self.0.into().into_bytes())
    }
}

impl<T> IntoResponse for Json<T>
where
    T: Serialize,
{
    fn into_response(self) -> Result<Response, ErrorCode> {
        json_response(200, &self.0)
    }
}

impl<T> IntoResponse for ResponseWithStatus<Html<T>>
where
    T: Into<String>,
{
    fn into_response(self) -> Result<Response, ErrorCode> {
        body_response(
            self.status,
            "text/html; charset=utf-8",
            self.response.0.into().into_bytes(),
        )
    }
}

impl<T> IntoResponse for ResponseWithStatus<Json<T>>
where
    T: Serialize,
{
    fn into_response(self) -> Result<Response, ErrorCode> {
        json_response(self.status, &self.response.0)
    }
}

impl IntoResponse for Response {
    fn into_response(self) -> Result<Response, ErrorCode> {
        Ok(self)
    }
}

#[doc(hidden)]
pub fn json_response<T>(status: u16, value: &T) -> Result<Response, ErrorCode>
where
    T: Serialize,
{
    let body = serde_json::to_vec(value)
        .map_err(|err| ErrorCode::InternalError(Some(format!("serializing response: {err}"))))?;
    body_response(status, "application/json", body)
}

fn body_response(status: u16, content_type: &str, body: Vec<u8>) -> Result<Response, ErrorCode> {
    let headers = Fields::new();
    headers
        .set("content-type", &[content_type.as_bytes().to_vec()])
        .map_err(|err| ErrorCode::InternalError(Some(format!("setting header: {err:?}"))))?;
    headers
        .set("content-length", &[body.len().to_string().into_bytes()])
        .map_err(|err| ErrorCode::InternalError(Some(format!("setting header: {err:?}"))))?;

    let (mut body_tx, body_rx) = wit_stream::new();
    let (body_result_tx, body_result_rx) = wit_future::new(|| Ok(None));
    let (response, _response_result) = Response::new(headers, Some(body_rx), body_result_rx);
    response
        .set_status_code(status)
        .map_err(|()| ErrorCode::InternalError(Some("setting status code".to_string())))?;
    drop(body_result_tx);

    wit_bindgen::spawn(async move {
        let remaining = body_tx.write_all(body).await;
        assert!(remaining.is_empty());
    });

    Ok(response)
}
