//! Upstream Client Implementation
//!
//! High-performance HTTP client for upstream API calls with:
//! - Multi-endpoint fallback support
//! - Circuit breaker pattern for resilience
//! - Proper error handling without panics

use reqwest::{header, Client, Response, StatusCode};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// Cloud Code v1internal endpoints (fallback order: prod -> daily)
const V1_INTERNAL_BASE_URL_PROD: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const V1_INTERNAL_BASE_URL_DAILY: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal";
const V1_INTERNAL_BASE_URL_FALLBACKS: [&str; 2] = [
    V1_INTERNAL_BASE_URL_PROD,   // Primary production endpoint
    V1_INTERNAL_BASE_URL_DAILY,  // Fallback daily/sandbox endpoint
];

// Circuit breaker configuration
const CIRCUIT_BREAKER_FAILURE_THRESHOLD: usize = 5;      // Open after 5 consecutive failures
const CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS: u64 = 30;   // Try half-open after 30 seconds
const CIRCUIT_BREAKER_SUCCESS_THRESHOLD: usize = 2;      // Close after 2 successes in half-open

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,    // Normal operation
    Open,      // Blocking requests
    HalfOpen,  // Testing if service recovered
}

/// Circuit breaker for an endpoint
#[derive(Debug)]
pub struct CircuitBreaker {
    state: RwLock<CircuitState>,
    failure_count: AtomicUsize,
    success_count: AtomicUsize,
    last_failure_time: AtomicU64,  // Unix timestamp in seconds
    endpoint: String,
}

impl CircuitBreaker {
    pub fn new(endpoint: &str) -> Self {
        Self {
            state: RwLock::new(CircuitState::Closed),
            failure_count: AtomicUsize::new(0),
            success_count: AtomicUsize::new(0),
            last_failure_time: AtomicU64::new(0),
            endpoint: endpoint.to_string(),
        }
    }

