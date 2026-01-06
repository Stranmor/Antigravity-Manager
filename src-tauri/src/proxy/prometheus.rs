//! Prometheus metrics for Antigravity proxy observability.
//!
//! Exposes metrics compatible with Prometheus/OpenMetrics format:
//! - `antigravity_requests_total{provider,model,status}` - Counter of total requests
//! - `antigravity_request_duration_seconds` - Histogram of request durations
//! - `antigravity_accounts_total` - Gauge of total accounts
//! - `antigravity_accounts_available` - Gauge of available accounts
//! - `antigravity_uptime_seconds` - Gauge of server uptime

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Instant;

/// Global Prometheus handle for rendering metrics
static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Global server start time for uptime calculation
static METRICS_START_TIME: OnceLock<Instant> = OnceLock::new();

/// Initialize Prometheus metrics recorder.
/// Must be called once at application startup before any metrics are recorded.
///
/// Returns the handle that can be used to render metrics as text.
pub fn init_metrics() -> PrometheusHandle {
    let _ = METRICS_START_TIME.get_or_init(Instant::now);

    let handle = PROMETHEUS_HANDLE.get_or_init(|| {
        let builder = PrometheusBuilder::new();
        let handle = builder
            .install_recorder()
            .expect("Failed to install Prometheus metrics recorder");

        // Register metric descriptions
        describe_counter!(
            "antigravity_requests_total",
            "Total number of proxy requests processed"
        );
        describe_histogram!(
            "antigravity_request_duration_seconds",
            "Request duration in seconds"
        );
        describe_gauge!(
            "antigravity_accounts_total",
            "Total number of registered accounts"
        );
        describe_gauge!(
            "antigravity_accounts_available",
            "Number of accounts currently available for use"
        );
        describe_gauge!(
            "antigravity_uptime_seconds",
            "Server uptime in seconds"
        );

        handle
    });

    handle.clone()
}

/// Get the Prometheus handle for rendering metrics.
/// Returns None if metrics have not been initialized.
pub fn get_prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

/// Record a completed request with labels.
///
/// # Arguments
/// * `provider` - The API provider (e.g., "anthropic", "openai", "gemini")
/// * `model` - The model name (e.g., "claude-3-opus", "gpt-4")
/// * `status` - HTTP status code category ("2xx", "4xx", "5xx")
/// * `duration_ms` - Request duration in milliseconds
pub fn record_request(provider: &str, model: &str, status: &str, duration_ms: u64) {
    let labels = [
        ("provider", provider.to_string()),
        ("model", model.to_string()),
        ("status", status.to_string()),
    ];

    counter!("antigravity_requests_total", &labels).increment(1);

    // Convert milliseconds to seconds for histogram
    let duration_seconds = duration_ms as f64 / 1000.0;
    histogram!("antigravity_request_duration_seconds", &labels).record(duration_seconds);
}

/// Update account gauges.
///
/// # Arguments
/// * `total` - Total number of accounts
/// * `available` - Number of available accounts
pub fn update_account_gauges(total: usize, available: usize) {
    gauge!("antigravity_accounts_total").set(total as f64);
    gauge!("antigravity_accounts_available").set(available as f64);
}

/// Update uptime gauge.
/// Should be called periodically or on metrics render.
pub fn update_uptime_gauge() {
    if let Some(start) = METRICS_START_TIME.get() {
        let uptime = start.elapsed().as_secs_f64();
        gauge!("antigravity_uptime_seconds").set(uptime);
    }
}

/// Render all metrics in Prometheus text format.
pub fn render_metrics() -> String {
    update_uptime_gauge();

    if let Some(handle) = get_prometheus_handle() {
        handle.render()
    } else {
        String::from("# Metrics not initialized\n")
    }
}

/// Determine the provider from URL path.
pub fn detect_provider_from_url(url: &str) -> &'static str {
    if url.contains("/v1/messages") || url.contains("/v1/models/claude") {
        "anthropic"
    } else if url.contains("/v1beta/models") {
        "gemini"
    } else if url.contains("/v1/chat/completions")
        || url.contains("/v1/completions")
        || url.contains("/v1/models")
        || url.contains("/v1/images")
    {
        "openai"
    } else if url.contains("/mcp/") {
        "mcp"
    } else {
        "unknown"
    }
}

/// Convert HTTP status code to category for metrics labels.
pub fn status_category(status: u16) -> &'static str {
    match status {
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_category() {
        assert_eq!(status_category(200), "2xx");
        assert_eq!(status_category(404), "4xx");
        assert_eq!(status_category(500), "5xx");
        assert_eq!(status_category(301), "3xx");
    }

    #[test]
    fn test_detect_provider() {
        assert_eq!(detect_provider_from_url("/v1/messages"), "anthropic");
        assert_eq!(
            detect_provider_from_url("/v1/chat/completions"),
            "openai"
        );
        assert_eq!(detect_provider_from_url("/v1beta/models/gemini-pro"), "gemini");
        assert_eq!(detect_provider_from_url("/mcp/web_search"), "mcp");
    }
}
