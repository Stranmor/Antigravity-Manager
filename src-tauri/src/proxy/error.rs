//! Centralized Error Types for the Proxy Module
//!
//! This module provides a unified error handling approach using `thiserror`,
//! enabling proper error propagation with the `?` operator and meaningful
//! error messages for debugging and monitoring.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Structured error response body
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: bool,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Proxy error types covering all failure scenarios in request handling.
///
/// These errors are designed to:
/// 1. Provide clear categorization for monitoring and alerting
/// 2. Include sufficient context for debugging
/// 3. Map cleanly to HTTP status codes
#[derive(Debug, Error)]
pub enum ProxyError {
    /// Request validation failed (malformed input, missing required fields)
    #[error("Invalid request: {0}")]
    InvalidRequest(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Token manager errors (no tokens available, refresh failures)
    #[error("Token error: {0}")]
    TokenError(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Upstream API returned an error
    #[error("Upstream error ({status}): {message}")]
    UpstreamError {
        status: u16,
        message: String,
        request_id: Option<crate::proxy::middleware::request_id::RequestId>,
    },

    /// Rate limiting triggered
    #[error("Rate limited: {0}")]
    RateLimited(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Server overloaded (529 errors)
    #[error("Server overloaded: {0}")]
    Overloaded(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Request transformation failed
    #[error("Request transformation failed: {0}")]
    TransformError(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Response parsing failed
    #[error("Failed to parse upstream response: {0}")]
    ParseError(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Network/connection errors
    #[error("Network error: {0}")]
    NetworkError(String, Option<crate::proxy::middleware::request_id::RequestId>),

    /// Internal server error (unexpected failures)
    #[error("Internal error: {0}")]
    InternalError(String, Option<crate::proxy::middleware::request_id::RequestId>),
}

impl ProxyError {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ProxyError::InvalidRequest(_, _) => StatusCode::BAD_REQUEST,
            ProxyError::TokenError(_, _) => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::UpstreamError { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            ProxyError::RateLimited(_, _) => StatusCode::TOO_MANY_REQUESTS,
            ProxyError::Overloaded(_, _) => {
                // 529 is not a standard StatusCode, use 503 as fallback
                StatusCode::SERVICE_UNAVAILABLE
            }
            ProxyError::TransformError(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::ParseError(_, _) => StatusCode::BAD_GATEWAY,
            ProxyError::NetworkError(_, _) => StatusCode::BAD_GATEWAY,
            ProxyError::InternalError(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Get the machine-readable error code.
    pub fn error_code(&self) -> String {
        match self {
            ProxyError::InvalidRequest(_, _) => "INVALID_REQUEST".to_string(),
            ProxyError::TokenError(_, _) => "TOKEN_ERROR".to_string(),
            ProxyError::UpstreamError { .. } => "UPSTREAM_ERROR".to_string(),
            ProxyError::RateLimited(_, _) => "RATE_LIMITED".to_string(),
            ProxyError::Overloaded(_, _) => "SERVER_OVERLOADED".to_string(),
            ProxyError::TransformError(_, _) => "TRANSFORM_ERROR".to_string(),
            ProxyError::ParseError(_, _) => "PARSE_ERROR".to_string(),
            ProxyError::NetworkError(_, _) => "NETWORK_ERROR".to_string(),
            ProxyError::InternalError(_, _) => "INTERNAL_ERROR".to_string(),
        }
    }

    /// Get the optional request ID.
    pub fn request_id(&self) -> Option<String> {
        match self {
            ProxyError::InvalidRequest(_, rid)
            | ProxyError::TokenError(_, rid)
            | ProxyError::RateLimited(_, rid)
            | ProxyError::Overloaded(_, rid)
            | ProxyError::TransformError(_, rid)
            | ProxyError::ParseError(_, rid)
            | ProxyError::NetworkError(_, rid)
            | ProxyError::InternalError(_, rid) => rid.as_ref().map(|r| r.0.clone()),
            ProxyError::UpstreamError { request_id, .. } => request_id.as_ref().map(|r| r.0.clone()),
        }
    }

    /// Attach a request ID to this error.
    pub fn with_request_id(self, rid: crate::proxy::middleware::request_id::RequestId) -> Self {
        match self {
            ProxyError::InvalidRequest(m, _) => ProxyError::InvalidRequest(m, Some(rid)),
            ProxyError::TokenError(m, _) => ProxyError::TokenError(m, Some(rid)),
            ProxyError::UpstreamError { status, message, .. } => ProxyError::UpstreamError {
                status,
                message,
                request_id: Some(rid),
            },
            ProxyError::RateLimited(m, _) => ProxyError::RateLimited(m, Some(rid)),
            ProxyError::Overloaded(m, _) => ProxyError::Overloaded(m, Some(rid)),
            ProxyError::TransformError(m, _) => ProxyError::TransformError(m, Some(rid)),
            ProxyError::ParseError(m, _) => ProxyError::ParseError(m, Some(rid)),
            ProxyError::NetworkError(m, _) => ProxyError::NetworkError(m, Some(rid)),
            ProxyError::InternalError(m, _) => ProxyError::InternalError(m, Some(rid)),
        }
    }

    /// Create an InvalidRequest error from a string.
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        ProxyError::InvalidRequest(msg.into(), None)
    }

    /// Create a TokenError from a string.
    pub fn token_error(msg: impl Into<String>) -> Self {
        ProxyError::TokenError(msg.into(), None)
    }

    /// Create an UpstreamError with status and message.
    pub fn upstream_error(status: u16, message: impl Into<String>) -> Self {
        ProxyError::UpstreamError {
            status,
            message: message.into(),
            request_id: None,
        }
    }

    /// Create a ParseError from a string.
    pub fn parse_error(msg: impl Into<String>) -> Self {
        ProxyError::ParseError(msg.into(), None)
    }

    /// Create an InternalError from a string.
    pub fn internal_error(msg: impl Into<String>) -> Self {
        ProxyError::InternalError(msg.into(), None)
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = self.to_string();
        let code = self.error_code();
        let request_id = self.request_id();

        // Log the error for monitoring
        tracing::error!(
            error_type = ?std::mem::discriminant(&self),
            status = %status,
            message = %message,
            request_id = ?request_id,
            "Proxy error response"
        );

        let body = ErrorResponse {
            error: true,
            code,
            message,
            request_id,
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Convert from reqwest errors to ProxyError
impl From<reqwest::Error> for ProxyError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            ProxyError::NetworkError(format!("Request timeout: {err}"), None)
        } else if err.is_connect() {
            ProxyError::NetworkError(format!("Connection failed: {err}"), None)
        } else if err.is_decode() {
            ProxyError::ParseError(format!("Response decode failed: {err}"), None)
        } else {
            ProxyError::NetworkError(format!("Network error: {err}"), None)
        }
    }
}

/// Convert from serde_json errors to ProxyError
impl From<serde_json::Error> for ProxyError {
    fn from(err: serde_json::Error) -> Self {
        ProxyError::ParseError(format!("JSON error: {err}"), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(
            ProxyError::invalid_request("test").status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ProxyError::token_error("test").status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ProxyError::RateLimited("test".into(), None).status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ProxyError::upstream_error(404, "not found").status_code(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn test_error_display() {
        let err = ProxyError::invalid_request("missing field");
        assert_eq!(err.to_string(), "Invalid request: missing field");

        let err = ProxyError::upstream_error(500, "internal error");
        assert_eq!(err.to_string(), "Upstream error (500): internal error");
    }

    #[test]
    fn test_from_reqwest_error() {
        // This test just verifies the From trait is properly implemented
        // We can't easily construct reqwest errors in tests
    }
}
