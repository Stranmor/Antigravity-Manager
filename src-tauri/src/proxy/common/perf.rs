// Performance Profiling Module
//
// Production-grade timing metrics for critical code paths in the Antigravity proxy.
// Uses zero-allocation where possible and integrates with the existing Prometheus metrics.
//
// Key measurements:
// - Request transformation latency (Claude->Gemini, OpenAI->Gemini)
// - Upstream API call duration
// - Response transformation latency
// - SSE chunk processing time
//
// Design principles:
// - Minimal overhead when not recording (<1us per call)
// - Thread-safe using atomics
// - Histograms with pre-defined buckets for P50/P95/P99 analysis

use metrics::{describe_histogram, histogram};
use std::sync::OnceLock;
use std::time::Instant;

// === Metrics Registration ===
static PERF_METRICS_REGISTERED: OnceLock<()> = OnceLock::new();

/// Initialize performance metrics descriptors.
/// Safe to call multiple times - only registers once.
pub fn init_perf_metrics() {
    PERF_METRICS_REGISTERED.get_or_init(|| {
        // Request transformation metrics
        describe_histogram!(
            "antigravity_transform_request_duration_ms",
            "Time to transform incoming request to Gemini format (milliseconds)"
        );
        describe_histogram!(
            "antigravity_transform_response_duration_ms",
            "Time to transform Gemini response to client format (milliseconds)"
        );

        // Upstream call metrics
        describe_histogram!(
            "antigravity_upstream_call_duration_ms",
            "Total upstream API call duration (milliseconds)"
        );
        describe_histogram!(
            "antigravity_upstream_ttfb_ms",
            "Time to first byte from upstream (milliseconds)"
        );

        // SSE processing metrics
        describe_histogram!(
            "antigravity_sse_chunk_process_us",
            "Time to process a single SSE chunk (microseconds)"
        );
        describe_histogram!(
            "antigravity_sse_json_parse_us",
            "JSON parsing time per SSE chunk (microseconds)"
        );

        // Token manager metrics
        describe_histogram!(
            "antigravity_token_acquire_us",
            "Time to acquire token from pool (microseconds)"
        );

        // Model resolution metrics
        describe_histogram!(
            "antigravity_model_resolve_us",
            "Time to resolve model mapping (microseconds)"
        );
    });
}

// === Timer Types ===

