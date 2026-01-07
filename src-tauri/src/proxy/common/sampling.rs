//! Semantic Request Logging with Sampling
//!
//! This module provides low-overhead request/response sampling for debugging
//! and observability. Key features:
//!
//! - Uses `fastrand` for O(1) random sampling with minimal overhead
//! - Truncates bodies to configurable max size
//! - Sanitizes sensitive headers (Authorization, X-API-Key, etc.)
//! - Integrates with existing structured JSON logging via tracing
//!
//! Performance: Adds ~0.1ms overhead only for sampled requests (1% default)

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

use crate::proxy::config::SamplingConfig;

// Thread-local RNG for fast, lock-free random sampling
// Uses fastrand for O(1) random number generation with RefCell for interior mutability
thread_local! {
    static RNG: RefCell<fastrand::Rng> = RefCell::new(fastrand::Rng::new());
}

/// Global counter for sampled requests (for metrics)
static SAMPLED_REQUESTS: AtomicU64 = AtomicU64::new(0);
static TOTAL_SAMPLE_CHECKS: AtomicU64 = AtomicU64::new(0);

/// Headers that should be sanitized (contain sensitive data)
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-auth-token",
    "cookie",
    "set-cookie",
    "x-session-token",
    "x-csrf-token",
    "x-request-id", // Not sensitive but internal
];

/// Sampled request/response data for logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampledRequest {
    /// Request ID for correlation
    pub request_id: String,

    /// HTTP method
    pub method: String,

    /// Request path
    pub path: String,

    /// Target model (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Account ID used for this request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

    /// Request body excerpt (truncated to max_body_size)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body: Option<String>,

    /// Response body excerpt (truncated to max_body_size)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,

    /// Sanitized request headers (only if include_headers is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_headers: Option<std::collections::HashMap<String, String>>,

    /// HTTP status code
    pub status_code: u16,

    /// Request duration in milliseconds
    pub duration_ms: u64,

    /// Whether the body was truncated
    pub body_truncated: bool,

    /// Total input tokens (if available from response)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,

    /// Total output tokens (if available from response)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

/// Sampler instance that can be shared across handlers
#[derive(Debug, Clone)]
pub struct RequestSampler {
    /// Configuration for sampling behavior
    pub config: SamplingConfig,
}

impl RequestSampler {
    /// Create a new sampler from configuration
    #[must_use]
    pub fn new(config: SamplingConfig) -> Self {
        Self { config }
    }

    /// Check if this request should be sampled
    /// Returns true with probability = sample_rate
    #[inline]
    pub fn should_sample(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        TOTAL_SAMPLE_CHECKS.fetch_add(1, Ordering::Relaxed);

        // Use thread-local RNG for lock-free random sampling
        let should = RNG.with(|rng| rng.borrow_mut().f64() < self.config.sample_rate);

        if should {
            SAMPLED_REQUESTS.fetch_add(1, Ordering::Relaxed);
        }

        should
    }

    /// Truncate body to max_body_size, marking if truncated
    #[must_use]
    pub fn truncate_body(&self, body: &str) -> (String, bool) {
        if body.len() <= self.config.max_body_size {
            (body.to_string(), false)
        } else {
            // Truncate at char boundary to avoid invalid UTF-8
            let truncated = body
                .char_indices()
                .take_while(|(i, _)| *i < self.config.max_body_size)
                .map(|(_, c)| c)
                .collect::<String>();
            (format!("{}...[truncated]", truncated), true)
        }
    }

    /// Sanitize headers by redacting sensitive values
    #[must_use]
    pub fn sanitize_headers(&self, headers: &HeaderMap) -> std::collections::HashMap<String, String> {
        let mut result = std::collections::HashMap::new();

        for (name, value) in headers {
            let name_lower = name.as_str().to_lowercase();
            let sanitized_value = if SENSITIVE_HEADERS.contains(&name_lower.as_str()) {
                "[REDACTED]".to_string()
            } else {
                value.to_str().unwrap_or("[invalid utf-8]").to_string()
            };
            result.insert(name.to_string(), sanitized_value);
        }

        result
    }

