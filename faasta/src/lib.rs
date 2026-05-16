#![forbid(unsafe_code)]

pub mod blob;
pub mod http;
pub mod kv;
pub mod sql;

pub use anyhow::{Error, Result};
pub use faasta_macros::handler;

#[doc(hidden)]
pub mod __private {
    use crate::http::IntoResponse;
    use serde::Serialize;

    pub use wasip3;

    pub fn response_from_result<T>(
        result: crate::Result<T>,
    ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode>
    where
        T: crate::http::IntoResponse,
    {
        match result {
            Ok(response) => response.into_response(),
            Err(err) => crate::http::Json(serde_json::json!({
                "error": err.to_string(),
            }))
            .with_status(500)
            .into_response(),
        }
    }

    pub fn json_response<T>(
        status: u16,
        value: &T,
    ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode>
    where
        T: Serialize,
    {
        crate::http::json_response(status, value)
    }
}