/// A zero-cost timer that records elapsed time to a histogram on drop.
/// Uses RAII pattern for guaranteed measurement even on early returns.
pub struct ScopedTimer {
    start: Instant,
    metric_name: &'static str,
    labels: Option<[(&'static str, String); 2]>,
}

impl ScopedTimer {
    /// Create a new scoped timer that will record to the given metric.
    #[inline]
    pub fn new(metric_name: &'static str) -> Self {
        init_perf_metrics();
        Self {
            start: Instant::now(),
            metric_name,
            labels: None,
        }
    }

    /// Create a timer with provider and model labels.
    #[inline]
    pub fn with_labels(metric_name: &'static str, provider: &str, model: &str) -> Self {
        init_perf_metrics();
        Self {
            start: Instant::now(),
            metric_name,
            labels: Some([
                ("provider", provider.to_string()),
                ("model", model.to_string()),
            ]),
        }
    }

    /// Get elapsed time in milliseconds without stopping.
    #[inline]
    pub fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// Get elapsed time in microseconds without stopping.
    #[inline]
    pub fn elapsed_us(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1_000_000.0
    }

    /// Manually record and reset (for reusable timers).
    #[inline]
    pub fn lap(&mut self) -> f64 {
        let elapsed = self.elapsed_ms();
        self.record_internal(elapsed);
        self.start = Instant::now();
        elapsed
    }

    #[inline]
    fn record_internal(&self, value: f64) {
        if let Some(ref labels) = self.labels {
            histogram!(self.metric_name, labels).record(value);
        } else {
            histogram!(self.metric_name).record(value);
        }
    }
}

impl Drop for ScopedTimer {
    #[inline]
    fn drop(&mut self) {
        let elapsed = self.elapsed_ms();
        self.record_internal(elapsed);
    }
}

/// A lightweight timer for microsecond measurements (SSE chunks).
/// Does NOT record on drop - call `finish()` explicitly for control.
pub struct MicroTimer {
    start: Instant,
}

impl MicroTimer {
    #[inline]
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Finish and record to the specified metric in microseconds.
    #[inline]
    pub fn finish(self, metric_name: &'static str) {
        let elapsed_us = self.start.elapsed().as_secs_f64() * 1_000_000.0;
        histogram!(metric_name).record(elapsed_us);
    }

    /// Finish and record with labels.
    #[inline]
    pub fn finish_with_labels(self, metric_name: &'static str, labels: &[(&'static str, &str)]) {
        let elapsed_us = self.start.elapsed().as_secs_f64() * 1_000_000.0;
        let labels: Vec<(&'static str, String)> = labels.iter().map(|(k, v)| (*k, v.to_string())).collect();
        histogram!(metric_name, &labels).record(elapsed_us);
    }

    /// Get elapsed without recording.
    #[inline]
    pub fn elapsed_us(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1_000_000.0
    }
}

// === Convenience Functions ===

/// Time a request transformation and record to histogram.
#[inline]
pub fn time_request_transform(provider: &str, model: &str) -> ScopedTimer {
    ScopedTimer::with_labels("antigravity_transform_request_duration_ms", provider, model)
}

/// Time a response transformation and record to histogram.
#[inline]
pub fn time_response_transform(provider: &str, model: &str) -> ScopedTimer {
    ScopedTimer::with_labels("antigravity_transform_response_duration_ms", provider, model)
}

/// Time an upstream API call.
#[inline]
pub fn time_upstream_call(provider: &str, model: &str) -> ScopedTimer {
    ScopedTimer::with_labels("antigravity_upstream_call_duration_ms", provider, model)
}

/// Time token acquisition from pool.
#[inline]
pub fn time_token_acquire() -> MicroTimer {
    init_perf_metrics();
    MicroTimer::start()
}

/// Time model resolution.
#[inline]
pub fn time_model_resolve() -> MicroTimer {
    init_perf_metrics();
    MicroTimer::start()
}

/// Time SSE chunk processing.
#[inline]
pub fn time_sse_chunk() -> MicroTimer {
    init_perf_metrics();
    MicroTimer::start()
}

/// Time JSON parsing within SSE processing.
#[inline]
pub fn time_json_parse() -> MicroTimer {
    init_perf_metrics();
    MicroTimer::start()
}

// === Debug Helpers ===

/// Log timing breakdown for debugging (only in debug builds).
#[cfg(debug_assertions)]
pub fn log_timing_breakdown(
    request_id: &str,
    transform_ms: f64,
    upstream_ms: f64,
    response_ms: f64,
) {
    tracing::debug!(
        "[{}] Timing breakdown: transform={:.2}ms, upstream={:.2}ms, response={:.2}ms, total={:.2}ms",
        request_id,
        transform_ms,
        upstream_ms,
        response_ms,
        transform_ms + upstream_ms + response_ms
    );
}

#[cfg(not(debug_assertions))]
#[inline]
pub fn log_timing_breakdown(_: &str, _: f64, _: f64, _: f64) {
    // No-op in release builds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_scoped_timer_basic() {
        let timer = ScopedTimer::new("test_metric");
        sleep(Duration::from_millis(10));
        let elapsed = timer.elapsed_ms();
        assert!((9.0..50.0).contains(&elapsed)); // Allow some variance
    }

    #[test]
    fn test_micro_timer() {
        let timer = MicroTimer::start();
        sleep(Duration::from_micros(100));
        let elapsed = timer.elapsed_us();
        assert!(elapsed >= 50.0); // Microseconds, with some tolerance
    }

    #[test]
    fn test_timer_with_labels() {
        let timer = ScopedTimer::with_labels("test_labeled", "claude", "gemini-2.5-pro");
        sleep(Duration::from_millis(5));
        drop(timer); // Should record without panic
    }

    #[test]
    fn test_lap_functionality() {
        let mut timer = ScopedTimer::new("test_lap");
        sleep(Duration::from_millis(10));
        let lap1 = timer.lap();
        assert!(lap1 >= 9.0);

        sleep(Duration::from_millis(5));
        let lap2 = timer.lap();
        assert!(lap2 >= 4.0 && lap2 < lap1 + 5.0);
    }

    #[test]
    fn test_init_metrics_idempotent() {
        init_perf_metrics();
        init_perf_metrics();
        init_perf_metrics();
        // Should not panic
    }
}
