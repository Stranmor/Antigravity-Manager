//! Comprehensive Integration Tests for the Proxy Module
//!
//! This module contains integration tests that verify the behavior of:
//! 1. Health monitoring (error threshold triggering disable, recovery after cooldown)
//! 2. Token manager sticky sessions
//! 3. Rate limiting detection and account rotation
//! 4. ProxyError with request_id serialization
//! 5. Model mapping resolution (custom -> openai -> anthropic fallback)

#![allow(unused_imports)]

use std::collections::HashMap;
use std::time::Duration;

// =============================================================================
// Health Monitoring Tests
// =============================================================================

mod health_monitoring {
    use super::*;
    use crate::proxy::health::{
        ErrorType, HealthConfig, HealthMonitor, HealthStatus,
    };

    /// Test that error threshold triggers account disable
    #[tokio::test]
    async fn test_error_threshold_triggers_disable() {
        let config = HealthConfig {
            error_threshold: 3,
            cooldown_seconds: 60,
            track_rate_limits: true,
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        // Register test account
        monitor.register_account("test-acc-1".to_string(), "test@example.com".to_string());

        // Initial state: account should be available
        assert!(monitor.is_available("test-acc-1"));

        // Record errors below threshold
        assert!(!monitor.record_error("test-acc-1", 500, "Error 1").await);
        assert!(monitor.is_available("test-acc-1"));

        assert!(!monitor.record_error("test-acc-1", 500, "Error 2").await);
        assert!(monitor.is_available("test-acc-1"));

        // Third error should trigger disable
        assert!(monitor.record_error("test-acc-1", 500, "Error 3").await);
        assert!(!monitor.is_available("test-acc-1"));
    }

    /// Test that success resets consecutive error count
    #[tokio::test]
    async fn test_success_resets_consecutive_errors() {
        let config = HealthConfig {
            error_threshold: 3,
            cooldown_seconds: 60,
            track_rate_limits: false,
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        monitor.register_account("test-acc-2".to_string(), "test2@example.com".to_string());

        // Record 2 errors
        monitor.record_error("test-acc-2", 500, "Error 1").await;
        monitor.record_error("test-acc-2", 500, "Error 2").await;

        // Success resets the counter
        monitor.record_success("test-acc-2");

        // Now we need 3 more errors to disable
        assert!(!monitor.record_error("test-acc-2", 500, "Error 3").await);
        assert!(!monitor.record_error("test-acc-2", 500, "Error 4").await);

        // Account still available (only 2 consecutive errors after reset)
        assert!(monitor.is_available("test-acc-2"));
    }

    /// Test force enable functionality
    #[tokio::test]
    async fn test_force_enable_disabled_account() {
        let config = HealthConfig {
            error_threshold: 1,
            cooldown_seconds: 3600, // Long cooldown
            track_rate_limits: true,
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        monitor.register_account("test-acc-3".to_string(), "test3@example.com".to_string());

        // Disable the account
        assert!(monitor.record_error("test-acc-3", 500, "Fatal error").await);
        assert!(!monitor.is_available("test-acc-3"));

        // Force enable should work
        assert!(monitor.force_enable("test-acc-3").await);
        assert!(monitor.is_available("test-acc-3"));
    }

    /// Test auto-recovery after cooldown period
    #[tokio::test]
    async fn test_auto_recovery_after_cooldown() {
        let config = HealthConfig {
            error_threshold: 1,
            cooldown_seconds: 1, // 1 second cooldown for testing
            track_rate_limits: true,
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        // Start recovery task
        let recovery_handle = monitor.start_recovery_task();

        monitor.register_account("test-acc-4".to_string(), "test4@example.com".to_string());

        // Disable the account
        assert!(monitor.record_error("test-acc-4", 500, "Error").await);
        assert!(!monitor.is_available("test-acc-4"));

        // Wait for cooldown + recovery check interval
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Account should be recovered
        assert!(monitor.is_available("test-acc-4"));

        // Cleanup
        monitor.shutdown();
        let _ = recovery_handle.await;
    }

    /// Test different error types from status codes
    #[test]
    fn test_error_type_mapping() {
        assert_eq!(ErrorType::from_status_code(401), Some(ErrorType::Unauthorized));
        assert_eq!(ErrorType::from_status_code(403), Some(ErrorType::Forbidden));
        assert_eq!(ErrorType::from_status_code(429), Some(ErrorType::RateLimited));
        assert_eq!(ErrorType::from_status_code(500), Some(ErrorType::ServerError));
        assert_eq!(ErrorType::from_status_code(502), Some(ErrorType::ServerError));
        assert_eq!(ErrorType::from_status_code(503), Some(ErrorType::ServerError));
        assert_eq!(ErrorType::from_status_code(504), Some(ErrorType::ServerError));

        // Non-trackable status codes
        assert_eq!(ErrorType::from_status_code(200), None);
        assert_eq!(ErrorType::from_status_code(400), None);
        assert_eq!(ErrorType::from_status_code(404), None);
    }

    /// Test health status display
    #[test]
    fn test_health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Disabled.to_string(), "disabled");
        assert_eq!(HealthStatus::Recovering.to_string(), "recovering");
    }

    /// Test rate limit tracking configuration
    #[tokio::test]
    async fn test_rate_limit_tracking_disabled() {
        let config = HealthConfig {
            error_threshold: 1,
            cooldown_seconds: 60,
            track_rate_limits: false, // Disabled
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        monitor.register_account("test-acc-5".to_string(), "test5@example.com".to_string());

        // 429 errors should NOT trigger disable when tracking is disabled
        let disabled = monitor.record_error("test-acc-5", 429, "Rate limited").await;
        assert!(!disabled);
        assert!(monitor.is_available("test-acc-5"));
    }

    /// Test healthy/disabled counts
    #[tokio::test]
    async fn test_account_counts() {
        let config = HealthConfig {
            error_threshold: 1,
            cooldown_seconds: 60,
            track_rate_limits: true,
            recovery_check_interval_seconds: 1,
        };
        let monitor = HealthMonitor::new(config);

        monitor.register_account("acc-1".to_string(), "acc1@example.com".to_string());
        monitor.register_account("acc-2".to_string(), "acc2@example.com".to_string());
        monitor.register_account("acc-3".to_string(), "acc3@example.com".to_string());

        assert_eq!(monitor.healthy_count(), 3);
        assert_eq!(monitor.disabled_count(), 0);

        // Disable one account
        monitor.record_error("acc-1", 500, "Error").await;

        assert_eq!(monitor.healthy_count(), 2);
        assert_eq!(monitor.disabled_count(), 1);
    }
}

// =============================================================================
// Rate Limiting Tests
// =============================================================================

mod rate_limiting {
    use super::*;
    use crate::proxy::rate_limit::{RateLimitTracker, RateLimitReason};

    /// Test rate limit detection from HTTP 429 status
    #[test]
    fn test_rate_limit_detection_429() {
        let tracker = RateLimitTracker::new();

        // Parse a 429 error with retry-after header
        let info = tracker.parse_from_error("account-1", 429, Some("60"), "Rate limit exceeded");
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.retry_after_sec, 60);
    }

    /// Test rate limit detection from 5xx status (soft avoidance)
    #[test]
    fn test_rate_limit_detection_5xx() {
        let tracker = RateLimitTracker::new();

        // 500 errors should trigger soft avoidance (20s default)
        let info = tracker.parse_from_error("account-2", 500, None, "Internal server error");
        assert!(info.is_some());
        assert_eq!(info.unwrap().retry_after_sec, 20);

        // 503 errors should also trigger soft avoidance
        let info = tracker.parse_from_error("account-3", 503, None, "Service unavailable");
        assert!(info.is_some());
    }

    /// Test rate limit NOT triggered for other status codes
    #[test]
    fn test_rate_limit_not_triggered_for_other_codes() {
        let tracker = RateLimitTracker::new();

        // 400 errors should not trigger rate limiting
        let info = tracker.parse_from_error("account-4", 400, None, "Bad request");
        assert!(info.is_none());

        // 404 errors should not trigger rate limiting
        let info = tracker.parse_from_error("account-5", 404, None, "Not found");
        assert!(info.is_none());
    }

    /// Test safety buffer for very short retry times
    #[test]
    fn test_safety_buffer_minimum_2s() {
        let tracker = RateLimitTracker::new();

        // If API returns 1s, we should enforce minimum 2s
        tracker.parse_from_error("account-6", 429, Some("1"), "");

        let wait = tracker.get_remaining_wait("account-6");
        // Wait should be 1-2 seconds (due to timing)
        assert!((1..=2).contains(&wait), "Expected wait 1-2s, got {wait}");
    }

    /// Test parsing retry time from body (Google API format)
    #[test]
    fn test_parse_google_quota_reset_delay() {
        let tracker = RateLimitTracker::new();

        // Test Google API error format with quotaResetDelay
        let body = r#"{
            "error": {
                "details": [
                    { "metadata": { "quotaResetDelay": "42s" } }
                ]
            }
        }"#;

        let info = tracker.parse_from_error("account-7", 429, None, body);
        assert!(info.is_some());
        assert_eq!(info.unwrap().retry_after_sec, 42);
    }

    /// Test parsing retry time with hours and minutes
    #[test]
    fn test_parse_duration_with_hours_minutes() {
        let tracker = RateLimitTracker::new();

        // Test complex duration format
        let body = r#"{
            "error": {
                "details": [
                    { "metadata": { "quotaResetDelay": "1h30m" } }
                ]
            }
        }"#;

        let info = tracker.parse_from_error("account-8", 429, None, body);
        assert!(info.is_some());
        // 1h30m = 5400 seconds
        assert_eq!(info.unwrap().retry_after_sec, 5400);
    }