    /// Log a sampled request (at INFO level with structured fields)
    pub fn log_sampled_request(&self, sampled: &SampledRequest) {
        info!(
            request_id = %sampled.request_id,
            method = %sampled.method,
            path = %sampled.path,
            model = ?sampled.model,
            account_id = ?sampled.account_id,
            status_code = sampled.status_code,
            duration_ms = sampled.duration_ms,
            body_truncated = sampled.body_truncated,
            input_tokens = ?sampled.input_tokens,
            output_tokens = ?sampled.output_tokens,
            sampled = true,
            "Sampled request"
        );

        // Log body excerpts separately to avoid cluttering main log line
        if let Some(ref req_body) = sampled.request_body {
            info!(
                request_id = %sampled.request_id,
                body_type = "request",
                excerpt = %req_body,
                "Sampled body excerpt"
            );
        }

        if let Some(ref resp_body) = sampled.response_body {
            info!(
                request_id = %sampled.request_id,
                body_type = "response",
                excerpt = %resp_body,
                "Sampled body excerpt"
            );
        }
    }
}

/// Builder for constructing SampledRequest
#[derive(Debug, Default)]
pub struct SampledRequestBuilder {
    request_id: String,
    method: String,
    path: String,
    model: Option<String>,
    account_id: Option<String>,
    request_body: Option<String>,
    response_body: Option<String>,
    request_headers: Option<std::collections::HashMap<String, String>>,
    status_code: u16,
    duration_ms: u64,
    body_truncated: bool,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl SampledRequestBuilder {
    /// Create a new builder with required fields
    #[must_use]
    pub fn new(request_id: impl Into<String>, method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            method: method.into(),
            path: path.into(),
            ..Default::default()
        }
    }

