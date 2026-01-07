// Handler Helper Functions
// Extracted from claude.rs and openai.rs to reduce code duplication

use crate::proxy::common::circuit_breaker::CircuitBreakerManager;
use crate::proxy::common::retry::{
    apply_jitter, MAX_OVERLOAD_RETRIES, OVERLOAD_BASE_DELAY_MS, OVERLOAD_MAX_DELAY_MS,
};
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::server::AppState;
use crate::proxy::token_manager::TokenManager;
use axum::response::Response;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{error, info, info_span, warn};

/// Result of account selection
#[derive(Clone)]
pub struct AccountSelection {
    pub access_token: String,
    pub project_id: String,
    pub email: String,
    pub account_id: String,
}

/// Selects an account for a request with circuit breaker check
///
/// Returns `Ok(AccountSelection)` if an account is available and circuit breaker allows it,
/// `Err(AccountSkipped)` if circuit breaker is open for the selected account,
/// or propagates the token error.
pub async fn select_account_for_request(
    token_manager: &Arc<TokenManager>,
    circuit_breaker: &Arc<CircuitBreakerManager>,
    request_type: &str,
    force_rotate: bool,
    session_id: Option<&str>,
    trace_id: &str,
) -> Result<AccountSelection, SelectAccountError> {
    // Create account selection span for tracing
    let account_selection_span = info_span!(
        "account_selection",
        request_type = %request_type,
        force_rotate = %force_rotate,
        otel.kind = "internal",
    );

    let (access_token, project_id, email, account_id) = {
        let _guard = account_selection_span.enter();
        match token_manager.get_token(request_type, force_rotate, session_id).await {
            Ok(t) => {
                info!(account_id = %t.3, email = %t.2, "Account selected successfully");
                t
            }
            Err(e) => {
                let safe_message = if e.contains("invalid_grant") {
                    "OAuth refresh failed (invalid_grant): refresh_token likely revoked/expired; reauthorize account(s) to restore service.".to_string()
                } else {
                    e
                };
                error!(error = %safe_message, "Account selection failed");
                return Err(SelectAccountError::NoAccounts(safe_message));
            }
        }
    };

    // Check circuit breaker
    if let Err(retry_after) = circuit_breaker.should_allow(&account_id) {
        warn!(
            "[{}] Circuit breaker OPEN for account {} - skipping (retry in {:?})",
            trace_id, email, retry_after
        );
        return Err(SelectAccountError::CircuitBreakerOpen { email, account_id });
    }

    info!("Using account: {} (type: {})", email, request_type);

    Ok(AccountSelection {
        access_token,
        project_id,
        email,
        account_id,
    })
}

/// Error type for account selection
#[derive(Debug)]
pub enum SelectAccountError {
    /// No accounts available or token refresh failed
    NoAccounts(String),
    /// Circuit breaker is open for the selected account
    CircuitBreakerOpen { email: String, account_id: String },
}

impl SelectAccountError {
    /// Convert to a proxy error response
    pub fn into_response(self, request_id: RequestId) -> Response {
        match self {
            SelectAccountError::NoAccounts(msg) => {
                ProxyError::token_error(format!("No available accounts: {msg}"))
                    .with_request_id(request_id)
                    .into_response()
            }
            SelectAccountError::CircuitBreakerOpen { .. } => {
                // This case should be handled by the caller to retry with a different account
                // If we get here, it means all attempts exhausted
                ProxyError::token_error("All accounts have circuit breaker open")
                    .with_request_id(request_id)
                    .into_response()
            }
        }
    }
}

/// Context for handling rate limit responses
pub struct RateLimitContext<'a> {
    pub token_manager: &'a Arc<TokenManager>,
    pub account_id: &'a str,
    pub email: &'a str,
    pub status_code: u16,
    pub retry_after: Option<&'a str>,
    pub error_text: &'a str,
    pub request_type: &'a str,
    pub trace_id: &'a str,
}