    /// Test is_rate_limited check
    #[test]
    fn test_is_rate_limited_check() {
        let tracker = RateLimitTracker::new();

        // Initially not limited
        assert!(!tracker.is_rate_limited("account-9"));

        // Mark as limited
        tracker.parse_from_error("account-9", 429, Some("60"), "");

        // Now should be limited
        assert!(tracker.is_rate_limited("account-9"));
    }

    /// Test per-model rate limiting with quota groups
    #[test]
    fn test_per_model_rate_limiting() {
        let tracker = RateLimitTracker::new();

        // Mark account limited for claude group
        tracker.parse_from_error_with_group("account-10", 429, Some("60"), "", Some("claude"));

        // Claude should be limited
        assert!(tracker.is_rate_limited_for_group("account-10", Some("claude")));

        // Gemini should NOT be limited (independent quota)
        assert!(!tracker.is_rate_limited_for_group("account-10", Some("gemini")));
    }

    /// Test key generation for quota groups
    #[test]
    fn test_make_key_with_quota_group() {
        // Empty group uses plain account_id
        assert_eq!(RateLimitTracker::make_key("acc", None), "acc");
        assert_eq!(RateLimitTracker::make_key("acc", Some("")), "acc");

        // "gemini" group uses plain account_id (backward compat)
        assert_eq!(RateLimitTracker::make_key("acc", Some("gemini")), "acc");

        // Other groups use "account:group" format
        assert_eq!(RateLimitTracker::make_key("acc", Some("claude")), "acc:claude");
        assert_eq!(RateLimitTracker::make_key("acc", Some("agent")), "acc:agent");
    }

