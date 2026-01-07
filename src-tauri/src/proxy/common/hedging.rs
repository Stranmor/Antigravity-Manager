//! Request Hedging (Speculative Retry) for improved tail latency
//!
//! Hedging is a technique where if the primary request doesn't respond within
//! a deadline, a backup request is fired using a different account. The first
//! response to complete wins, and the other is cancelled.
//!
//! ## Key Design Decisions:
//! - Only applies to non-streaming requests (SSE streaming is not hedged)
//! - Uses a different account for the hedge request to avoid double-hitting the same account
//! - Configurable delay before firing the hedge (default: 2 seconds)
//! - Tracks metrics for hedged requests for observability
//!
//! ## Usage:
//! ```rust,ignore
//! let hedger = RequestHedger::new(config);
//! let result = hedger.execute_with_hedging(
//!     primary_request_fn,
//!     hedge_request_fn,
//! ).await;
//! ```

use crate::proxy::config::HedgingConfig;
use metrics::{counter, describe_counter, describe_gauge, gauge};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

/// Global hedging statistics
static HEDGING_STATS: OnceLock<HedgingStats> = OnceLock::new();

/// Statistics for hedging operations
#[derive(Debug)]
pub struct HedgingStats {
    /// Total hedged requests fired
    pub hedges_fired: AtomicUsize,
    /// Hedges that won (primary was slower)
    pub hedges_won: AtomicUsize,
    /// Primary requests that won
    pub primaries_won: AtomicUsize,
}

impl Default for HedgingStats {
    fn default() -> Self {
        Self {
            hedges_fired: AtomicUsize::new(0),
            hedges_won: AtomicUsize::new(0),
            primaries_won: AtomicUsize::new(0),
        }
    }
}

/// Initialize hedging metrics (call once at startup)
pub fn init_hedging_metrics() {
    let _ = HEDGING_STATS.get_or_init(HedgingStats::default);

    describe_counter!(
        "antigravity_hedged_requests_total",
        "Total number of hedge requests fired"
    );
    describe_counter!(
        "antigravity_hedge_wins_total",
        "Number of times the hedge request won (completed before primary)"
    );
    describe_counter!(
        "antigravity_primary_wins_total",
        "Number of times the primary request won"
    );
    describe_gauge!(
        "antigravity_hedge_win_rate",
        "Ratio of hedge wins to total hedges (0.0-1.0)"
    );
}

/// Get current hedging statistics
pub fn get_hedging_stats() -> (usize, usize, usize) {
    let stats = HEDGING_STATS.get_or_init(HedgingStats::default);
    (
        stats.hedges_fired.load(Ordering::Relaxed),
        stats.hedges_won.load(Ordering::Relaxed),
        stats.primaries_won.load(Ordering::Relaxed),
    )
}

/// Record that a hedge was fired
fn record_hedge_fired() {
    let stats = HEDGING_STATS.get_or_init(HedgingStats::default);
    stats.hedges_fired.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_hedged_requests_total").increment(1);
}

/// Record that a hedge won
fn record_hedge_won() {
    let stats = HEDGING_STATS.get_or_init(HedgingStats::default);
    stats.hedges_won.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_hedge_wins_total").increment(1);
    update_win_rate();
}

/// Record that the primary won
fn record_primary_won() {
    let stats = HEDGING_STATS.get_or_init(HedgingStats::default);
    stats.primaries_won.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_primary_wins_total").increment(1);
    update_win_rate();
}

/// Update the hedge win rate gauge
fn update_win_rate() {
    let (fired, won, _) = get_hedging_stats();
    if fired > 0 {
        let rate = won as f64 / fired as f64;
        gauge!("antigravity_hedge_win_rate").set(rate);
    }
}

/// Result of a hedged request execution
#[derive(Debug)]
pub enum HedgeResult<T> {
    /// Primary request completed first
    PrimaryWon(T),
    /// Hedge request completed first
    HedgeWon(T),
    /// Only primary was executed (no hedging needed or disabled)
    NoHedge(T),
}

impl<T> HedgeResult<T> {
    /// Get the inner value regardless of which request won
    pub fn into_inner(self) -> T {
        match self {
            Self::PrimaryWon(t) | Self::HedgeWon(t) | Self::NoHedge(t) => t,
        }
    }

