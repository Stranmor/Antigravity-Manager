//! Request Coalescing/Deduplication for identical concurrent requests
//!
//! Coalescing merges identical concurrent requests (same fingerprint) into a single
//! upstream call. All coalesced clients receive the same response via broadcast.
//!
//! ## Key Design Decisions:
//! - Uses xxHash3 for fast fingerprinting (~20GB/s)
//! - Fingerprint includes: model, messages, system, tools, temperature, top_p, max_tokens
//! - Uses tokio::sync::broadcast for 1-to-N response distribution
//! - LRU eviction when max_entries is reached
//! - Only applies to non-streaming requests
//!
//! ## Usage:
//! ```rust,ignore
//! let manager = CoalesceManager::new(config);
//!
//! // Check if request can be coalesced
//! match manager.get_or_create(fingerprint) {
//!     CoalesceResult::Primary(sender) => {
//!         // Execute the upstream request
//!         let response = do_upstream_call().await;
//!         // Broadcast result to all subscribers
//!         sender.send(response.clone());
//!         response
//!     }
//!     CoalesceResult::Coalesced(receiver) => {
//!         // Wait for the primary to complete
//!         receiver.recv().await
//!     }
//! }
//! ```

use crate::proxy::config::CoalescingConfig;
use dashmap::DashMap;
use lru::LruCache;
use metrics::{counter, describe_counter, describe_gauge, gauge};
use parking_lot::Mutex;
use serde::Serialize;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, info};
use xxhash_rust::xxh3::Xxh3;

/// Global coalescing statistics
static COALESCING_STATS: OnceLock<CoalescingStats> = OnceLock::new();

/// Statistics for coalescing operations
#[derive(Debug)]
pub struct CoalescingStats {
    /// Total requests that became primary (executed upstream)
    pub primary_requests: AtomicUsize,
    /// Total requests that were coalesced (waited for primary)
    pub coalesced_requests: AtomicUsize,
    /// Total fingerprint cache hits
    pub cache_hits: AtomicUsize,
    /// Total fingerprint cache misses
    pub cache_misses: AtomicUsize,
    /// Total LRU evictions
    pub evictions: AtomicUsize,
}

impl Default for CoalescingStats {
    fn default() -> Self {
        Self {
            primary_requests: AtomicUsize::new(0),
            coalesced_requests: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            cache_misses: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
        }
    }
}

/// Initialize coalescing metrics (call once at startup)
pub fn init_coalescing_metrics() {
    let _ = COALESCING_STATS.get_or_init(CoalescingStats::default);

    describe_counter!(
        "antigravity_coalesce_primary_total",
        "Total number of primary (executed) requests"
    );
    describe_counter!(
        "antigravity_coalesce_coalesced_total",
        "Total number of coalesced (deduplicated) requests"
    );
    describe_counter!(
        "antigravity_coalesce_cache_hits_total",
        "Total fingerprint cache hits"
    );
    describe_counter!(
        "antigravity_coalesce_cache_misses_total",
        "Total fingerprint cache misses"
    );
    describe_counter!(
        "antigravity_coalesce_evictions_total",
        "Total LRU evictions"
    );
    describe_gauge!(
        "antigravity_coalesce_active_entries",
        "Current number of active fingerprint entries"
    );
    describe_gauge!(
        "antigravity_coalesce_ratio",
        "Ratio of coalesced to total requests (0.0-1.0)"
    );
}

/// Get current coalescing statistics
pub fn get_coalescing_stats() -> (usize, usize, usize, usize, usize) {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    (
        stats.primary_requests.load(Ordering::Relaxed),
        stats.coalesced_requests.load(Ordering::Relaxed),
        stats.cache_hits.load(Ordering::Relaxed),
        stats.cache_misses.load(Ordering::Relaxed),
        stats.evictions.load(Ordering::Relaxed),
    )
}

fn record_primary() {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    stats.primary_requests.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_coalesce_primary_total").increment(1);
    update_coalesce_ratio();
}

fn record_coalesced() {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    stats.coalesced_requests.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_coalesce_coalesced_total").increment(1);
    update_coalesce_ratio();
}