    /// Test rate limit reason parsing
    #[test]
    fn test_rate_limit_reason_parsing() {
        let tracker = RateLimitTracker::new();

        // Quota exhausted
        let body = r#"{"error": {"details": [{"reason": "QUOTA_EXHAUSTED"}]}}"#;
        let info = tracker.parse_from_error("acc-reason-1", 429, None, body);
        assert!(info.is_some());
        assert_eq!(info.unwrap().reason, RateLimitReason::QuotaExhausted);

        // Rate limit exceeded
        let body = r#"{"error": {"details": [{"reason": "RATE_LIMIT_EXCEEDED"}]}}"#;
        let info = tracker.parse_from_error("acc-reason-2", 429, None, body);
        assert!(info.is_some());
        assert_eq!(info.unwrap().reason, RateLimitReason::RateLimitExceeded);

        // Server error
        let info = tracker.parse_from_error("acc-reason-3", 500, None, "Internal error");
        assert!(info.is_some());
        assert_eq!(info.unwrap().reason, RateLimitReason::ServerError);
    }

    /// Test cleanup of expired rate limits
    #[test]
    fn test_cleanup_expired_rate_limits() {
        let tracker = RateLimitTracker::new();

        // Mark with 0 retry (expires immediately due to safety buffer of 2s)
        tracker.parse_from_error("expired-acc", 429, Some("0"), "");

        // Wait for expiry
        std::thread::sleep(Duration::from_secs(3));

        // Cleanup should remove expired entries
        let cleaned = tracker.cleanup_expired();
        assert!(cleaned >= 1);
    }
}