    /// Check if the hedge won
    pub fn hedge_won(&self) -> bool {
        matches!(self, Self::HedgeWon(_))
    }
}

/// Request hedger that manages speculative retries
#[derive(Clone)]
pub struct RequestHedger {
    config: HedgingConfig,
}

impl RequestHedger {
    /// Create a new hedger with the given configuration
    pub fn new(config: HedgingConfig) -> Self {
        Self { config }
    }

    /// Check if hedging should be applied for this request
    ///
    /// Hedging is skipped for:
    /// - Streaming requests (stream=true)
    /// - When hedging is disabled
    /// - When max_hedged_requests is 0
    pub fn should_hedge(&self, is_streaming: bool) -> bool {
        self.config.enabled && !is_streaming && self.config.max_hedged_requests > 0
    }

    /// Execute a request with hedging
    ///
    /// If the primary request doesn't complete within `hedge_delay_ms`,
    /// a hedge request is fired. The first to complete wins.
    ///
    /// # Arguments
    /// * `primary_fn` - Async function for the primary request
    /// * `hedge_fn` - Async function for the hedge request (uses different account)
    /// * `trace_id` - Request trace ID for logging
    ///
    /// # Returns
    /// `HedgeResult<T>` indicating which request won and containing the result
    pub async fn execute_with_hedging<T, E, PrimaryFut, HedgeFut, PrimaryFn, HedgeFn>(
        &self,
        primary_fn: PrimaryFn,
        hedge_fn: HedgeFn,
        trace_id: &str,
    ) -> Result<HedgeResult<T>, E>
    where
        PrimaryFut: Future<Output = Result<T, E>> + Send,
        HedgeFut: Future<Output = Result<T, E>> + Send,
        PrimaryFn: FnOnce() -> PrimaryFut + Send,
        HedgeFn: FnOnce() -> HedgeFut + Send,
        T: Send,
        E: Send,
    {
        let hedge_delay = Duration::from_millis(self.config.hedge_delay_ms);

        debug!(
            "[{}] Starting hedged request (delay: {}ms)",
            trace_id, self.config.hedge_delay_ms
        );

        // Start the primary request
        let primary_future = primary_fn();
        tokio::pin!(primary_future);

        // Race: primary vs timeout
        tokio::select! {
            biased; // Prefer primary if both are ready

            result = &mut primary_future => {
                // Primary completed before hedge delay
                debug!("[{}] Primary completed before hedge delay", trace_id);
                result.map(HedgeResult::NoHedge)
            }

            () = sleep(hedge_delay) => {
                // Primary didn't complete in time, fire the hedge
                info!(
                    "[{}] Primary request exceeded {}ms deadline, firing hedge request",
                    trace_id, self.config.hedge_delay_ms
                );
                record_hedge_fired();

                // Start the hedge request
                let hedge_future = hedge_fn();
                tokio::pin!(hedge_future);

                // Race: primary vs hedge
                tokio::select! {
                    biased; // Prefer primary if both are ready simultaneously

                    result = &mut primary_future => {
                        // Primary completed first (hedge is auto-cancelled)
                        info!("[{}] Primary request won the race", trace_id);
                        record_primary_won();
                        result.map(HedgeResult::PrimaryWon)
                    }

                    result = &mut hedge_future => {
                        // Hedge completed first (primary is auto-cancelled)
                        info!("[{}] Hedge request won the race", trace_id);
                        record_hedge_won();
                        result.map(HedgeResult::HedgeWon)
                    }
                }
            }
        }
    }

    /// Execute with hedging only if conditions are met
    ///
    /// This is a convenience method that checks `should_hedge` and either
    /// executes with hedging or just runs the primary request.
    pub async fn maybe_hedge<T, E, PrimaryFut, HedgeFut, PrimaryFn, HedgeFn>(
        &self,
        is_streaming: bool,
        primary_fn: PrimaryFn,
        hedge_fn: HedgeFn,
        trace_id: &str,
    ) -> Result<HedgeResult<T>, E>
    where
        PrimaryFut: Future<Output = Result<T, E>> + Send,
        HedgeFut: Future<Output = Result<T, E>> + Send,
        PrimaryFn: FnOnce() -> PrimaryFut + Send,
        HedgeFn: FnOnce() -> HedgeFut + Send,
        T: Send,
        E: Send,
    {
        if self.should_hedge(is_streaming) {
            self.execute_with_hedging(primary_fn, hedge_fn, trace_id)
                .await
        } else {
            // Just run the primary request
            debug!("[{}] Hedging disabled or streaming request, running primary only", trace_id);
            primary_fn().await.map(HedgeResult::NoHedge)
        }
    }
}

