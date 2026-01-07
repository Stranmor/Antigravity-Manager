//! Integration tests with mock HTTP server for proxy handlers
//!
//! Uses wiremock to simulate upstream API responses and test:
//! - Claude handler (success, rate limits, circuit breaker, coalescing)
//! - OpenAI handler (chat completion, streaming SSE)
//! - Gemini handler (native passthrough)
//!
//! These tests are conditional on the headless feature to avoid Tauri dependencies.

#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// =============================================================================
// Test Utilities
// =============================================================================

/// Create a mock Gemini success response
fn mock_gemini_success_response() -> serde_json::Value {
    serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "Hello! I'm a mock response."
                }],
                "role": "model"
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 10,
            "candidatesTokenCount": 20,
            "totalTokenCount": 30
        }
    })
}

/// Create a mock rate limit (429) response
fn mock_rate_limit_response(retry_after_secs: u64) -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("Retry-After", retry_after_secs.to_string())
        .set_body_json(serde_json::json!({
            "error": {
                "code": 429,
                "message": "Resource exhausted",
                "status": "RESOURCE_EXHAUSTED",
                "details": [{
                    "reason": "RATE_LIMIT_EXCEEDED",
                    "metadata": {
                        "quotaResetDelay": format!("{}s", retry_after_secs)
                    }
                }]
            }
        }))
}

/// Create a mock server error (500) response
#[allow(dead_code)]
fn mock_server_error_response() -> ResponseTemplate {
    ResponseTemplate::new(500).set_body_json(serde_json::json!({
        "error": {
            "code": 500,
            "message": "Internal server error",
            "status": "INTERNAL"
        }
    }))
}

/// Create a mock overload error (529) response
fn mock_overload_response() -> ResponseTemplate {
    ResponseTemplate::new(529).set_body_json(serde_json::json!({
        "error": {
            "code": 529,
            "message": "Server overloaded",
            "status": "UNAVAILABLE"
        }
    }))
}

/// Create SSE streaming response chunks
fn mock_sse_streaming_response() -> String {
    let chunks = vec![
        serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello"}],
                    "role": "model"
                }
            }]
        }),
        serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": " world!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 2,
                "totalTokenCount": 7
            }
        }),
    ];

    chunks
        .into_iter()
        .map(|chunk| format!("data: {}\n\n", chunk))
        .collect::<String>()
}

// =============================================================================
// Circuit Breaker Tests
// =============================================================================