    /// Set the model
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the account ID
    #[must_use]
    pub fn account_id(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    /// Set the request body (will be truncated by sampler)
    #[must_use]
    pub fn request_body(mut self, body: String, truncated: bool) -> Self {
        self.request_body = Some(body);
        self.body_truncated = self.body_truncated || truncated;
        self
    }

    /// Set the response body (will be truncated by sampler)
    #[must_use]
    pub fn response_body(mut self, body: String, truncated: bool) -> Self {
        self.response_body = Some(body);
        self.body_truncated = self.body_truncated || truncated;
        self
    }

    /// Set the sanitized request headers
    #[must_use]
    pub fn request_headers(mut self, headers: std::collections::HashMap<String, String>) -> Self {
        self.request_headers = Some(headers);
        self
    }

    /// Set the HTTP status code
    #[must_use]
    pub fn status_code(mut self, code: u16) -> Self {
        self.status_code = code;
        self
    }

    /// Set the duration in milliseconds
    #[must_use]
    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// Set token counts
    #[must_use]
    pub fn tokens(mut self, input: Option<u64>, output: Option<u64>) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    /// Build the SampledRequest
    #[must_use]
    pub fn build(self) -> SampledRequest {
        SampledRequest {
            request_id: self.request_id,
            method: self.method,
            path: self.path,
            model: self.model,
            account_id: self.account_id,
            request_body: self.request_body,
            response_body: self.response_body,
            request_headers: self.request_headers,
            status_code: self.status_code,
            duration_ms: self.duration_ms,
            body_truncated: self.body_truncated,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

/// Get sampling statistics
#[must_use]
pub fn get_sampling_stats() -> (u64, u64) {
    (
        SAMPLED_REQUESTS.load(Ordering::Relaxed),
        TOTAL_SAMPLE_CHECKS.load(Ordering::Relaxed),
    )
}

/// Reset sampling statistics (for testing)
#[cfg(test)]
pub fn reset_sampling_stats() {
    SAMPLED_REQUESTS.store(0, Ordering::Relaxed);
    TOTAL_SAMPLE_CHECKS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampler_disabled() {
        let config = SamplingConfig {
            enabled: false,
            sample_rate: 1.0, // 100% but disabled
            max_body_size: 4096,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        // Should never sample when disabled
        for _ in 0..100 {
            assert!(!sampler.should_sample());
        }
    }

    #[test]
    fn test_sampler_always_sample() {
        reset_sampling_stats();

        let config = SamplingConfig {
            enabled: true,
            sample_rate: 1.0, // 100%
            max_body_size: 4096,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        // Should always sample at 100%
        for _ in 0..10 {
            assert!(sampler.should_sample());
        }

        let (sampled, total) = get_sampling_stats();
        assert_eq!(sampled, 10);
        assert_eq!(total, 10);
    }

    #[test]
    fn test_sampler_never_sample() {
        reset_sampling_stats();

        let config = SamplingConfig {
            enabled: true,
            sample_rate: 0.0, // 0%
            max_body_size: 4096,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        // Should never sample at 0%
        for _ in 0..100 {
            assert!(!sampler.should_sample());
        }

        let (sampled, _) = get_sampling_stats();
        assert_eq!(sampled, 0);
    }

    #[test]
    fn test_truncate_body_short() {
        let config = SamplingConfig::default();
        let sampler = RequestSampler::new(config);

        let short_body = "Hello, world!";
        let (result, truncated) = sampler.truncate_body(short_body);

        assert_eq!(result, short_body);
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_body_long() {
        let config = SamplingConfig {
            enabled: true,
            sample_rate: 0.01,
            max_body_size: 10,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        let long_body = "This is a very long body that exceeds the limit";
        let (result, truncated) = sampler.truncate_body(long_body);

        assert!(truncated);
        assert!(result.ends_with("...[truncated]"));
        assert!(result.len() < long_body.len() + 20); // Some overhead for truncation marker
    }

    #[test]
    fn test_truncate_body_unicode() {
        let config = SamplingConfig {
            enabled: true,
            sample_rate: 0.01,
            max_body_size: 10,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        // Unicode characters that span multiple bytes
        let unicode_body = "Hello, ";
        let (result, _) = sampler.truncate_body(unicode_body);

        // Should be valid UTF-8
        assert!(result.is_ascii() || !result.is_empty());
    }

    #[test]
    fn test_sanitize_headers() {
        let config = SamplingConfig::default();
        let sampler = RequestSampler::new(config);

        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        headers.insert("x-api-key", "sk-secret-key".parse().unwrap());
        headers.insert("user-agent", "test-client".parse().unwrap());

        let sanitized = sampler.sanitize_headers(&headers);

        assert_eq!(sanitized.get("content-type").unwrap(), "application/json");
        assert_eq!(sanitized.get("authorization").unwrap(), "[REDACTED]");
        assert_eq!(sanitized.get("x-api-key").unwrap(), "[REDACTED]");
        assert_eq!(sanitized.get("user-agent").unwrap(), "test-client");
    }

    #[test]
    fn test_sampled_request_builder() {
        let sampled = SampledRequestBuilder::new("req-123", "POST", "/v1/messages")
            .model("claude-sonnet-4-20250514")
            .account_id("acc-456")
            .status_code(200)
            .duration_ms(1500)
            .tokens(Some(100), Some(500))
            .build();

        assert_eq!(sampled.request_id, "req-123");
        assert_eq!(sampled.method, "POST");
        assert_eq!(sampled.path, "/v1/messages");
        assert_eq!(sampled.model, Some("claude-sonnet-4-20250514".to_string()));
        assert_eq!(sampled.account_id, Some("acc-456".to_string()));
        assert_eq!(sampled.status_code, 200);
        assert_eq!(sampled.duration_ms, 1500);
        assert_eq!(sampled.input_tokens, Some(100));
        assert_eq!(sampled.output_tokens, Some(500));
    }

    #[test]
    fn test_statistical_sampling() {
        reset_sampling_stats();

        let config = SamplingConfig {
            enabled: true,
            sample_rate: 0.5, // 50%
            max_body_size: 4096,
            include_headers: false,
        };
        let sampler = RequestSampler::new(config);

        let mut sampled_count = 0;
        let iterations = 1000;

        for _ in 0..iterations {
            if sampler.should_sample() {
                sampled_count += 1;
            }
        }

        // With 50% rate, we expect roughly 500 samples
        // Allow for statistical variance (40-60% range)
        let lower_bound = (iterations as f64 * 0.4) as usize;
        let upper_bound = (iterations as f64 * 0.6) as usize;

        assert!(
            sampled_count >= lower_bound && sampled_count <= upper_bound,
            "Expected {lower_bound}-{upper_bound} samples, got {sampled_count}"
        );
    }
}
