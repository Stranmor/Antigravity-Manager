//! Proxy module integration tests
//!
//! Comprehensive test suite covering:
//! - Health monitoring (error thresholds, auto-disable, recovery)
//! - Token manager sticky sessions
//! - Rate limiting detection and account rotation
//! - ProxyError with request_id serialization
//! - Model mapping resolution with fallback chain

mod integration_tests;
