//! Centralized Error Types for the Proxy Module
//!
//! This module provides a unified error handling approach using `thiserror`,
//! enabling proper error propagation with the `?` operator and meaningful
//! error messages for debugging and monitoring.
//!
//! ## Error Hierarchy
//!
//! - **Client Errors (4xx)**: `InvalidRequest`, `RateLimited`
//! - **Server Errors (5xx)**: `InternalError`, `Overloaded`
//! - **Upstream Errors**: `UpstreamError`, `NetworkError`, `ParseError`
//! - **Infrastructure Errors**: `TokenError`, `CircuitBreakerOpen`, `RetryExhausted`
//!
//! ## Retry-Specific Errors
//!
//! The retry system uses specific error types for better observability:
//! - `RetryExhausted`: All retry attempts failed
//! - `CircuitBreakerOpen`: Upstream is temporarily unavailable

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;

/// Structured error response body for JSON API responses
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: bool,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<usize>,
}

/// Breakdown of errors encountered during retry attempts
#[derive(Debug, Clone, Default, Serialize)]
pub struct RetryErrorBreakdown {
    /// Count of 429 rate limit errors
    pub rate_limit_count: usize,
    /// Count of 529/503 overload errors
    pub overload_count: usize,
    /// Count of 5xx server errors (excluding 503/529)
    pub server_error_count: usize,
    /// Count of network/timeout errors
    pub network_error_count: usize,
    /// Count of 4xx client errors (excluding 429)
    pub client_error_count: usize,
}

impl RetryErrorBreakdown {
    /// Record an error by HTTP status code
    pub fn record_status(&mut self, status: u16) {
        match status {
            429 => self.rate_limit_count += 1,
            529 | 503 => self.overload_count += 1,
            500..=502 | 504..=599 => self.server_error_count += 1,
            400..=428 | 430..=499 => self.client_error_count += 1,
            _ => {}
        }
    }

    /// Record a network error
    pub fn record_network_error(&mut self) {
        self.network_error_count += 1;
    }

    /// Total error count
    pub fn total(&self) -> usize {
        self.rate_limit_count
            + self.overload_count
            + self.server_error_count
            + self.network_error_count
            + self.client_error_count
    }
}

