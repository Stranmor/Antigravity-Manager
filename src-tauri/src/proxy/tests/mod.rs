//! Proxy module integration tests
//!
//! Comprehensive test suite covering:
//! - Health monitoring (error thresholds, auto-disable, recovery)
//! - Token manager sticky sessions
//! - Rate limiting detection and account rotation
//! - ProxyError with request_id serialization
//! - Model mapping resolution with fallback chain
//! - Proxy handler request/response transformations
//! - SSE streaming response handling
//! - Error handling for rate limits, server errors, and auth errors

mod handler_tests;
mod integration_tests;
