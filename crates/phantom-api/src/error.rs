//! API error types and response formatting.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Standard API error type.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Client sent a bad request.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Internal server error.
    #[error("internal error: {0}")]
    Internal(String),
}

/// JSON error body returned to clients.
#[derive(Debug, Serialize)]
struct ErrorBody {
    success: bool,
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        let body = ErrorBody {
            success: false,
            error: message,
        };

        (status, Json(body)).into_response()
    }
}

/// Standard API success response.
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: T,
}

impl<T: Serialize> ApiResponse<T> {
    /// Wraps data in a success response.
    pub fn ok(data: T) -> Json<Self> {
        Json(Self {
            success: true,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_display() {
        let e = ApiError::NotFound("order 123".to_string());
        assert_eq!(e.to_string(), "not found: order 123");

        let e = ApiError::BadRequest("missing field".to_string());
        assert_eq!(e.to_string(), "bad request: missing field");

        let e = ApiError::Internal("db connection failed".to_string());
        assert_eq!(e.to_string(), "internal error: db connection failed");
    }

    #[test]
    fn api_response_serialization() {
        let response = ApiResponse {
            success: true,
            data: "hello",
        };
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"data\":\"hello\""));
    }
}