// =============================================================================
// ProxyError Tests
// =============================================================================

mod proxy_error {
    use super::*;
    use crate::proxy::error::ProxyError;
    use crate::proxy::middleware::request_id::RequestId;
    use axum::http::StatusCode;

    /// Test error status code mapping
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
            ProxyError::upstream_error(500, "internal").status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            ProxyError::parse_error("json error").status_code(),
            StatusCode::BAD_GATEWAY
        );
    }

    /// Test error code strings
    #[test]
    fn test_error_codes() {
        assert_eq!(ProxyError::invalid_request("").error_code(), "INVALID_REQUEST");
        assert_eq!(ProxyError::token_error("").error_code(), "TOKEN_ERROR");
        assert_eq!(ProxyError::RateLimited(String::new(), None).error_code(), "RATE_LIMITED");
        assert_eq!(ProxyError::Overloaded(String::new(), None).error_code(), "SERVER_OVERLOADED");
        assert_eq!(ProxyError::upstream_error(500, "").error_code(), "UPSTREAM_ERROR");
    }

    /// Test error display messages
    #[test]
    fn test_error_display() {
        let err = ProxyError::invalid_request("missing field");
        assert_eq!(err.to_string(), "Invalid request: missing field");

        let err = ProxyError::upstream_error(500, "internal error");
        assert_eq!(err.to_string(), "Upstream error (500): internal error");

        let err = ProxyError::RateLimited("too many requests".into(), None);
        assert_eq!(err.to_string(), "Rate limited: too many requests");
    }

    /// Test request_id attachment
    #[test]
    fn test_request_id_attachment() {
        let request_id = RequestId::new();
        let id_str = request_id.0.clone();

        let err = ProxyError::invalid_request("test").with_request_id(request_id);

        assert_eq!(err.request_id(), Some(id_str));
    }

    /// Test request_id extraction from all variants
    #[test]
    fn test_request_id_extraction_all_variants() {
        let rid = RequestId::new();
        let rid_str = rid.0.clone();

        // Test each variant
        let errors = vec![
            ProxyError::InvalidRequest(String::new(), Some(rid.clone())),
            ProxyError::TokenError(String::new(), Some(rid.clone())),
            ProxyError::RateLimited(String::new(), Some(rid.clone())),
            ProxyError::Overloaded(String::new(), Some(rid.clone())),
            ProxyError::TransformError(String::new(), Some(rid.clone())),
            ProxyError::ParseError(String::new(), Some(rid.clone())),
            ProxyError::NetworkError(String::new(), Some(rid.clone())),
            ProxyError::InternalError(String::new(), Some(rid.clone())),
            ProxyError::UpstreamError {
                status: 500,
                message: String::new(),
                request_id: Some(rid.clone()),
            },
        ];

        for err in errors {
            assert_eq!(
                err.request_id(),
                Some(rid_str.clone()),
                "Failed for variant: {:?}",
                std::mem::discriminant(&err)
            );
        }
    }

    /// Test None request_id for errors without ID
    #[test]
    fn test_no_request_id() {
        let err = ProxyError::invalid_request("test");
        assert_eq!(err.request_id(), None);

        let err = ProxyError::upstream_error(500, "test");
        assert_eq!(err.request_id(), None);
    }

    /// Test From<serde_json::Error> conversion
    #[test]
    fn test_from_serde_json_error() {
        let json_err: Result<serde_json::Value, _> = serde_json::from_str("invalid json");
        let proxy_err: ProxyError = json_err.unwrap_err().into();

        assert!(proxy_err.to_string().contains("JSON error"));
        assert_eq!(proxy_err.status_code(), StatusCode::BAD_GATEWAY);
    }
}