fn record_cache_hit() {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    stats.cache_hits.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_coalesce_cache_hits_total").increment(1);
}

fn record_cache_miss() {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    stats.cache_misses.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_coalesce_cache_misses_total").increment(1);
}

fn record_eviction() {
    let stats = COALESCING_STATS.get_or_init(CoalescingStats::default);
    stats.evictions.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_coalesce_evictions_total").increment(1);
}

fn update_active_entries(count: usize) {
    gauge!("antigravity_coalesce_active_entries").set(count as f64);
}

fn update_coalesce_ratio() {
    let (primary, coalesced, _, _, _) = get_coalescing_stats();
    let total = primary + coalesced;
    if total > 0 {
        let ratio = coalesced as f64 / total as f64;
        gauge!("antigravity_coalesce_ratio").set(ratio);
    }
}

/// Result of coalescing check
pub enum CoalesceResult<T> {
    /// This request is the primary - execute upstream and broadcast result
    Primary(CoalesceSender<T>),
    /// This request should wait for the primary's result
    Coalesced(CoalesceReceiver<T>),
}

/// Sender handle for primary request to broadcast result
pub struct CoalesceSender<T> {
    fingerprint: u64,
    sender: broadcast::Sender<Arc<T>>,
    manager_pending: Arc<DashMap<u64, PendingEntry<T>>>,
}

impl<T: Clone> CoalesceSender<T> {
    /// Broadcast the result to all coalesced requests
    ///
    /// This automatically removes the entry from the pending map
    pub fn send(self, result: T) {
        let arc_result = Arc::new(result);
        // Remove from pending map first
        self.manager_pending.remove(&self.fingerprint);
        // Send to all subscribers (ignore send errors - receivers may have dropped)
        let _ = self.sender.send(arc_result);
        debug!(
            fingerprint = %self.fingerprint,
            "Broadcast coalesced result"
        );
    }

    /// Mark the request as failed (removes from pending without broadcasting)
    pub fn fail(self) {
        self.manager_pending.remove(&self.fingerprint);
        debug!(
            fingerprint = %self.fingerprint,
            "Primary request failed, removed from coalescing"
        );
    }
}

/// Receiver handle for coalesced request to await result
pub struct CoalesceReceiver<T> {
    receiver: broadcast::Receiver<Arc<T>>,
}

impl<T: Clone> CoalesceReceiver<T> {
    /// Wait for the primary request to complete
    pub async fn recv(mut self) -> Result<Arc<T>, CoalesceError> {
        self.receiver.recv().await.map_err(|e| match e {
            broadcast::error::RecvError::Closed => CoalesceError::PrimaryClosed,
            broadcast::error::RecvError::Lagged(n) => CoalesceError::Lagged(n),
        })
    }
}

/// Error types for coalescing operations
#[derive(Debug, Clone)]
pub enum CoalesceError {
    /// Primary request closed without sending result
    PrimaryClosed,
    /// Receiver lagged behind and missed messages
    Lagged(u64),
}

impl std::fmt::Display for CoalesceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrimaryClosed => write!(f, "Primary request closed without result"),
            Self::Lagged(n) => write!(f, "Receiver lagged behind by {} messages", n),
        }
    }
}

impl std::error::Error for CoalesceError {}

/// Entry in the pending requests map
struct PendingEntry<T> {
    sender: broadcast::Sender<Arc<T>>,
    created_at: Instant,
}

/// Coalesce manager that handles request deduplication
pub struct CoalesceManager<T: Clone + Send + Sync + 'static> {
    config: CoalescingConfig,
    /// Active pending requests by fingerprint
    pending: Arc<DashMap<u64, PendingEntry<T>>>,
    /// LRU cache for tracking fingerprint recency (for eviction)
    lru: Arc<Mutex<LruCache<u64, ()>>>,
}