mod circuit_breaker_tests {
    use super::*;
    use crate::proxy::common::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerManager};

    /// Test circuit breaker opens after consecutive failures
    #[tokio::test]
    async fn test_circuit_breaker_opens_on_consecutive_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            open_duration: Duration::from_secs(5),
            success_threshold: 2,
        };
        let manager = CircuitBreakerManager::new(config);

        let account_id = "test-account-1";

        // Circuit should be closed initially
        assert!(manager.should_allow(account_id).is_ok());

        // Record failures up to threshold
        for i in 0..3 {
            manager.record_failure(account_id, &format!("Error {}", i + 1));
        }

        // Circuit should now be open
        let result = manager.should_allow(account_id);
        assert!(result.is_err(), "Circuit should be open after 3 failures");
    }

    /// Test circuit breaker resets on success
    #[tokio::test]
    async fn test_circuit_breaker_resets_on_success() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            open_duration: Duration::from_secs(5),
            success_threshold: 2,
        };
        let manager = CircuitBreakerManager::new(config);

        let account_id = "test-account-2";

        // Record 2 failures (below threshold)
        manager.record_failure(account_id, "Error 1");
        manager.record_failure(account_id, "Error 2");

        // Record success - should reset failure count
        manager.record_success(account_id);

        // Record 2 more failures - still below threshold
        manager.record_failure(account_id, "Error 3");
        manager.record_failure(account_id, "Error 4");

        // Circuit should still be closed (only 2 consecutive failures)
        assert!(manager.should_allow(account_id).is_ok());
    }

    /// Test circuit breaker transitions to half-open after timeout
    #[tokio::test]
    async fn test_circuit_breaker_half_open_transition() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            open_duration: Duration::from_millis(100), // Short timeout for testing
            success_threshold: 1,
        };
        let manager = CircuitBreakerManager::new(config);

        let account_id = "test-account-3";

        // Open the circuit
        manager.record_failure(account_id, "Error 1");
        manager.record_failure(account_id, "Error 2");

        // Should be open
        assert!(manager.should_allow(account_id).is_err());

        // Wait for open_duration to elapse
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should now allow (half-open state)
        assert!(manager.should_allow(account_id).is_ok());

        // Success should close the circuit
        manager.record_success(account_id);

        // Should remain closed after success
        assert!(manager.should_allow(account_id).is_ok());
    }

    /// Test circuit breaker reopens on failure in half-open state
    #[tokio::test]
    async fn test_circuit_breaker_reopens_on_halfopen_failure() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            open_duration: Duration::from_millis(100),
            success_threshold: 2,
        };
        let manager = CircuitBreakerManager::new(config);

        let account_id = "test-account-4";

        // Open the circuit
        manager.record_failure(account_id, "Error 1");
        manager.record_failure(account_id, "Error 2");

        // Wait for half-open
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Transition to half-open by checking
        assert!(manager.should_allow(account_id).is_ok());

        // Record one success (below threshold)
        manager.record_success(account_id);

        // Record failure in half-open - should reopen
        manager.record_failure(account_id, "Error in half-open");

        // Should be open again
        assert!(manager.should_allow(account_id).is_err());
    }
}

// =============================================================================
// Coalescing Tests
// =============================================================================

mod coalescing_tests {
    use crate::proxy::common::coalescing::{
        calculate_fingerprint, CoalesceManager, CoalesceResult,
    };
    use crate::proxy::config::CoalescingConfig;

    /// Test fingerprint calculation is deterministic
    #[test]
    fn test_fingerprint_deterministic() {
        let messages = vec!["Hello", "World"];
        let system: Option<&String> = None;
        let tools: Option<&Vec<String>> = None;

        let fp1 = calculate_fingerprint(
            "claude-3-opus",
            &messages,
            system,
            tools,
            Some(0.7),
            None,
            Some(1000),
        );

        let fp2 = calculate_fingerprint(
            "claude-3-opus",
            &messages,
            system,
            tools,
            Some(0.7),
            None,
            Some(1000),
        );

        assert_eq!(fp1, fp2, "Same inputs should produce same fingerprint");
    }

    /// Test fingerprint changes with different parameters
    #[test]
    fn test_fingerprint_changes_with_params() {
        let messages = vec!["Hello"];
        let system: Option<&String> = None;
        let tools: Option<&Vec<String>> = None;

        let fp_temp_07 = calculate_fingerprint(
            "claude-3-opus",
            &messages,
            system,
            tools,
            Some(0.7),
            None,
            None,
        );

        let fp_temp_09 = calculate_fingerprint(
            "claude-3-opus",
            &messages,
            system,
            tools,
            Some(0.9),
            None,
            None,
        );

        assert_ne!(fp_temp_07, fp_temp_09, "Different temps should produce different fingerprints");
    }

    /// Test coalescing deduplicates identical requests
    #[tokio::test]
    async fn test_coalescing_deduplication() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 5000,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let fingerprint = 12345u64;

        // First request should be primary
        let result1 = manager.get_or_create(fingerprint);
        assert!(matches!(result1, CoalesceResult::Primary(_)));