// =============================================================================
// Model Mapping Tests
// =============================================================================

mod model_mapping {
    use super::*;
    use crate::proxy::common::model_mapping::{
        resolve_model_route, map_claude_model_to_gemini, invalidate_model_cache,
    };

    /// Test custom mapping takes highest priority
    #[test]
    fn test_custom_mapping_priority() {
        invalidate_model_cache();

        let mut custom_mapping = HashMap::new();
        custom_mapping.insert("my-model".to_string(), "my-target".to_string());

        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        let result = resolve_model_route(
            "my-model",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result, "my-target");
    }

    /// Test OpenAI family mapping fallback
    #[test]
    fn test_openai_family_mapping() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let mut openai_mapping = HashMap::new();
        openai_mapping.insert("gpt-4-series".to_string(), "gemini-3-pro-high".to_string());
        let anthropic_mapping = HashMap::new();

        // gpt-4 should match gpt-4-series
        let result = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result, "gemini-3-pro-high");
    }

    /// Test GPT-4o series mapping
    #[test]
    fn test_gpt4o_series_mapping() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let mut openai_mapping = HashMap::new();
        openai_mapping.insert("gpt-4o-series".to_string(), "gemini-2.5-flash".to_string());
        let anthropic_mapping = HashMap::new();

        // gpt-4o-mini should match gpt-4o-series
        let result = resolve_model_route(
            "gpt-4o-mini",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result, "gemini-2.5-flash");
    }

    /// Test Anthropic Claude family mapping
    #[test]
    fn test_claude_family_mapping() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let mut anthropic_mapping = HashMap::new();
        anthropic_mapping.insert("claude-4.5-series".to_string(), "gemini-3-pro-high".to_string());

        // claude-4.5-sonnet should match claude-4.5-series
        let result = resolve_model_route(
            "claude-4-5-sonnet",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result, "gemini-3-pro-high");
    }

    /// Test built-in model mapping fallback
    #[test]
    fn test_builtin_mapping_fallback() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // gpt-4 should fall back to built-in mapping
        let result = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        // Built-in maps gpt-4 to gemini-2.5-pro
        assert_eq!(result, "gemini-2.5-pro");
    }

    /// Test Haiku downgrade strategy (CLI mode)
    #[test]
    fn test_haiku_downgrade_cli_mode() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // Haiku models should downgrade to gemini-3-flash in CLI mode
        let result = resolve_model_route(
            "claude-haiku-4-5-20251001",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true, // CLI mode
        );

        assert_eq!(result, "gemini-3-flash");
    }

    /// Test Haiku NOT downgraded in non-CLI mode
    #[test]
    fn test_haiku_no_downgrade_non_cli_mode() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // Haiku should use built-in mapping in non-CLI mode
        let result = resolve_model_route(
            "claude-3-haiku-20240307",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            false, // Non-CLI mode
        );

        // Built-in maps to claude-sonnet-4-5
        assert_eq!(result, "claude-sonnet-4-5");
    }

    /// Test direct model mapping function
    #[test]
    fn test_map_claude_model_to_gemini() {
        // Known mappings
        assert_eq!(map_claude_model_to_gemini("claude-opus-4"), "claude-opus-4-5-thinking");
        assert_eq!(map_claude_model_to_gemini("gpt-4"), "gemini-2.5-pro");
        assert_eq!(map_claude_model_to_gemini("gpt-4o-mini"), "gemini-2.5-flash");

        // Pass-through for gemini models
        assert_eq!(map_claude_model_to_gemini("gemini-2.5-flash"), "gemini-2.5-flash");
        assert_eq!(map_claude_model_to_gemini("gemini-3-pro-low"), "gemini-3-pro-low");

        // Unknown model fallback
        assert_eq!(map_claude_model_to_gemini("unknown-model-xyz"), "claude-sonnet-4-5");
    }

    /// Test caching behavior
    #[test]
    fn test_model_route_caching() {
        invalidate_model_cache();

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // First call - cache miss
        let result1 = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        // Second call - should hit cache (same result)
        let result2 = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result1, result2);
    }

    /// Test cache invalidation
    #[test]
    fn test_cache_invalidation() {
        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // Populate cache
        let _ = resolve_model_route(
            "test-model",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            false,
        );

        // Invalidate cache
        invalidate_model_cache();

        // Next call recomputes (still correct result)
        let result = resolve_model_route(
            "test-model",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            false,
        );

        assert_eq!(result, "claude-sonnet-4-5"); // Default fallback
    }
}

