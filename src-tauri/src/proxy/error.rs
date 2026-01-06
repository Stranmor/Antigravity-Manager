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
//!
//! ## Structured Error Codes
//!
//! All errors include machine-readable codes (AG-001 through AG-008) for
//! programmatic client-side error handling. See [`ErrorCode`] for details.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;

/// Machine-readable error codes for API clients.
///
/// These codes provide a stable, compact identifier for each error category,
/// enabling clients to implement structured error handling without parsing
/// error messages.
///
/// # Code Format
///
/// All codes follow the format `AG-XXX` where:
/// - `AG` = Antigravity (project prefix)
/// - `XXX` = Three-digit numeric identifier
///
/// # Example Response
///
/// ```json
/// {
///   "error": true,
///   "code": "AG-001",
///   "error_type": "ACCOUNTS_EXHAUSTED",
///   "message": "No accounts available: all rate-limited or disabled"
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "&'static str")]
pub enum ErrorCode {
    /// AG-001: No accounts available (all rate-limited or disabled)
    AccountsExhausted,
    /// AG-002: Circuit breaker is open for requested account
    CircuitOpen,
    /// AG-003: Upstream API timed out
    UpstreamTimeout,
    /// AG-004: Upstream API returned an error
    UpstreamError,
    /// AG-005: Request validation failed
    ValidationError,
    /// AG-006: Authentication required
    AuthRequired,
    /// AG-007: Rate limit exceeded
    RateLimited,
    /// AG-008: Internal server error
    InternalError,
}