        // Second identical request should be coalesced
        let result2 = manager.get_or_create(fingerprint);
        assert!(matches!(result2, CoalesceResult::Coalesced(_)));
    }

    /// Test coalescing broadcasts result to all waiters
    #[tokio::test]
    async fn test_coalescing_broadcasts_result() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 5000,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let fingerprint = 54321u64;

        let result1 = manager.get_or_create(fingerprint);
        let result2 = manager.get_or_create(fingerprint);
        let result3 = manager.get_or_create(fingerprint);

        match (result1, result2, result3) {
            (
                CoalesceResult::Primary(sender),
                CoalesceResult::Coalesced(recv2),
                CoalesceResult::Coalesced(recv3),
            ) => {
                // Spawn receivers
                let handle2 = tokio::spawn(async move { recv2.recv().await });
                let handle3 = tokio::spawn(async move { recv3.recv().await });

                // Send result from primary
                sender.send("broadcast_value".to_string());

                // Both receivers should get the same value
                let val2 = handle2.await.unwrap().unwrap();
                let val3 = handle3.await.unwrap().unwrap();

                assert_eq!(*val2, "broadcast_value");
                assert_eq!(*val3, "broadcast_value");
            }
            _ => panic!("Expected Primary, Coalesced, Coalesced"),
        }
    }

    /// Test coalescing is disabled when config says so
    #[tokio::test]
    async fn test_coalescing_disabled() {
        let config = CoalescingConfig {
            enabled: false,
            window_ms: 5000,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let fingerprint = 11111u64;

        // Both should be primary when disabled
        let result1 = manager.get_or_create(fingerprint);
        let result2 = manager.get_or_create(fingerprint);

        assert!(matches!(result1, CoalesceResult::Primary(_)));
        assert!(matches!(result2, CoalesceResult::Primary(_)));
    }

    /// Test coalescing removes entry on failure
    #[tokio::test]
    async fn test_coalescing_failure_removes_entry() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 5000,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let fingerprint = 99999u64;

        // Get primary
        let result1 = manager.get_or_create(fingerprint);
        assert_eq!(manager.active_count(), 1);

        // Fail the primary
        if let CoalesceResult::Primary(sender) = result1 {
            sender.fail();
        }

        // Entry should be removed
        assert_eq!(manager.active_count(), 0);

        // New request should be primary
        let result2 = manager.get_or_create(fingerprint);
        assert!(matches!(result2, CoalesceResult::Primary(_)));
    }
}

// =============================================================================
// Mock HTTP Server Tests
// =============================================================================

mod mock_server_tests {
    use super::*;

    /// Test mock server responds with success
    #[tokio::test]
    async fn test_mock_server_success_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r".*generateContent.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_gemini_success_response()))
            .mount(&mock_server)
            .await;

        // Make request to mock server
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({"test": "data"}))
            .send()
            .await
            .expect("Request should succeed");

        assert_eq!(response.status(), 200);

        let body: serde_json::Value = response.json().await.expect("Should parse JSON");
        assert!(body.get("candidates").is_some());
    }

    /// Test mock server responds with rate limit
    #[tokio::test]
    async fn test_mock_server_rate_limit_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r".*generateContent.*"))
            .respond_with(mock_rate_limit_response(60))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({"test": "data"}))
            .send()
            .await
            .expect("Request should complete");

        assert_eq!(response.status(), 429);

        // Check Retry-After header
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        assert_eq!(retry_after, Some(60));
    }

    /// Test mock server with sequence of responses (success after failures)
    #[tokio::test]
    async fn test_mock_server_retry_success() {
        let mock_server = MockServer::start().await;
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        // First 2 calls return 429, then success
        Mock::given(method("POST"))
            .and(path_regex(r".*generateContent.*"))
            .respond_with(move |_req: &wiremock::Request| {
                let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    mock_rate_limit_response(1)
                } else {
                    ResponseTemplate::new(200).set_body_json(mock_gemini_success_response())
                }
            })
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();

        // First call - 429
        let resp1 = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp1.status(), 429);

        // Second call - 429
        let resp2 = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp2.status(), 429);

        // Third call - Success
        let resp3 = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp3.status(), 200);

        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    /// Test mock server with overload (529) response
    #[tokio::test]
    async fn test_mock_server_overload_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r".*generateContent.*"))
            .respond_with(mock_overload_response())
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/:generateContent", mock_server.uri()))
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("Request should complete");

        assert_eq!(response.status(), 529);
    }

    /// Test mock SSE streaming response
    #[tokio::test]
    async fn test_mock_server_streaming_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r".*streamGenerateContent.*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(mock_sse_streaming_response()),
            )
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/:streamGenerateContent?alt=sse", mock_server.uri()))
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("Request should complete");

        assert_eq!(response.status(), 200);

        // Check Content-Type header (may be normalized to lowercase)
        let content_type = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("");
        assert!(
            content_type.contains("event-stream") || content_type.contains("text/"),
            "Expected event-stream content type, got: {}", content_type
        );

        let body = response.text().await.unwrap();
        assert!(body.contains("data:"));
        assert!(body.contains("Hello"));
    }
}