// =============================================================================
// Sticky Session Config Tests
// =============================================================================

mod sticky_session {
    use crate::proxy::sticky_config::{SchedulingMode, StickySessionConfig};

    /// Test default configuration
    #[test]
    fn test_default_config() {
        let config = StickySessionConfig::default();

        assert_eq!(config.mode, SchedulingMode::Balance);
        assert_eq!(config.max_wait_seconds, 60);
    }

    /// Test scheduling mode defaults
    #[test]
    fn test_scheduling_mode_default() {
        let mode: SchedulingMode = SchedulingMode::default();
        assert_eq!(mode, SchedulingMode::Balance);
    }

    /// Test all scheduling modes exist
    #[test]
    fn test_scheduling_modes() {
        let cache_first = SchedulingMode::CacheFirst;
        let balance = SchedulingMode::Balance;
        let perf_first = SchedulingMode::PerformanceFirst;

        // Verify they're distinct
        assert_ne!(cache_first, balance);
        assert_ne!(balance, perf_first);
        assert_ne!(cache_first, perf_first);
    }

    /// Test serialization roundtrip
    #[test]
    fn test_config_serialization() {
        let config = StickySessionConfig {
            mode: SchedulingMode::CacheFirst,
            max_wait_seconds: 120,
            session_ttl_secs: 1800,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: StickySessionConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.mode, SchedulingMode::CacheFirst);
        assert_eq!(deserialized.max_wait_seconds, 120);
        assert_eq!(deserialized.session_ttl_secs, 1800);
    }
}

// =============================================================================
// Request ID Tests
// =============================================================================

mod request_id_tests {
    use crate::proxy::middleware::request_id::{RequestId, X_REQUEST_ID_HEADER};
    use uuid::Uuid;

    /// Test unique ID generation
    #[test]
    fn test_unique_id_generation() {
        let id1 = RequestId::new();
        let id2 = RequestId::new();

        assert_ne!(id1.0, id2.0);
    }

    /// Test UUID format
    #[test]
    fn test_uuid_format() {
        let id = RequestId::new();

        // Should be 36 chars (UUID with hyphens)
        assert_eq!(id.as_str().len(), 36);

        // Should be valid UUID
        assert!(Uuid::parse_str(id.as_str()).is_ok());
    }

    /// Test display trait
    #[test]
    fn test_display() {
        let id = RequestId::new();
        let display = format!("{id}");

        assert_eq!(display, id.0);
    }

    /// Test clone
    #[test]
    fn test_clone() {
        let id = RequestId::new();
        let cloned = id.clone();

        assert_eq!(id.0, cloned.0);
    }

    /// Test header constant
    #[test]
    fn test_header_constant() {
        assert_eq!(X_REQUEST_ID_HEADER, "X-Request-ID");
    }
}
