use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

pub const MAX_RETRY_ATTEMPTS: usize = 3;
pub const MAX_OVERLOAD_RETRIES: usize = 30;
pub const OVERLOAD_BASE_DELAY_MS: u64 = 2000;
pub const OVERLOAD_MAX_DELAY_MS: u64 = 60000;
const JITTER_FACTOR: f64 = 0.2;

#[derive(Debug, Clone)]
pub enum RetryStrategy {
    NoRetry,
    FixedDelay(Duration),
    LinearBackoff { base_ms: u64 },
    ExponentialBackoff { base_ms: u64, max_ms: u64 },
}

pub fn apply_jitter(delay_ms: u64) -> u64 {
    use rand::Rng;
    let jitter_range = (delay_ms as f64 * JITTER_FACTOR) as i64;
    let jitter: i64 = rand::thread_rng().gen_range(-jitter_range..=jitter_range);
    #[allow(clippy::cast_possible_wrap)]
    let delay_signed = delay_ms as i64;
    (delay_signed + jitter).max(1) as u64
}

pub fn determine_retry_strategy(
    status_code: u16,
    error_text: &str,
    retried_without_thinking: bool,
) -> RetryStrategy {
    match status_code {
        400 if !retried_without_thinking
            && (error_text.contains("Invalid `signature`")
                || error_text.contains("thinking.signature")
                || error_text.contains("thinking.thinking")) =>
        {
            RetryStrategy::FixedDelay(Duration::from_millis(200))
        }

        429 => {
            if let Some(delay_ms) = crate::proxy::upstream::retry::parse_retry_delay(error_text) {
                let actual_delay = delay_ms.saturating_add(200).min(10_000);
                RetryStrategy::FixedDelay(Duration::from_millis(actual_delay))
            } else {
                RetryStrategy::LinearBackoff { base_ms: 1000 }
            }
        }

        503 | 529 => RetryStrategy::ExponentialBackoff {
            base_ms: 1000,
            max_ms: 8000,
        },

        500 => RetryStrategy::LinearBackoff { base_ms: 500 },

        401 | 403 => RetryStrategy::FixedDelay(Duration::from_millis(100)),

        _ => RetryStrategy::NoRetry,
    }
}

pub async fn execute_retry_strategy(
    strategy: RetryStrategy,
    attempt: usize,
    status_code: u16,
    trace_id: &str,
) -> bool {
    match strategy {
        RetryStrategy::NoRetry => {
            debug!("[{}] Non-retryable error {}, stopping", trace_id, status_code);
            false
        }

        RetryStrategy::FixedDelay(duration) => {
            let base_ms = duration.as_millis() as u64;
            let jittered_ms = apply_jitter(base_ms);
            info!(
                "[{}] ⏱️  Retry with fixed delay: status={}, attempt={}/{}, base={}ms, actual={}ms (jitter applied)",
                trace_id,
                status_code,
                attempt + 1,
                MAX_RETRY_ATTEMPTS,
                base_ms,
                jittered_ms
            );
            sleep(Duration::from_millis(jittered_ms)).await;
            true
        }

        RetryStrategy::LinearBackoff { base_ms } => {
            let calculated_ms = base_ms * (attempt as u64 + 1);
            let jittered_ms = apply_jitter(calculated_ms);
            info!(
                "[{}] ⏱️  Retry with linear backoff: status={}, attempt={}/{}, base={}ms, actual={}ms (jitter applied)",
                trace_id,
                status_code,
                attempt + 1,
                MAX_RETRY_ATTEMPTS,
                calculated_ms,
                jittered_ms
            );
            sleep(Duration::from_millis(jittered_ms)).await;
            true
        }

        RetryStrategy::ExponentialBackoff { base_ms, max_ms } => {
            let calculated_ms = (base_ms * 2_u64.pow(attempt as u32)).min(max_ms);
            let jittered_ms = apply_jitter(calculated_ms);
            info!(
                "[{}] ⏱️  Retry with exponential backoff: status={}, attempt={}/{}, base={}ms, actual={}ms (jitter applied)",
                trace_id,
                status_code,
                attempt + 1,
                MAX_RETRY_ATTEMPTS,
                calculated_ms,
                jittered_ms
            );
            sleep(Duration::from_millis(jittered_ms)).await;
            true
        }
    }
}

pub fn should_rotate_account(status_code: u16) -> bool {
    matches!(status_code, 429 | 401 | 403 | 500)
}