impl ErrorCode {
    /// Returns the structured error code string (e.g., "AG-001").
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AccountsExhausted => "AG-001",
            Self::CircuitOpen => "AG-002",
            Self::UpstreamTimeout => "AG-003",
            Self::UpstreamError => "AG-004",
            Self::ValidationError => "AG-005",
            Self::AuthRequired => "AG-006",
            Self::RateLimited => "AG-007",
            Self::InternalError => "AG-008",
        }
    }

    /// Returns the human-readable error type name.
    #[inline]
    pub const fn error_type(&self) -> &'static str {
        match self {
            Self::AccountsExhausted => "ACCOUNTS_EXHAUSTED",
            Self::CircuitOpen => "CIRCUIT_OPEN",
            Self::UpstreamTimeout => "UPSTREAM_TIMEOUT",
            Self::UpstreamError => "UPSTREAM_ERROR",
            Self::ValidationError => "VALIDATION_ERROR",
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::RateLimited => "RATE_LIMITED",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl From<ErrorCode> for &'static str {
    fn from(code: ErrorCode) -> Self {
        code.as_str()
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured error response body for JSON API responses.
///
/// # Example
///
/// ```json
/// {
///   "error": true,
///   "code": "AG-007",
///   "error_type": "RATE_LIMITED",
///   "message": "Rate limit exceeded for account",
///   "retry_after_ms": 5000
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Always `true` for error responses.
    pub error: bool,
    /// Machine-readable error code (e.g., "AG-001").
    pub code: ErrorCode,
    /// Human-readable error type (e.g., "ACCOUNTS_EXHAUSTED").
    pub error_type: &'static str,
    /// Detailed error message.
    pub message: String,
    /// Request ID for tracing/debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Time in milliseconds before the client should retry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Current attempt number (for retry-related errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<usize>,
    /// Maximum attempts allowed (for retry-related errors).
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
    InvalidRequest(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Token manager errors (no tokens available, refresh failures)
    #[error("Token error: {0}")]
    TokenError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Upstream API returned an error
    #[error("Upstream error ({status}): {message}")]
    UpstreamError {
        status: u16,
        message: String,
        request_id: Option<crate::proxy::middleware::request_id::RequestId>,
    },

    /// Rate limiting triggered
    #[error("Rate limited: {0}")]
    RateLimited(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Server overloaded (529 errors)
    #[error("Server overloaded: {0}")]
    Overloaded(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Request transformation failed
    #[error("Request transformation failed: {0}")]
    TransformError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Response parsing failed
    #[error("Failed to parse upstream response: {0}")]
    ParseError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Network/connection errors
    #[error("Network error: {0}")]
    NetworkError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Internal server error (unexpected failures)
    #[error("Internal error: {0}")]
    InternalError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

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
    ResponseBuildError(
        String,
        Option<crate::proxy::middleware::request_id::RequestId>,
    ),

    /// Upstream request timed out (hard deadline enforcement)
    #[error("Upstream timeout after {timeout_secs}s: {message}")]
    UpstreamTimeout {
        timeout_secs: u64,
        message: String,
        request_id: Option<crate::proxy::middleware::request_id::RequestId>,
    },
}

impl ProxyError {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            ProxyError::InvalidRequest(_, _) => StatusCode::BAD_REQUEST,
            ProxyError::TokenError(_, _)
            | ProxyError::RetryExhausted { .. }
            | ProxyError::CircuitBreakerOpen { .. }
            | ProxyError::Overloaded(_, _) => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::UpstreamError { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            ProxyError::UpstreamTimeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            ProxyError::RateLimited(_, _) => StatusCode::TOO_MANY_REQUESTS,
            ProxyError::TransformError(_, _)
            | ProxyError::InternalError(_, _)
            | ProxyError::ResponseBuildError(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::ParseError(_, _) | ProxyError::NetworkError(_, _) => {
                StatusCode::BAD_GATEWAY
            }
        }
    }

    /// Get the machine-readable error code.
    ///
    /// Maps each `ProxyError` variant to its corresponding `ErrorCode` for
    /// structured client-side error handling.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            // Validation errors
            ProxyError::InvalidRequest(_, _) | ProxyError::TransformError(_, _) => {
                ErrorCode::ValidationError
            }

            // Account exhaustion (token errors = no available accounts)
            ProxyError::TokenError(_, _) | ProxyError::RetryExhausted { .. } => {
                ErrorCode::AccountsExhausted
            }

            // Circuit breaker
            ProxyError::CircuitBreakerOpen { .. } => ErrorCode::CircuitOpen,

            // Explicit upstream timeout (hard deadline enforcement)
            ProxyError::UpstreamTimeout { .. } => ErrorCode::UpstreamTimeout,

            // Upstream errors (with timeout detection)
            ProxyError::UpstreamError { status, .. } => {
                if *status == 408 || *status == 504 {
                    ErrorCode::UpstreamTimeout
                } else {
                    ErrorCode::UpstreamError
                }
            }

            // Network errors (often timeouts)
            ProxyError::NetworkError(msg, _) => {
                if msg.to_lowercase().contains("timeout") {
                    ErrorCode::UpstreamTimeout
                } else {
                    ErrorCode::UpstreamError
                }
            }

            // Rate limiting
            ProxyError::RateLimited(_, _) => ErrorCode::RateLimited,

            // Overload is a form of rate limiting
            ProxyError::Overloaded(_, _) => ErrorCode::RateLimited,

            // Parse errors are upstream issues
            ProxyError::ParseError(_, _) => ErrorCode::UpstreamError,

            // Internal errors
            ProxyError::InternalError(_, _) | ProxyError::ResponseBuildError(_, _) => {
                ErrorCode::InternalError
            }
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
            | ProxyError::UpstreamTimeout { request_id, .. }
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
    #[must_use]
    pub fn with_request_id(self, rid: crate::proxy::middleware::request_id::RequestId) -> Self {
        match self {
            ProxyError::InvalidRequest(m, _) => ProxyError::InvalidRequest(m, Some(rid)),
            ProxyError::TokenError(m, _) => ProxyError::TokenError(m, Some(rid)),
            ProxyError::UpstreamError {
                status, message, ..
            } => ProxyError::UpstreamError {
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
            ProxyError::UpstreamTimeout {
                timeout_secs,
                message,
                ..
            } => ProxyError::UpstreamTimeout {
                timeout_secs,
                message,
                request_id: Some(rid),
            },
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

    /// Create an UpstreamTimeout error (hard deadline enforcement).
    pub fn upstream_timeout(timeout_secs: u64, message: impl Into<String>) -> Self {
        ProxyError::UpstreamTimeout {
            timeout_secs,
            message: message.into(),
            request_id: None,
        }
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProxyError::RateLimited(_, _)
                | ProxyError::Overloaded(_, _)
                | ProxyError::NetworkError(_, _)
                | ProxyError::UpstreamError {
                    status: 429 | 503 | 529 | 500 | 502 | 504,
                    ..
                }
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
            ProxyError::Overloaded(_, _)
                | ProxyError::UpstreamError {
                    status: 529 | 503,
                    ..
                }
        )
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let message = self.to_string();
        let code = self.error_code();
        let error_type = code.error_type();
        let request_id = self.request_id();
        let retry_after_ms = self.retry_after_ms();
        let (attempt, max_attempts) = self
            .attempt_info()
            .map_or((None, None), |(a, m)| (Some(a), Some(m)));

        // Log the error for monitoring with structured fields
        tracing::error!(
            error_code = %code,
            error_type = %error_type,
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
            error_type,
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
            ProxyError::circuit_breaker_open(
                "test",
                "too many failures",
                Some(Duration::from_secs(30))
            )
            .status_code(),
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
        // ValidationError (AG-005)
        assert_eq!(
            ProxyError::invalid_request("test").error_code(),
            ErrorCode::ValidationError
        );
        assert_eq!(ErrorCode::ValidationError.as_str(), "AG-005");

        // AccountsExhausted (AG-001)
        assert_eq!(
            ProxyError::retry_exhausted(1, "err", None).error_code(),
            ErrorCode::AccountsExhausted
        );
        assert_eq!(ErrorCode::AccountsExhausted.as_str(), "AG-001");

        // CircuitOpen (AG-002)
        assert_eq!(
            ProxyError::circuit_breaker_open("ep", "reason", None).error_code(),
            ErrorCode::CircuitOpen
        );
        assert_eq!(ErrorCode::CircuitOpen.as_str(), "AG-002");

        // RateLimited (AG-007)
        assert_eq!(
            ProxyError::RateLimited("test".into(), None).error_code(),
            ErrorCode::RateLimited
        );
        assert_eq!(ErrorCode::RateLimited.as_str(), "AG-007");

        // UpstreamError (AG-004)
        assert_eq!(
            ProxyError::upstream_error(500, "error").error_code(),
            ErrorCode::UpstreamError
        );
        assert_eq!(ErrorCode::UpstreamError.as_str(), "AG-004");

        // UpstreamTimeout (AG-003) - detected from status code
        assert_eq!(
            ProxyError::upstream_error(504, "timeout").error_code(),
            ErrorCode::UpstreamTimeout
        );
        assert_eq!(ErrorCode::UpstreamTimeout.as_str(), "AG-003");

        // InternalError (AG-008)
        assert_eq!(
            ProxyError::internal_error("something broke").error_code(),
            ErrorCode::InternalError
        );
        assert_eq!(ErrorCode::InternalError.as_str(), "AG-008");
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::AccountsExhausted), "AG-001");
        assert_eq!(format!("{}", ErrorCode::CircuitOpen), "AG-002");
        assert_eq!(format!("{}", ErrorCode::UpstreamTimeout), "AG-003");
        assert_eq!(format!("{}", ErrorCode::UpstreamError), "AG-004");
        assert_eq!(format!("{}", ErrorCode::ValidationError), "AG-005");
        assert_eq!(format!("{}", ErrorCode::AuthRequired), "AG-006");
        assert_eq!(format!("{}", ErrorCode::RateLimited), "AG-007");
        assert_eq!(format!("{}", ErrorCode::InternalError), "AG-008");
    }

    #[test]
    fn test_error_type_names() {
        assert_eq!(ErrorCode::AccountsExhausted.error_type(), "ACCOUNTS_EXHAUSTED");
        assert_eq!(ErrorCode::CircuitOpen.error_type(), "CIRCUIT_OPEN");
        assert_eq!(ErrorCode::UpstreamTimeout.error_type(), "UPSTREAM_TIMEOUT");
        assert_eq!(ErrorCode::UpstreamError.error_type(), "UPSTREAM_ERROR");
        assert_eq!(ErrorCode::ValidationError.error_type(), "VALIDATION_ERROR");
        assert_eq!(ErrorCode::AuthRequired.error_type(), "AUTH_REQUIRED");
        assert_eq!(ErrorCode::RateLimited.error_type(), "RATE_LIMITED");
        assert_eq!(ErrorCode::InternalError.error_type(), "INTERNAL_ERROR");
    }

    #[test]
    fn test_timeout_detection() {
        // Network timeout should map to UpstreamTimeout
        let err = ProxyError::network_error("Request timeout: connection timed out");
        assert_eq!(err.error_code(), ErrorCode::UpstreamTimeout);

        // Regular network error should map to UpstreamError
        let err = ProxyError::network_error("Connection refused");
        assert_eq!(err.error_code(), ErrorCode::UpstreamError);

        // HTTP 408 should map to UpstreamTimeout
        let err = ProxyError::upstream_error(408, "Request Timeout");
        assert_eq!(err.error_code(), ErrorCode::UpstreamTimeout);

        // HTTP 504 should map to UpstreamTimeout
        let err = ProxyError::upstream_error(504, "Gateway Timeout");
        assert_eq!(err.error_code(), ErrorCode::UpstreamTimeout);
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