/// Proxy error types covering all failure scenarios in request handling.
///
/// These errors are designed to:
/// 1. Provide clear categorization for monitoring and alerting
/// 2. Include sufficient context for debugging
/// 3. Map cleanly to HTTP status codes
/// 4. Support retry observability with context tracking
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

    // === Retry-Specific Error Types ===

    /// All retry attempts exhausted
    #[error("Retry exhausted after {attempts} attempts: {last_error}")]
    RetryExhausted {
        attempts: usize,
        last_error: String,
        request_id: Option<crate::proxy::middleware::request_id::RequestId>,
        /// Breakdown of errors by type for observability
        error_breakdown: Option<RetryErrorBreakdown>,
    },

    /// Circuit breaker is open - upstream temporarily unavailable
    #[error("Circuit breaker open for {endpoint}: {reason}")]
    CircuitBreakerOpen {
        endpoint: String,
        reason: String,
        /// Estimated time until circuit closes (in milliseconds for serialization)
        retry_after_ms: Option<u64>,
        request_id: Option<crate::proxy::middleware::request_id::RequestId>,
    },

    /// Response building failed (should never happen in production)
    #[error("Failed to build response: {0}")]
    ResponseBuildError(String, Option<crate::proxy::middleware::request_id::RequestId>),
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
            ProxyError::RetryExhausted { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::CircuitBreakerOpen { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::ResponseBuildError(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
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
            ProxyError::RetryExhausted { .. } => "RETRY_EXHAUSTED".to_string(),
            ProxyError::CircuitBreakerOpen { .. } => "CIRCUIT_BREAKER_OPEN".to_string(),
            ProxyError::ResponseBuildError(_, _) => "RESPONSE_BUILD_ERROR".to_string(),
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
            | ProxyError::InternalError(_, rid)
            | ProxyError::ResponseBuildError(_, rid) => rid.as_ref().map(|r| r.0.clone()),
            ProxyError::UpstreamError { request_id, .. }
            | ProxyError::RetryExhausted { request_id, .. }
            | ProxyError::CircuitBreakerOpen { request_id, .. } => {
                request_id.as_ref().map(|r| r.0.clone())
            }
        }
    }

    /// Get retry-after hint in milliseconds (if applicable)
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            ProxyError::CircuitBreakerOpen { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        }
    }

    /// Get attempt info for retry-related errors
    pub fn attempt_info(&self) -> Option<(usize, usize)> {
        match self {
            ProxyError::RetryExhausted { attempts, .. } => Some((*attempts, *attempts)),
            _ => None,
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
            ProxyError::RetryExhausted {
                attempts,
                last_error,
                error_breakdown,
                ..
            } => ProxyError::RetryExhausted {
                attempts,
                last_error,
                request_id: Some(rid),
                error_breakdown,
            },
            ProxyError::CircuitBreakerOpen {
                endpoint,
                reason,
                retry_after_ms,
                ..
            } => ProxyError::CircuitBreakerOpen {
                endpoint,
                reason,
                retry_after_ms,
                request_id: Some(rid),
            },
            ProxyError::ResponseBuildError(m, _) => ProxyError::ResponseBuildError(m, Some(rid)),
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

    /// Create a NetworkError from a string.
    pub fn network_error(msg: impl Into<String>) -> Self {
        ProxyError::NetworkError(msg.into(), None)
    }

    /// Create a RetryExhausted error with full context.
    pub fn retry_exhausted(
        attempts: usize,
        last_error: impl Into<String>,
        error_breakdown: Option<RetryErrorBreakdown>,
    ) -> Self {
        ProxyError::RetryExhausted {
            attempts,
            last_error: last_error.into(),
            request_id: None,
            error_breakdown,
        }
    }

    /// Create a CircuitBreakerOpen error.
    pub fn circuit_breaker_open(
        endpoint: impl Into<String>,
        reason: impl Into<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        ProxyError::CircuitBreakerOpen {
            endpoint: endpoint.into(),
            reason: reason.into(),
            retry_after_ms: retry_after.map(|d| d.as_millis() as u64),
            request_id: None,
        }
    }

    /// Create a ResponseBuildError from a string.
    pub fn response_build_error(msg: impl Into<String>) -> Self {
        ProxyError::ResponseBuildError(msg.into(), None)
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProxyError::RateLimited(_, _)
                | ProxyError::Overloaded(_, _)
                | ProxyError::NetworkError(_, _)
                | ProxyError::UpstreamError { status: 429 | 503 | 529 | 500 | 502 | 504, .. }
        )
    }

    /// Check if this is a rate limit error
    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            ProxyError::RateLimited(_, _) | ProxyError::UpstreamError { status: 429, .. }
        )
    }

    /// Check if this is an overload error
    pub fn is_overload(&self) -> bool {
        matches!(
            self,
            ProxyError::Overloaded(_, _) | ProxyError::UpstreamError { status: 529 | 503, .. }
        )
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = self.to_string();
        let code = self.error_code();
        let request_id = self.request_id();
        let retry_after_ms = self.retry_after_ms();
        let (attempt, max_attempts) = self.attempt_info().map_or((None, None), |(a, m)| (Some(a), Some(m)));

        // Log the error for monitoring with structured fields
        tracing::error!(
            error_type = %code,
            status = %status,
            message = %message,
            request_id = ?request_id,
            retry_after_ms = ?retry_after_ms,
            attempt = ?attempt,
            "Proxy error response"
        );

        let body = ErrorResponse {
            error: true,
            code,
            message,
            request_id,
            retry_after_ms,
            attempt,
            max_attempts,
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

/// Convert from std::io::Error to ProxyError
impl From<std::io::Error> for ProxyError {
    fn from(err: std::io::Error) -> Self {
        ProxyError::InternalError(format!("IO error: {err}"), None)
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
        assert_eq!(
            ProxyError::retry_exhausted(3, "failed", None).status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ProxyError::circuit_breaker_open("test", "too many failures", Some(Duration::from_secs(30))).status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn test_error_display() {
        let err = ProxyError::invalid_request("missing field");
        assert_eq!(err.to_string(), "Invalid request: missing field");

        let err = ProxyError::upstream_error(500, "internal error");
        assert_eq!(err.to_string(), "Upstream error (500): internal error");

        let err = ProxyError::retry_exhausted(5, "connection refused", None);
        assert_eq!(
            err.to_string(),
            "Retry exhausted after 5 attempts: connection refused"
        );

        let err = ProxyError::circuit_breaker_open("api.example.com", "high error rate", None);
        assert_eq!(
            err.to_string(),
            "Circuit breaker open for api.example.com: high error rate"
        );
    }

    #[test]
    fn test_error_codes() {
        assert_eq!(ProxyError::invalid_request("test").error_code(), "INVALID_REQUEST");
        assert_eq!(ProxyError::retry_exhausted(1, "err", None).error_code(), "RETRY_EXHAUSTED");
        assert_eq!(
            ProxyError::circuit_breaker_open("ep", "reason", None).error_code(),
            "CIRCUIT_BREAKER_OPEN"
        );
    }

    #[test]
    fn test_retry_error_breakdown() {
        let mut breakdown = RetryErrorBreakdown::default();
        breakdown.record_status(429);
        breakdown.record_status(429);
        breakdown.record_status(503);
        breakdown.record_status(500);
        breakdown.record_network_error();

        assert_eq!(breakdown.rate_limit_count, 2);
        assert_eq!(breakdown.overload_count, 1);
        assert_eq!(breakdown.server_error_count, 1);
        assert_eq!(breakdown.network_error_count, 1);
        assert_eq!(breakdown.total(), 5);
    }

    #[test]
    fn test_is_retryable() {
        assert!(ProxyError::RateLimited("test".into(), None).is_retryable());
        assert!(ProxyError::Overloaded("test".into(), None).is_retryable());
        assert!(ProxyError::network_error("timeout").is_retryable());
        assert!(ProxyError::upstream_error(429, "rate limited").is_retryable());
        assert!(ProxyError::upstream_error(503, "unavailable").is_retryable());

        assert!(!ProxyError::invalid_request("bad input").is_retryable());
        assert!(!ProxyError::upstream_error(400, "bad request").is_retryable());
        assert!(!ProxyError::upstream_error(404, "not found").is_retryable());
    }

    #[test]
    fn test_is_rate_limited() {
        assert!(ProxyError::RateLimited("test".into(), None).is_rate_limited());
        assert!(ProxyError::upstream_error(429, "too many").is_rate_limited());
        assert!(!ProxyError::Overloaded("test".into(), None).is_rate_limited());
    }

    #[test]
    fn test_is_overload() {
        assert!(ProxyError::Overloaded("test".into(), None).is_overload());
        assert!(ProxyError::upstream_error(529, "overloaded").is_overload());
        assert!(ProxyError::upstream_error(503, "unavailable").is_overload());
        assert!(!ProxyError::RateLimited("test".into(), None).is_overload());
    }

    #[test]
    fn test_from_reqwest_error() {
        // This test just verifies the From trait is properly implemented
        // We can't easily construct reqwest errors in tests
    }

    #[test]
    fn test_retry_after_ms() {
        let err = ProxyError::circuit_breaker_open("ep", "reason", Some(Duration::from_secs(30)));
        assert_eq!(err.retry_after_ms(), Some(30000));

        let err = ProxyError::circuit_breaker_open("ep", "reason", None);
        assert_eq!(err.retry_after_ms(), None);

        let err = ProxyError::invalid_request("test");
        assert_eq!(err.retry_after_ms(), None);
    }
}