    /// Check if requests should be allowed
    pub async fn should_allow(&self) -> bool {
        let state = *self.state.read().await;
        match state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true,  // Allow probe requests
            CircuitState::Open => {
                // Check if recovery timeout has passed
                let last_failure = self.last_failure_time.load(Ordering::Relaxed);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                if now - last_failure >= CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS {
                    // Transition to half-open
                    let mut state_guard = self.state.write().await;
                    if *state_guard == CircuitState::Open {
                        *state_guard = CircuitState::HalfOpen;
                        self.success_count.store(0, Ordering::Relaxed);
                        tracing::info!(
                            "Circuit breaker for {} transitioning to half-open after {}s recovery timeout",
                            self.endpoint,
                            CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS
                        );
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a successful request
    pub async fn record_success(&self) {
        let state = *self.state.read().await;

        match state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::Relaxed);
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::Relaxed) + 1;
                if successes >= CIRCUIT_BREAKER_SUCCESS_THRESHOLD {
                    let mut state_guard = self.state.write().await;
                    *state_guard = CircuitState::Closed;
                    self.failure_count.store(0, Ordering::Relaxed);
                    self.success_count.store(0, Ordering::Relaxed);
                    tracing::info!(
                        "Circuit breaker for {} closed after {} successful requests",
                        self.endpoint,
                        CIRCUIT_BREAKER_SUCCESS_THRESHOLD
                    );
                }
            }
            CircuitState::Open => {
                // Shouldn't happen, but reset if it does
                self.failure_count.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Record a failed request
    pub async fn record_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;

        // Update last failure time
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_failure_time.store(now, Ordering::Relaxed);

        let state = *self.state.read().await;

        match state {
            CircuitState::Closed => {
                if failures >= CIRCUIT_BREAKER_FAILURE_THRESHOLD {
                    let mut state_guard = self.state.write().await;
                    *state_guard = CircuitState::Open;
                    tracing::warn!(
                        "Circuit breaker for {} opened after {} consecutive failures",
                        self.endpoint,
                        failures
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state reopens the circuit
                let mut state_guard = self.state.write().await;
                *state_guard = CircuitState::Open;
                self.success_count.store(0, Ordering::Relaxed);
                tracing::warn!(
                    "Circuit breaker for {} reopened after failure in half-open state",
                    self.endpoint
                );
            }
            CircuitState::Open => {
                // Already open, just update timestamp
            }
        }
    }

    /// Get current state
    pub async fn state(&self) -> CircuitState {
        *self.state.read().await
    }

    /// Get time until circuit breaker might close (in seconds)
    pub fn time_until_retry(&self) -> Option<u64> {
        let last_failure = self.last_failure_time.load(Ordering::Relaxed);
        if last_failure == 0 {
            return None;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let elapsed = now.saturating_sub(last_failure);
        if elapsed >= CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS {
            None
        } else {
            Some(CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS - elapsed)
        }
    }
}

/// Upstream client with circuit breaker support
pub struct UpstreamClient {
    http_client: Client,
    circuit_breakers: Vec<Arc<CircuitBreaker>>,
}

impl UpstreamClient {
    /// Create a new upstream client
    ///
    /// Returns an error if the HTTP client cannot be built (e.g., invalid TLS config)
    pub fn new(proxy_config: Option<crate::proxy::config::UpstreamProxyConfig>) -> Self {
        let mut builder = Client::builder()
            // Connection settings
            .connect_timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(16)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .timeout(Duration::from_secs(600))
            .user_agent("antigravity/1.11.9 windows/amd64");

        if let Some(config) = proxy_config {
            if config.enabled && !config.url.is_empty() {
                if let Ok(proxy) = reqwest::Proxy::all(&config.url) {
                    builder = builder.proxy(proxy);
                    tracing::info!("UpstreamClient enabled proxy: {}", config.url);
                }
            }
        }

        // Build HTTP client - this should never fail with our configuration
        // but we handle it gracefully anyway
        let http_client = builder.build().unwrap_or_else(|e| {
            tracing::error!("Failed to build HTTP client with custom config: {}, using defaults", e);
            Client::new()
        });

        // Initialize circuit breakers for each endpoint
        let circuit_breakers = V1_INTERNAL_BASE_URL_FALLBACKS
            .iter()
            .map(|url| Arc::new(CircuitBreaker::new(url)))
            .collect();

        Self {
            http_client,
            circuit_breakers,
        }
    }

    /// Build v1internal URL
    fn build_url(base_url: &str, method: &str, query_string: Option<&str>) -> String {
        if let Some(qs) = query_string {
            format!("{base_url}:{method}?{qs}")
        } else {
            format!("{base_url}:{method}")
        }
    }

    /// Check if we should try the next endpoint based on status code
    fn should_try_next_endpoint(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::NOT_FOUND
            || status.is_server_error()
    }

    /// Get circuit breaker status for monitoring
    pub async fn get_circuit_breaker_status(&self) -> Vec<(String, CircuitState, Option<u64>)> {
        let mut status = Vec::new();
        for cb in &self.circuit_breakers {
            let state = cb.state().await;
            let retry_in = cb.time_until_retry();
            status.push((cb.endpoint.clone(), state, retry_in));
        }
        status
    }

    /// Call v1internal API with circuit breaker protection
    pub async fn call_v1_internal(
        &self,
        method: &str,
        access_token: &str,
        body: Value,
        query_string: Option<&str>,
    ) -> Result<Response, String> {
        // Build headers (reused across endpoints)
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {access_token}"))
                .map_err(|e| format!("Invalid authorization header: {e}"))?,
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("antigravity/1.11.9 windows/amd64"),
        );

        let mut last_err: Option<String> = None;
        let start_time = Instant::now();

        // Try each endpoint with circuit breaker check
        for (idx, base_url) in V1_INTERNAL_BASE_URL_FALLBACKS.iter().enumerate() {
            let circuit_breaker = &self.circuit_breakers[idx];
            let has_next = idx + 1 < V1_INTERNAL_BASE_URL_FALLBACKS.len();

            // Check circuit breaker
            if !circuit_breaker.should_allow().await {
                let retry_in = circuit_breaker.time_until_retry();
                tracing::debug!(
                    "Circuit breaker open for {}, skipping (retry in {:?}s)",
                    base_url,
                    retry_in
                );
                last_err = Some(format!(
                    "Circuit breaker open for {} (retry in {:?}s)",
                    base_url,
                    retry_in
                ));
                continue;
            }

            let url = Self::build_url(base_url, method, query_string);

            let response = self
                .http_client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let elapsed = start_time.elapsed();

                    if status.is_success() {
                        // Record success in circuit breaker
                        circuit_breaker.record_success().await;

                        if idx > 0 {
                            tracing::info!(
                                "Upstream fallback succeeded | Endpoint: {} | Status: {} | Attempt: {}/{} | Latency: {:?}",
                                base_url,
                                status,
                                idx + 1,
                                V1_INTERNAL_BASE_URL_FALLBACKS.len(),
                                elapsed
                            );
                        } else {
                            tracing::debug!(
                                "Upstream request succeeded | Endpoint: {} | Status: {} | Latency: {:?}",
                                base_url,
                                status,
                                elapsed
                            );
                        }
                        return Ok(resp);
                    }

                    // Record failure for circuit breaker (only for server errors)
                    if status.is_server_error() {
                        circuit_breaker.record_failure().await;
                    }

                    // Try next endpoint if applicable
                    if has_next && Self::should_try_next_endpoint(status) {
                        tracing::warn!(
                            "Upstream endpoint returned {} at {} (method={}), trying next endpoint",
                            status,
                            base_url,
                            method
                        );
                        last_err = Some(format!("Upstream {} returned {}", base_url, status));
                        continue;
                    }

                    // Return non-retryable error or last endpoint response
                    return Ok(resp);
                }
                Err(e) => {
                    // Record network failure in circuit breaker
                    circuit_breaker.record_failure().await;

                    let msg = format!("HTTP request failed at {}: {}", base_url, e);
                    tracing::debug!("{}", msg);
                    last_err = Some(msg);

                    if !has_next {
                        break;
                    }
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| "All endpoints failed".to_string()))
    }

    /// Fetch available models from upstream
    #[allow(dead_code)]
    pub async fn fetch_available_models(&self, access_token: &str) -> Result<Value, String> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {access_token}"))
                .map_err(|e| format!("Invalid authorization header: {e}"))?,
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("antigravity/1.11.9 windows/amd64"),
        );

        let mut last_err: Option<String> = None;

        for (idx, base_url) in V1_INTERNAL_BASE_URL_FALLBACKS.iter().enumerate() {
            let circuit_breaker = &self.circuit_breakers[idx];

            if !circuit_breaker.should_allow().await {
                last_err = Some(format!("Circuit breaker open for {}", base_url));
                continue;
            }

            let url = Self::build_url(base_url, "fetchAvailableModels", None);

            let response = self
                .http_client
                .post(&url)
                .headers(headers.clone())
                .json(&serde_json::json!({}))
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        circuit_breaker.record_success().await;

                        if idx > 0 {
                            tracing::info!(
                                "Upstream fallback succeeded for fetchAvailableModels | Endpoint: {} | Status: {}",
                                base_url,
                                status
                            );
                        } else {
                            tracing::debug!("fetchAvailableModels succeeded | Endpoint: {}", base_url);
                        }

                        let json: Value = resp
                            .json()
                            .await
                            .map_err(|e| format!("Parse json failed: {e}"))?;
                        return Ok(json);
                    }

                    if status.is_server_error() {
                        circuit_breaker.record_failure().await;
                    }

                    let has_next = idx + 1 < V1_INTERNAL_BASE_URL_FALLBACKS.len();
                    if has_next && Self::should_try_next_endpoint(status) {
                        tracing::warn!(
                            "fetchAvailableModels returned {} at {}, trying next endpoint",
                            status,
                            base_url
                        );
                        last_err = Some(format!("Upstream error: {}", status));
                        continue;
                    }

                    return Err(format!("Upstream error: {}", status));
                }
                Err(e) => {
                    circuit_breaker.record_failure().await;

                    let msg = format!("Request failed at {}: {}", base_url, e);
                    tracing::debug!("{}", msg);
                    last_err = Some(msg);

                    if idx + 1 >= V1_INTERNAL_BASE_URL_FALLBACKS.len() {
                        break;
                    }
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| "All endpoints failed".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url() {
        let base_url = "https://cloudcode-pa.googleapis.com/v1internal";

        let url1 = UpstreamClient::build_url(base_url, "generateContent", None);
        assert_eq!(
            url1,
            "https://cloudcode-pa.googleapis.com/v1internal:generateContent"
        );

        let url2 = UpstreamClient::build_url(base_url, "streamGenerateContent", Some("alt=sse"));
        assert_eq!(
            url2,
            "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
    }

    #[tokio::test]
    async fn test_circuit_breaker_initial_state() {
        let cb = CircuitBreaker::new("test-endpoint");
        assert_eq!(cb.state().await, CircuitState::Closed);
        assert!(cb.should_allow().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let cb = CircuitBreaker::new("test-endpoint");

        // Record failures up to threshold
        for _ in 0..CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            cb.record_failure().await;
        }

        assert_eq!(cb.state().await, CircuitState::Open);
        assert!(!cb.should_allow().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_resets_failures() {
        let cb = CircuitBreaker::new("test-endpoint");

        // Record some failures (below threshold)
        for _ in 0..(CIRCUIT_BREAKER_FAILURE_THRESHOLD - 1) {
            cb.record_failure().await;
        }

        // Record success
        cb.record_success().await;

        // Should still be closed
        assert_eq!(cb.state().await, CircuitState::Closed);
        assert_eq!(cb.failure_count.load(Ordering::Relaxed), 0);
    }
}