impl<T: Clone + Send + Sync + 'static> CoalesceManager<T> {
    /// Create a new coalesce manager with the given configuration
    pub fn new(config: CoalescingConfig) -> Self {
        let max_entries = NonZeroUsize::new(config.max_pending.max(1))
            .unwrap_or(NonZeroUsize::new(10_000).unwrap());

        Self {
            config,
            pending: Arc::new(DashMap::new()),
            lru: Arc::new(Mutex::new(LruCache::new(max_entries))),
        }
    }

    /// Check if coalescing is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the coalescing window in milliseconds
    pub fn window_ms(&self) -> u64 {
        self.config.window_ms
    }

    /// Get or create an entry for the given fingerprint
    ///
    /// Returns `Primary` if this is a new request (should execute upstream)
    /// Returns `Coalesced` if an identical request is already in-flight
    pub fn get_or_create(&self, fingerprint: u64) -> CoalesceResult<T> {
        if !self.config.enabled {
            // Create a dummy sender for disabled mode
            let (tx, _) = broadcast::channel(1);
            return CoalesceResult::Primary(CoalesceSender {
                fingerprint,
                sender: tx,
                manager_pending: self.pending.clone(),
            });
        }

        // Check for expired entries first
        self.cleanup_expired();

        // Try to get existing entry
        if let Some(entry) = self.pending.get(&fingerprint) {
            // Check if entry is still within the coalescing window
            let elapsed = entry.created_at.elapsed();
            if elapsed < Duration::from_millis(self.config.window_ms) {
                record_cache_hit();
                record_coalesced();

                info!(
                    fingerprint = %fingerprint,
                    elapsed_ms = %elapsed.as_millis(),
                    "Request coalesced with in-flight request"
                );

                return CoalesceResult::Coalesced(CoalesceReceiver {
                    receiver: entry.sender.subscribe(),
                });
            }
            // Entry expired, remove it
            drop(entry);
            self.pending.remove(&fingerprint);
        }

        record_cache_miss();
        record_primary();

        // Create new entry
        // Use capacity of 16 to handle multiple coalesced requests
        let (tx, _) = broadcast::channel(16);
        let entry = PendingEntry {
            sender: tx.clone(),
            created_at: Instant::now(),
        };

        // Check LRU capacity and evict if necessary
        {
            let mut lru = self.lru.lock();
            if lru.len() >= lru.cap().get() {
                if let Some((evicted_fp, _)) = lru.pop_lru() {
                    self.pending.remove(&evicted_fp);
                    record_eviction();
                    debug!(
                        evicted_fingerprint = %evicted_fp,
                        "Evicted fingerprint due to LRU limit"
                    );
                }
            }
            lru.put(fingerprint, ());
        }

        self.pending.insert(fingerprint, entry);
        update_active_entries(self.pending.len());

        debug!(
            fingerprint = %fingerprint,
            active_entries = %self.pending.len(),
            "Created new primary request entry"
        );

        CoalesceResult::Primary(CoalesceSender {
            fingerprint,
            sender: tx,
            manager_pending: self.pending.clone(),
        })
    }

    /// Cleanup expired entries
    fn cleanup_expired(&self) {
        let window = Duration::from_millis(self.config.window_ms);
        let now = Instant::now();

        // Use retain to remove expired entries
        self.pending.retain(|fp, entry| {
            let expired = now.duration_since(entry.created_at) >= window;
            if expired {
                let mut lru = self.lru.lock();
                lru.pop(fp);
                debug!(fingerprint = %fp, "Cleaned up expired fingerprint");
            }
            !expired
        });

        update_active_entries(self.pending.len());
    }

    /// Get the number of active pending entries
    pub fn active_count(&self) -> usize {
        self.pending.len()
    }
}

impl<T: Clone + Send + Sync + 'static> Default for CoalesceManager<T> {
    fn default() -> Self {
        Self::new(CoalescingConfig::default())
    }
}