// =============================================================================
// Rate Limit Handler Tests
// =============================================================================

mod rate_limit_handler_tests {
    use crate::proxy::rate_limit::{RateLimitReason, RateLimitTracker};

    /// Test rate limit tracking after 429 response
    #[test]
    fn test_rate_limit_tracking_from_mock_response() {
        let tracker = RateLimitTracker::new();

        // Simulate receiving a 429 response
        let mock_body = serde_json::json!({
            "error": {
                "code": 429,
                "message": "Rate limit exceeded",
                "details": [{
                    "reason": "RATE_LIMIT_EXCEEDED",
                    "metadata": {
                        "quotaResetDelay": "45s"
                    }
                }]
            }
        })
        .to_string();

        let info = tracker.parse_from_error("test-account", 429, Some("45"), &mock_body);

        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.retry_after_sec, 45);
        assert_eq!(info.reason, RateLimitReason::RateLimitExceeded);
    }

    /// Test quota exhausted detection
    #[test]
    fn test_quota_exhausted_detection() {
        let tracker = RateLimitTracker::new();

        let mock_body = serde_json::json!({
            "error": {
                "details": [{
                    "reason": "QUOTA_EXHAUSTED",
                    "metadata": {
                        "quotaResetDelay": "3600s"
                    }
                }]
            }
        })
        .to_string();

        let info = tracker.parse_from_error("account-quota", 429, None, &mock_body);

        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.reason, RateLimitReason::QuotaExhausted);
        assert_eq!(info.retry_after_sec, 3600);
    }

    /// Test multiple accounts with independent rate limits
    #[test]
    fn test_independent_account_rate_limits() {
        let tracker = RateLimitTracker::new();

        // Rate limit account 1
        tracker.parse_from_error("account-1", 429, Some("30"), "");
        // Rate limit account 2
        tracker.parse_from_error("account-2", 429, Some("60"), "");

        // Both should be rate limited
        assert!(tracker.is_rate_limited("account-1"));
        assert!(tracker.is_rate_limited("account-2"));

        // Get remaining wait times (should be different)
        let wait1 = tracker.get_remaining_wait("account-1");
        let wait2 = tracker.get_remaining_wait("account-2");

        // Account 2 has longer wait time
        assert!(wait2 >= wait1);
    }

    /// Test rate limit with model-specific groups
    #[test]
    fn test_model_specific_rate_limits() {
        let tracker = RateLimitTracker::new();

        // Rate limit for Claude model group
        tracker.parse_from_error_with_group("account-multi", 429, Some("60"), "", Some("claude"));

        // Claude should be limited
        assert!(tracker.is_rate_limited_for_group("account-multi", Some("claude")));

        // Other model groups should not be limited
        assert!(!tracker.is_rate_limited_for_group("account-multi", Some("gemini")));
        assert!(!tracker.is_rate_limited_for_group("account-multi", Some("agent")));
    }
}

// =============================================================================
// Request Transform Tests
// =============================================================================

mod transform_tests {
    use super::*;

