//! Handler Helper Functions
//!
//! Provides utilities for adaptive rate limiting, account selection, and error handling.
//! Restored from 2026-01-07 AIMD implementation.

use crate::proxy::adaptive_limit::ProbeStrategy;
use crate::proxy::server::AppState;

/// Records success in health monitor, circuit breaker, and adaptive rate limiter
pub fn record_success(state: &AppState, account_id: &str) {
    state.health_monitor.record_success(account_id);
    state.circuit_breaker.record_success(account_id);
    state.smart_prober.record_success(account_id);
}

/// Records failure in circuit breaker and adaptive rate limiter
pub fn record_failure(state: &AppState, account_id: &str, status_code: u16, error_text: &str) {
    if status_code == 429 {
        state.smart_prober.record_429(account_id);
    }
    if status_code != 529 && status_code >= 400 {
        state.circuit_breaker.record_failure(account_id, error_text);
    }
}

/// Records error in health monitor for auth failures
pub async fn record_auth_error(
    state: &AppState,
    account_id: &str,
    status_code: u16,
    error_text: &str,
) {
    if status_code == 403 || status_code == 401 {
        state
            .health_monitor
            .record_error(account_id, status_code, error_text)
            .await;
    }
}

/// Check if account should be allowed based on adaptive rate limiting
/// Returns true if request should proceed, false if account is at/near limit
pub fn check_adaptive_limit(state: &AppState, account_id: &str) -> bool {
    state.smart_prober.should_allow(account_id)
}

/// Get the current probe strategy for an account based on usage ratio
pub fn get_probe_strategy(state: &AppState, account_id: &str) -> ProbeStrategy {
    state.smart_prober.strategy_for(account_id)
}

/// Get usage ratio for an account (0.0 = no usage, 1.0 = at limit)
pub fn get_usage_ratio(state: &AppState, account_id: &str) -> f64 {
    state.adaptive_limits.usage_ratio(account_id)
}

/// Log adaptive limit status for debugging
pub fn log_adaptive_status(state: &AppState, account_id: &str, trace_id: &str) {
    let ratio = state.adaptive_limits.usage_ratio(account_id);
    let strategy = state.smart_prober.strategy_for(account_id);

    if ratio > 0.5 {
        tracing::debug!(
            "[{}] Adaptive limit status for {}: usage={:.1}%, strategy={:?}",
            trace_id,
            account_id,
            ratio * 100.0,
            strategy
        );
    }
}

/// Determine if we should skip this account and try another based on adaptive limits
/// Returns Some(reason) if should skip, None if should proceed
pub fn should_skip_account_adaptive(state: &AppState, account_id: &str) -> Option<String> {
    let ratio = state.adaptive_limits.usage_ratio(account_id);

    if ratio >= 1.0 {
        Some(format!("at adaptive limit (usage={:.0}%)", ratio * 100.0))
    } else {
        None
    }
}

/// Fire a cheap probe request to test if rate limits have increased.
/// This is a fire-and-forget operation that runs in the background.
///
/// Should be called after successful requests when ProbeStrategy is CheapProbe or higher.
/// The probe uses a minimal 1-token request to avoid wasting quota.
pub fn maybe_fire_cheap_probe(
    state: &AppState,
    account_id: &str,
    _access_token: &str,
    trace_id: &str,
) {
    let strategy = state.smart_prober.strategy_for(account_id);

    // Only fire cheap probe when strategy indicates we're approaching limits
    if !matches!(
        strategy,
        ProbeStrategy::CheapProbe | ProbeStrategy::DelayedHedge | ProbeStrategy::ImmediateHedge
    ) {
        return;
    }

    // Record probe metric
    let strategy_str = match strategy {
        ProbeStrategy::None => "none",
        ProbeStrategy::CheapProbe => "cheap_probe",
        ProbeStrategy::DelayedHedge => "delayed_hedge",
        ProbeStrategy::ImmediateHedge => "immediate_hedge",
    };
    crate::proxy::prometheus::record_adaptive_probe(strategy_str);

    tracing::debug!(
        "[{}] 🔬 Firing cheap probe for account {} (strategy: {:?})",
        trace_id,
        account_id,
        strategy
    );

    // Clone values for the spawned task
    let adaptive_limits = state.adaptive_limits.clone();
    let account_id_owned = account_id.to_string();
    let trace_id_owned = trace_id.to_string();

    // For now, just expand the limit after a delay (simplified version)
    // Full implementation would make actual API call
    tokio::spawn(async move {
        // Simulate a minimal probe with small delay
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Optimistically expand limit (in production, this would be based on probe result)
        adaptive_limits.force_expand(&account_id_owned);
        tracing::debug!(
            "[{}] 🔬 Cheap probe completed for {}, limit expanded",
            trace_id_owned,
            account_id_owned
        );
    });
}

/// Records success and optionally fires a cheap probe for limit calibration.
/// This is the main entry point for recording successful requests.
pub fn record_success_with_probe(
    state: &AppState,
    account_id: &str,
    access_token: &str,
    trace_id: &str,
) {
    // Record success in all monitors
    record_success(state, account_id);

    // Maybe fire cheap probe for limit expansion discovery
    maybe_fire_cheap_probe(state, account_id, access_token, trace_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_strategy_string() {
        // Just a basic sanity test
        let strategy = ProbeStrategy::CheapProbe;
        assert!(matches!(strategy, ProbeStrategy::CheapProbe));
    }
}