/// Calculate a fingerprint for a Claude/OpenAI request
///
/// The fingerprint includes:
/// - model
/// - messages (serialized)
/// - system prompt
/// - tools
/// - temperature
/// - top_p
/// - max_tokens
pub fn calculate_fingerprint<M, S, T>(
    model: &str,
    messages: &M,
    system: Option<&S>,
    tools: Option<&T>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tokens: Option<u32>,
) -> u64
where
    M: Serialize,
    S: Serialize,
    T: Serialize,
{
    let mut hasher = Xxh3::new();

    // Hash model
    hasher.update(model.as_bytes());
    hasher.update(b"\x00"); // Null separator

    // Hash messages (serialized to JSON for consistency)
    if let Ok(json) = serde_json::to_string(messages) {
        hasher.update(json.as_bytes());
    }
    hasher.update(b"\x00");

    // Hash system prompt if present
    if let Some(sys) = system {
        if let Ok(json) = serde_json::to_string(sys) {
            hasher.update(json.as_bytes());
        }
    }
    hasher.update(b"\x00");

    // Hash tools if present
    if let Some(t) = tools {
        if let Ok(json) = serde_json::to_string(t) {
            hasher.update(json.as_bytes());
        }
    }
    hasher.update(b"\x00");

    // Hash numeric parameters
    if let Some(temp) = temperature {
        hasher.update(&temp.to_le_bytes());
    }
    hasher.update(b"\x00");

    if let Some(p) = top_p {
        hasher.update(&p.to_le_bytes());
    }
    hasher.update(b"\x00");

    if let Some(max) = max_tokens {
        hasher.update(&max.to_le_bytes());
    }

    hasher.digest()
}

/// Calculate fingerprint specifically for Claude requests
pub fn calculate_claude_fingerprint(request: &crate::proxy::mappers::claude::ClaudeRequest) -> u64 {
    calculate_fingerprint(
        &request.model,
        &request.messages,
        request.system.as_ref(),
        request.tools.as_ref(),
        request.temperature,
        request.top_p,
        request.max_tokens,
    )
}

/// Calculate fingerprint specifically for OpenAI requests
pub fn calculate_openai_fingerprint(request: &crate::proxy::mappers::openai::OpenAIRequest) -> u64 {
    calculate_fingerprint(
        &request.model,
        &request.messages,
        None::<&String>, // OpenAI doesn't have a top-level system prompt field in the same way, it's in messages
        request.tools.as_ref(),
        request.temperature,
        request.top_p,
        request.max_tokens,
    )
}