impl Default for RequestHedger {
    fn default() -> Self {
        Self::new(HedgingConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_hedging_disabled() {
        let config = HedgingConfig {
            enabled: false,
            hedge_delay_ms: 100,
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        assert!(!hedger.should_hedge(false));
        assert!(!hedger.should_hedge(true));
    }

    #[tokio::test]
    async fn test_hedging_enabled_non_streaming() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 100,
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        assert!(hedger.should_hedge(false));
        assert!(!hedger.should_hedge(true)); // Streaming should not be hedged
    }

    #[tokio::test]
    async fn test_primary_wins_before_delay() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 1000, // 1 second delay
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        let hedge_called = Arc::new(AtomicBool::new(false));
        let hedge_called_clone = Arc::clone(&hedge_called);

        let result: Result<HedgeResult<&str>, ()> = hedger
            .execute_with_hedging(
                || async {
                    // Primary completes immediately
                    Ok("primary")
                },
                || async move {
                    hedge_called_clone.store(true, Ordering::SeqCst);
                    Ok("hedge")
                },
                "test-1",
            )
            .await;

        assert!(matches!(result, Ok(HedgeResult::NoHedge("primary"))));
        assert!(!hedge_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_hedge_fires_after_delay() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 50, // 50ms delay
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        let result: Result<HedgeResult<&str>, ()> = hedger
            .execute_with_hedging(
                || async {
                    // Primary takes 200ms
                    sleep(Duration::from_millis(200)).await;
                    Ok("primary")
                },
                || async {
                    // Hedge returns immediately
                    Ok("hedge")
                },
                "test-2",
            )
            .await;

        assert!(matches!(result, Ok(HedgeResult::HedgeWon("hedge"))));
    }

    #[tokio::test]
    async fn test_primary_wins_race_after_hedge_fired() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 50, // 50ms delay
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        let result: Result<HedgeResult<&str>, ()> = hedger
            .execute_with_hedging(
                || async {
                    // Primary takes 100ms total (50ms after hedge fires)
                    sleep(Duration::from_millis(100)).await;
                    Ok("primary")
                },
                || async {
                    // Hedge takes 200ms
                    sleep(Duration::from_millis(200)).await;
                    Ok("hedge")
                },
                "test-3",
            )
            .await;

        assert!(matches!(result, Ok(HedgeResult::PrimaryWon("primary"))));
    }

    #[tokio::test]
    async fn test_maybe_hedge_skips_streaming() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 50,
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        let result: Result<HedgeResult<&str>, ()> = hedger
            .maybe_hedge(
                true, // streaming = true
                || async { Ok("primary") },
                || async { Ok("hedge") },
                "test-4",
            )
            .await;

        // Should not attempt hedging for streaming
        assert!(matches!(result, Ok(HedgeResult::NoHedge("primary"))));
    }

    #[tokio::test]
    async fn test_error_propagation() {
        let config = HedgingConfig {
            enabled: true,
            hedge_delay_ms: 50,
            max_hedged_requests: 1,
        };
        let hedger = RequestHedger::new(config);

        let result: Result<HedgeResult<&str>, &str> = hedger
            .execute_with_hedging(
                || async {
                    sleep(Duration::from_millis(100)).await;
                    Err("primary error")
                },
                || async { Err("hedge error") },
                "test-5",
            )
            .await;

        // Hedge fires and returns error first
        assert!(matches!(result, Err("hedge error")));
    }

    #[test]
    fn test_hedge_result_into_inner() {
        let primary: HedgeResult<i32> = HedgeResult::PrimaryWon(42);
        assert_eq!(primary.into_inner(), 42);

        let hedge: HedgeResult<i32> = HedgeResult::HedgeWon(99);
        assert!(hedge.hedge_won());
        assert_eq!(hedge.into_inner(), 99);

        let no_hedge: HedgeResult<i32> = HedgeResult::NoHedge(0);
        assert!(!no_hedge.hedge_won());
    }
}
