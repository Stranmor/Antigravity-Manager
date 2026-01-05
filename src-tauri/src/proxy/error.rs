//! Centralized Error Types for the Proxy Module
//!
//! This module provides a unified error handling approach using `thiserror`,
//! enabling proper error propagation with the `?` operator and meaningful
//! error messages for debugging and monitoring.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

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
    InvalidRequest(String),

    /// Authentication/authorization failure
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// Token manager errors (no tokens available, refresh failures)
    #[error("Token error: {0}")]
    TokenError(String),

    /// Upstream API returned an error
    #[error("Upstream error ({status}): {message}")]
    UpstreamError { status: u16, message: String },

    /// Rate limiting triggered
    #[error("Rate limited: {0}")]
    RateLimited(String),

    /// Server overloaded (529 errors)
    #[error("Server overloaded: {0}")]
    Overloaded(String),

    /// Request transformation failed
    #[error("Request transformation failed: {0}")]
    TransformError(String),

    /// Response parsing failed
    #[error("Failed to parse upstream response: {0}")]
    ParseError(String),

    /// Network/connection errors
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Internal server error (unexpected failures)
    #[error("Internal error: {0}")]
    InternalError(String),

    /// Service unavailable (token pool exhausted, etc.)
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl ProxyError {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ProxyError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ProxyError::AuthError(_) => StatusCode::UNAUTHORIZED,
            ProxyError::TokenError(_) => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::UpstreamError { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            ProxyError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            ProxyError::Overloaded(_) => {
                // 529 is not a standard StatusCode, use 503 as fallback
                StatusCode::SERVICE_UNAVAILABLE
            }
            ProxyError::TransformError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::ParseError(_) => StatusCode::BAD_GATEWAY,
            ProxyError::NetworkError(_) => StatusCode::BAD_GATEWAY,
            ProxyError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Create an InvalidRequest error from a string.
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        ProxyError::InvalidRequest(msg.into())
    }

    /// Create a TokenError from a string.
    pub fn token_error(msg: impl Into<String>) -> Self {
        ProxyError::TokenError(msg.into())
    }

    /// Create an UpstreamError with status and message.
    pub fn upstream_error(status: u16, message: impl Into<String>) -> Self {
        ProxyError::UpstreamError {
            status,
            message: message.into(),
        }
    }

    /// Create a ParseError from a string.
    pub fn parse_error(msg: impl Into<String>) -> Self {
        ProxyError::ParseError(msg.into())
    }

    /// Create an InternalError from a string.
    pub fn internal_error(msg: impl Into<String>) -> Self {
        ProxyError::InternalError(msg.into())
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = self.to_string();

        // Log the error for monitoring
        tracing::error!(
            error_type = ?std::mem::discriminant(&self),
            status = %status,
            message = %body,
            "Proxy error response"
        );

        (status, body).into_response()
    }
}

/// Convert from reqwest errors to ProxyError
impl From<reqwest::Error> for ProxyError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            ProxyError::NetworkError(format!("Request timeout: {}", err))
        } else if err.is_connect() {
            ProxyError::NetworkError(format!("Connection failed: {}", err))
        } else if err.is_decode() {
            ProxyError::ParseError(format!("Response decode failed: {}", err))
        } else {
            ProxyError::NetworkError(format!("Network error: {}", err))
        }
    }
}

/// Convert from serde_json errors to ProxyError
impl From<serde_json::Error> for ProxyError {
    fn from(err: serde_json::Error) -> Self {
        ProxyError::ParseError(format!("JSON error: {}", err))
    }
}

/// Result type alias for proxy operations
pub type ProxyResult<T> = Result<T, ProxyError>;

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
            ProxyError::RateLimited("test".into()).status_code(),
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