/// Handles rate limit responses by marking accounts and recording events
pub fn handle_rate_limit_response(ctx: RateLimitContext<'_>) {
    // Mark rate limited and unbind sessions
    ctx.token_manager.mark_rate_limited_and_unbind(
        ctx.account_id,
        ctx.status_code,
        ctx.retry_after,
        ctx.error_text,
        Some(ctx.request_type),
    );

    // Persist rate limit event to database (fire-and-forget)
    if ctx.status_code == 429 {
        let account_id_owned = ctx.account_id.to_string();
        let quota_group = ctx.request_type.to_string();
        let retry_after_secs = ctx.retry_after.and_then(|r| r.parse::<i32>().ok());
        let reset_at = retry_after_secs.map(|secs| {
            time::OffsetDateTime::now_utc().unix_timestamp() + i64::from(secs)
        });

        std::thread::spawn(move || {
            if let Err(e) = crate::proxy::db::record_rate_limit_event(
                &account_id_owned,
                reset_at,
                Some(&quota_group),
                retry_after_secs,
            ) {
                warn!("Failed to persist rate limit event: {}", e);
            }
        });
    }
}

/// Handles 529 overload errors with exponential backoff
///
/// Returns `Some(delay_ms)` if should retry, `None` if max retries exhausted
pub async fn handle_overload_retry(
    overload_retry_count: &mut usize,
    trace_id: &str,
    email: &str,
    handler_name: &str,
) -> bool {
    *overload_retry_count += 1;

    if *overload_retry_count <= MAX_OVERLOAD_RETRIES {
        // Exponential backoff with jitter: 2s, 4s, 8s, 16s, ... capped at 60s
        let base_delay = OVERLOAD_BASE_DELAY_MS * 2_u64.pow((*overload_retry_count - 1).min(5) as u32);
        let capped_delay = base_delay.min(OVERLOAD_MAX_DELAY_MS);
        let jittered_delay = apply_jitter(capped_delay);

        warn!(
            "[{}] 529 Overloaded ({}) - retry {}/{} in {}ms (account: {}, NOT rotating)",
            trace_id,
            handler_name,
            overload_retry_count,
            MAX_OVERLOAD_RETRIES,
            jittered_delay,
            email
        );

        sleep(Duration::from_millis(jittered_delay)).await;
        true
    } else {
        error!(
            "[{}] 529 Overloaded ({}) - exhausted {} retries, giving up",
            trace_id, handler_name, MAX_OVERLOAD_RETRIES
        );
        false
    }
}

/// Checks if an error indicates server overload (529 or 503 with "overloaded")
pub fn is_overload_error(status_code: u16, error_text: &str) -> bool {
    status_code == 529 || (status_code == 503 && error_text.contains("overloaded"))
}

/// Formats the final error message including overload retry info
pub fn format_final_error(max_attempts: usize, overload_retry_count: usize, last_error: &str) -> String {
    let retry_info = if overload_retry_count > 0 {
        format!(" (including {overload_retry_count} overload retries)")
    } else {
        String::new()
    };

    format!("All {max_attempts} attempts failed{retry_info}. Last error: {last_error}")
}

/// Records success in health monitor and circuit breaker
pub fn record_success(state: &AppState, account_id: &str) {
    state.health_monitor.record_success(account_id);
    state.circuit_breaker.record_success(account_id);
}

/// Records failure in circuit breaker (skip for 529 global overload)
pub fn record_failure(state: &AppState, account_id: &str, status_code: u16, error_text: &str) {
    if status_code != 529 && status_code >= 400 {
        state.circuit_breaker.record_failure(account_id, error_text);
    }
}

use axum::response::IntoResponse;

/// Records error in health monitor for auth failures
pub async fn record_auth_error(state: &AppState, account_id: &str, status_code: u16, error_text: &str) {
    if status_code == 403 || status_code == 401 {
        state.health_monitor.record_error(account_id, status_code, error_text).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_overload_error() {
        assert!(is_overload_error(529, ""));
        assert!(is_overload_error(503, "server is overloaded"));
        assert!(!is_overload_error(503, "service unavailable"));
        assert!(!is_overload_error(500, "internal error"));
        assert!(!is_overload_error(429, "rate limited"));
    }

    #[test]
    fn test_format_final_error() {
        let msg = format_final_error(3, 0, "HTTP 500");
        assert_eq!(msg, "All 3 attempts failed. Last error: HTTP 500");

        let msg = format_final_error(3, 5, "HTTP 529");
        assert_eq!(msg, "All 3 attempts failed (including 5 overload retries). Last error: HTTP 529");
    }
}