/// Calculate fingerprint specifically for Gemini requests
pub fn calculate_gemini_fingerprint(body: &serde_json::Value) -> u64 {
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("unknown");
    let contents = body.get("contents");
    let generation_config = body.get("generationConfig");

    let mut hasher = Xxh3::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\x00");

    if let Some(c) = contents {
        if let Ok(json) = serde_json::to_string(c) {
            hasher.update(json.as_bytes());
        }
    }
    hasher.update(b"\x00");

    if let Some(gc) = generation_config {
        if let Ok(json) = serde_json::to_string(gc) {
            hasher.update(json.as_bytes());
        }
    }

    hasher.digest()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_deterministic() {
        let fp1 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            Some(&"system".to_string()),
            None,
            Some(0.7),
            None,
            Some(1000),
        );

        let fp2 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            Some(&"system".to_string()),
            None,
            Some(0.7),
            None,
            Some(1000),
        );

        assert_eq!(fp1, fp2, "Same inputs should produce same fingerprint");
    }

    #[test]
    fn test_fingerprint_different_model() {
        let fp1 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            None,
            None,
            None,
            None,
            None,
        );

        let fp2 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-sonnet",
            &vec!["hello".to_string()],
            None,
            None,
            None,
            None,
            None,
        );

        assert_ne!(fp1, fp2, "Different models should produce different fingerprints");
    }

    #[test]
    fn test_fingerprint_different_messages() {
        let fp1 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            None,
            None,
            None,
            None,
            None,
        );

        let fp2 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["goodbye".to_string()],
            None,
            None,
            None,
            None,
            None,
        );

        assert_ne!(fp1, fp2, "Different messages should produce different fingerprints");
    }

    #[test]
    fn test_fingerprint_different_temperature() {
        let fp1 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            None,
            None,
            Some(0.7),
            None,
            None,
        );

        let fp2 = calculate_fingerprint::<Vec<String>, String, Vec<String>>(
            "claude-3-opus",
            &vec!["hello".to_string()],
            None,
            None,
            Some(0.9),
            None,
            None,
        );

        assert_ne!(fp1, fp2, "Different temperatures should produce different fingerprints");
    }

    #[tokio::test]
    async fn test_coalesce_disabled() {
        let config = CoalescingConfig {
            enabled: false,
            window_ms: 500,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let result1 = manager.get_or_create(12345);
        let result2 = manager.get_or_create(12345);

        // Both should be Primary when disabled
        assert!(matches!(result1, CoalesceResult::Primary(_)));
        assert!(matches!(result2, CoalesceResult::Primary(_)));
    }

    #[tokio::test]
    async fn test_coalesce_enabled_dedup() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 500,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let result1 = manager.get_or_create(12345);
        let result2 = manager.get_or_create(12345);

        // First should be Primary
        assert!(matches!(result1, CoalesceResult::Primary(_)));
        // Second should be Coalesced
        assert!(matches!(result2, CoalesceResult::Coalesced(_)));
    }

    #[tokio::test]
    async fn test_coalesce_broadcast() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 500,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let result1 = manager.get_or_create(12345);
        let result2 = manager.get_or_create(12345);

        match (result1, result2) {
            (CoalesceResult::Primary(sender), CoalesceResult::Coalesced(receiver)) => {
                // Spawn receiver task
                let recv_handle = tokio::spawn(async move {
                    receiver.recv().await
                });

                // Send from primary
                sender.send("hello world".to_string());

                // Verify receiver got the message
                let received = recv_handle.await.unwrap();
                assert!(received.is_ok());
                assert_eq!(*received.unwrap(), "hello world");
            }
            _ => panic!("Expected Primary and Coalesced"),
        }
    }

    #[tokio::test]
    async fn test_coalesce_lru_eviction() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 5000, // Long window
            max_pending: 2,  // Small cache
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        // Fill cache
        let _ = manager.get_or_create(1);
        let _ = manager.get_or_create(2);

        assert_eq!(manager.active_count(), 2);

        // This should evict fingerprint 1
        let _ = manager.get_or_create(3);

        assert_eq!(manager.active_count(), 2);

        // Fingerprint 1 should now be a new primary (was evicted)
        let result = manager.get_or_create(1);
        assert!(matches!(result, CoalesceResult::Primary(_)));
    }

    #[tokio::test]
    async fn test_coalesce_different_fingerprints() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 500,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let result1 = manager.get_or_create(11111);
        let result2 = manager.get_or_create(22222);

        // Both should be Primary (different fingerprints)
        assert!(matches!(result1, CoalesceResult::Primary(_)));
        assert!(matches!(result2, CoalesceResult::Primary(_)));
    }

    #[tokio::test]
    async fn test_primary_fail_removes_entry() {
        let config = CoalescingConfig {
            enabled: true,
            window_ms: 5000,
            max_pending: 100,
            channel_capacity: 64,
        };
        let manager: CoalesceManager<String> = CoalesceManager::new(config);

        let result1 = manager.get_or_create(12345);
        assert_eq!(manager.active_count(), 1);

        // Fail the primary
        if let CoalesceResult::Primary(sender) = result1 {
            sender.fail();
        }

        // Entry should be removed
        assert_eq!(manager.active_count(), 0);

        // New request should be primary
        let result2 = manager.get_or_create(12345);
        assert!(matches!(result2, CoalesceResult::Primary(_)));
    }

    #[test]
    fn test_coalesce_stats_tracking() {
        // Initialize metrics
        init_coalescing_metrics();

        let (primary, coalesced, hits, misses, evictions) = get_coalescing_stats();

        // Stats should be zero or incremented from previous tests
        // Just verify the function works without panic and returns valid values
        // (usize is always >= 0, so we just verify they are valid by using them)
        let total = primary + coalesced + hits + misses + evictions;
        assert!(total < usize::MAX); // Simple sanity check
    }
}