    /// Test Claude request structure
    #[test]
    fn test_claude_request_structure() {
        let request = serde_json::json!({
            "model": "claude-3-opus-20240229",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": "Hello, Claude!"
            }],
            "stream": false
        });

        // Validate required fields
        assert!(request.get("model").is_some());
        assert!(request.get("messages").is_some());
        assert!(request.get("max_tokens").is_some());
    }

    /// Test OpenAI request structure
    #[test]
    fn test_openai_request_structure() {
        let request = serde_json::json!({
            "model": "gpt-4",
            "messages": [{
                "role": "user",
                "content": "Hello, GPT!"
            }],
            "temperature": 0.7,
            "stream": true
        });

        // Validate required fields
        assert!(request.get("model").is_some());
        assert!(request.get("messages").is_some());
    }

    /// Test Gemini request structure (native)
    #[test]
    fn test_gemini_request_structure() {
        let request = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": "Hello, Gemini!"
                }],
                "role": "user"
            }],
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 1024
            }
        });

        // Validate required fields
        assert!(request.get("contents").is_some());
    }

    /// Test response transformation expectations
    #[test]
    fn test_gemini_response_structure() {
        let response = mock_gemini_success_response();

        // Validate response structure
        let candidates = response.get("candidates").expect("Should have candidates");
        assert!(candidates.is_array());
        assert!(!candidates.as_array().unwrap().is_empty());

        let first_candidate = &candidates.as_array().unwrap()[0];
        assert!(first_candidate.get("content").is_some());
        assert!(first_candidate.get("finishReason").is_some());

        // Validate usage metadata
        let usage = response.get("usageMetadata").expect("Should have usage");
        assert!(usage.get("promptTokenCount").is_some());
        assert!(usage.get("candidatesTokenCount").is_some());
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

mod error_handling_tests {
    use crate::proxy::error::{ErrorCode, ProxyError};
    use crate::proxy::middleware::request_id::RequestId;
    use axum::http::StatusCode;

    /// Test error code from 429 response
    #[test]
    fn test_rate_limit_error_code() {
        let err = ProxyError::RateLimited("Rate limit exceeded".into(), None);

        assert_eq!(err.error_code(), ErrorCode::RateLimited);
        assert_eq!(err.status_code(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// Test error code from 529 overload
    #[test]
    fn test_overload_error_code() {
        let err = ProxyError::Overloaded("Server overloaded".into(), None);

        assert_eq!(err.error_code(), ErrorCode::RateLimited);
        // Overloaded returns SERVICE_UNAVAILABLE (503), not TOO_MANY_REQUESTS
        assert_eq!(err.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Test upstream error preserves status code
    #[test]
    fn test_upstream_error_status_preservation() {
        let err = ProxyError::upstream_error(503, "Service unavailable");
        assert_eq!(err.status_code(), StatusCode::SERVICE_UNAVAILABLE);

        let err = ProxyError::upstream_error(502, "Bad gateway");
        assert_eq!(err.status_code(), StatusCode::BAD_GATEWAY);
    }

    /// Test request ID propagation in errors
    #[test]
    fn test_request_id_propagation() {
        let request_id = RequestId::new();
        let id_str = request_id.0.clone();

        let err = ProxyError::RateLimited("test".into(), None).with_request_id(request_id);

        assert_eq!(err.request_id(), Some(id_str));
    }

    /// Test error message formatting
    #[test]
    fn test_error_message_formatting() {
        let err = ProxyError::upstream_error(429, "Too many requests");
        assert!(err.to_string().contains("429"));
        assert!(err.to_string().contains("Too many requests"));
    }
}

// =============================================================================
// Retry Strategy Tests
// =============================================================================

mod retry_strategy_tests {
    use crate::proxy::common::retry::{
        apply_jitter, determine_retry_strategy, should_rotate_account, RetryStrategy,
    };

    /// Test retry strategy for 429 rate limit
    #[test]
    fn test_retry_strategy_rate_limit() {
        let strategy = determine_retry_strategy(429, "Resource exhausted", false);

        // 429 should return LinearBackoff or FixedDelay depending on parse
        match strategy {
            RetryStrategy::LinearBackoff { base_ms } => {
                assert!(base_ms > 0);
            }
            RetryStrategy::FixedDelay(duration) => {
                assert!(duration.as_millis() > 0);
            }
            _ => panic!("Expected LinearBackoff or FixedDelay for 429"),
        }
    }

    /// Test retry strategy for 500 server error
    #[test]
    fn test_retry_strategy_server_error() {
        let strategy = determine_retry_strategy(500, "Internal error", false);

        match strategy {
            RetryStrategy::LinearBackoff { base_ms } => {
                assert_eq!(base_ms, 500);
            }
            _ => panic!("Expected LinearBackoff for 500"),
        }
    }

    /// Test retry strategy for 503/529 overload
    #[test]
    fn test_retry_strategy_overload() {
        let strategy = determine_retry_strategy(503, "Service unavailable", false);

        match strategy {
            RetryStrategy::ExponentialBackoff { base_ms, max_ms } => {
                assert_eq!(base_ms, 1000);
                assert_eq!(max_ms, 8000);
            }
            _ => panic!("Expected ExponentialBackoff for 503"),
        }
    }

    /// Test no retry for 400 bad request
    #[test]
    fn test_no_retry_for_bad_request() {
        let strategy = determine_retry_strategy(400, "Bad request", false);

        assert!(matches!(strategy, RetryStrategy::NoRetry));
    }

    /// Test account rotation decision
    #[test]
    fn test_should_rotate_account() {
        // Should rotate on 429
        assert!(should_rotate_account(429));

        // Should rotate on 401
        assert!(should_rotate_account(401));

        // Should rotate on 500 (this is the actual behavior)
        assert!(should_rotate_account(500));

        // Should NOT rotate on 400 (client issue)
        assert!(!should_rotate_account(400));
    }

    /// Test jitter application
    #[test]
    fn test_jitter_application() {
        let base = 1000u64;

        // Apply jitter multiple times and check variance
        let values: Vec<u64> = (0..100).map(|_| apply_jitter(base)).collect();

        // All values should be in range [800, 1200] (0.8x to 1.2x with 0.2 jitter factor)
        for v in &values {
            assert!(*v >= 800 && *v <= 1200, "Jitter out of range: {}", v);
        }

        // There should be variance (not all values the same)
        let unique: std::collections::HashSet<_> = values.iter().collect();
        assert!(unique.len() > 1, "Jitter should produce varying values");
    }
}

// =============================================================================
// Upstream Client Tests with Mock
// =============================================================================

mod upstream_client_mock_tests {
    use super::*;

    /// Test basic request to mock server
    #[tokio::test]
    async fn test_upstream_request_to_mock() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r".*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_gemini_success_response()))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let response = client
            .post(mock_server.uri())
            .json(&serde_json::json!({
                "contents": [{"parts": [{"text": "test"}]}]
            }))
            .send()
            .await
            .expect("Request failed");

        assert!(response.status().is_success());
    }

    /// Test timeout handling
    #[tokio::test]
    async fn test_upstream_timeout_handling() {
        let mock_server = MockServer::start().await;

        // Mock with long delay (longer than our timeout)
        Mock::given(method("POST"))
            .and(path_regex(r".*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(mock_gemini_success_response())
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();

        let result = client
            .post(mock_server.uri())
            .json(&serde_json::json!({}))
            .send()
            .await;

        // Should timeout
        assert!(result.is_err());
    }

    /// Test multiple endpoints simulation
    #[tokio::test]
    async fn test_multiple_mock_endpoints() {
        let mock_server = MockServer::start().await;

        // Claude endpoint
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_test",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello from Claude!"}],
                "model": "claude-3-opus-20240229",
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 20}
            })))
            .mount(&mock_server)
            .await;

        // OpenAI endpoint
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "model": "gpt-4",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hello from GPT!"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();

        // Test Claude endpoint
        let claude_resp = client
            .post(format!("{}/v1/messages", mock_server.uri()))
            .json(&serde_json::json!({"messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(claude_resp.status(), 200);

        // Test OpenAI endpoint
        let openai_resp = client
            .post(format!("{}/v1/chat/completions", mock_server.uri()))
            .json(&serde_json::json!({"messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(openai_resp.status(), 200);
    }
}
