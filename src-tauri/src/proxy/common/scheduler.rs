//! Priority Queue Scheduler with Multi-Level Queue (MLQ) and Deficit Round Robin (DRR)
//!
//! This module implements a fair priority queue scheduler for request processing.
//!
//! ## Key Features:
//! - Multi-Level Queue (MLQ) with 3 priority levels: High, Normal, Low
//! - Deficit Round Robin (DRR) for weighted fair dequeuing
//! - Priority aging to prevent starvation
//! - Backpressure via 429 when queue depth exceeds limit
//!
//! ## Priority Classification:
//! 1. Explicit `x-priority` header (0=High, 1=Normal, 2=Low)
//! 2. Account tier: ULTRA -> High, PRO -> Normal, FREE -> Low
//! 3. Model type: Fast models (haiku, 4o-mini) -> High, Large (opus, gpt-4) -> Normal
//! 4. Default: Normal
//!
//! ## Fairness Mechanism:
//! - Weighted Fair Queuing: High=5, Normal=2, Low=1
//! - Aging: Priority boosted if wait time exceeds threshold (default 500ms)
//!
//! ## Usage:
//! ```rust,ignore
//! let scheduler = PriorityScheduler::new(config);
//!
//! // Classify and enqueue a request
//! let priority = scheduler.classify_priority(headers, tier, model);
//! match scheduler.enqueue(request, priority) {
//!     Ok(()) => { /* queued successfully */ }
//!     Err(SchedulerError::QueueFull) => { /* return 429 */ }
//! }
//!
//! // Dequeue next request (applies aging + DRR)
//! if let Some(request) = scheduler.dequeue() {
//!     // Process request
//! }
//! ```

use crate::proxy::config::SchedulerConfig;
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Global scheduler statistics
static SCHEDULER_STATS: OnceLock<SchedulerStats> = OnceLock::new();

/// Priority level for request scheduling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PriorityLevel {
    /// High priority: ULTRA tier, fast models, explicit priority 0
    High = 0,
    /// Normal priority: PRO tier, standard models, default
    #[default]
    Normal = 1,
    /// Low priority: FREE tier, large models, explicit priority 2
    Low = 2,
}

impl PriorityLevel {
    /// Create from numeric value (0=High, 1=Normal, 2=Low)
    pub fn from_value(value: u8) -> Self {
        match value {
            0 => Self::High,
            1 => Self::Normal,
            _ => Self::Low,
        }
    }

    /// Get the numeric value
    pub fn as_value(&self) -> u8 {
        *self as u8
    }

    /// Get the label for metrics
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }

    #[must_use]
    pub fn boosted(&self) -> Self {
        match self {
            Self::Low => Self::Normal,
            Self::Normal | Self::High => Self::High,
        }
    }
}

impl std::fmt::Display for PriorityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_label())
    }
}

/// Metadata for a queued request
#[derive(Debug, Clone)]
pub struct QueuedRequest<T> {
    /// The actual request payload
    pub payload: T,
    /// Request ID for tracing
    pub request_id: String,
    /// Original priority classification
    pub original_priority: PriorityLevel,
    /// Current priority (may be boosted due to aging)
    pub current_priority: PriorityLevel,
    /// Timestamp when request was enqueued
    pub enqueued_at: Instant,
    /// Weight for DRR scheduling (higher = more quota)
    pub weight: u32,
    /// Whether this is a streaming request
    pub is_streaming: bool,
    /// Account ID for attribution
    pub account_id: Option<String>,
    /// Account tier (ULTRA/PRO/FREE)
    pub account_tier: Option<String>,
    /// Model name
    pub model: Option<String>,
}

impl<T> QueuedRequest<T> {
    /// Create a new queued request
    pub fn new(
        payload: T,
        request_id: String,
        priority: PriorityLevel,
        weight: u32,
        is_streaming: bool,
    ) -> Self {
        Self {
            payload,
            request_id,
            original_priority: priority,
            current_priority: priority,
            enqueued_at: Instant::now(),
            weight,
            is_streaming,
            account_id: None,
            account_tier: None,
            model: None,
        }
    }

    #[must_use]
    pub fn with_account(mut self, account_id: String, tier: Option<String>) -> Self {
        self.account_id = Some(account_id);
        self.account_tier = tier;
        self
    }

    #[must_use]
    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    /// Get time spent waiting in queue
    pub fn wait_time(&self) -> Duration {
        self.enqueued_at.elapsed()
    }

    /// Check if request has exceeded aging threshold
    pub fn needs_aging(&self, threshold: Duration) -> bool {
        self.wait_time() >= threshold && self.current_priority != PriorityLevel::High
    }

    /// Apply aging boost to priority
    pub fn apply_aging(&mut self) {
        if self.current_priority != PriorityLevel::High {
            let old = self.current_priority;
            self.current_priority = self.current_priority.boosted();
            debug!(
                request_id = %self.request_id,
                old_priority = %old,
                new_priority = %self.current_priority,
                wait_ms = %self.wait_time().as_millis(),
                "Applied aging boost to request"
            );
        }
    }
}

/// Statistics for scheduler operations
#[derive(Debug)]
pub struct SchedulerStats {
    /// Total requests enqueued
    pub enqueued: AtomicU64,
    /// Total requests dequeued
    pub dequeued: AtomicU64,
    /// Total requests dropped due to queue full
    pub dropped: AtomicU64,
    /// Total starvation boosts (aging)
    pub starvation_boosts: AtomicU64,
    /// Current queue depth (high)
    pub queue_depth_high: AtomicUsize,
    /// Current queue depth (normal)
    pub queue_depth_normal: AtomicUsize,
    /// Current queue depth (low)
    pub queue_depth_low: AtomicUsize,
}

impl Default for SchedulerStats {
    fn default() -> Self {
        Self {
            enqueued: AtomicU64::new(0),
            dequeued: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            starvation_boosts: AtomicU64::new(0),
            queue_depth_high: AtomicUsize::new(0),
            queue_depth_normal: AtomicUsize::new(0),
            queue_depth_low: AtomicUsize::new(0),
        }
    }
}

/// Initialize scheduler metrics (call once at startup)
pub fn init_scheduler_metrics() {
    let _ = SCHEDULER_STATS.get_or_init(SchedulerStats::default);

    describe_gauge!(
        "antigravity_queue_depth",
        "Current number of requests waiting in queue by priority"
    );
    describe_histogram!(
        "antigravity_queue_wait_seconds",
        "Time spent waiting in queue before dequeue"
    );
    describe_counter!(
        "antigravity_queue_dropped_total",
        "Total requests dropped due to queue depth limit"
    );
    describe_counter!(
        "antigravity_starvation_boosts_total",
        "Total priority boosts due to aging (starvation prevention)"
    );
    describe_counter!(
        "antigravity_queue_enqueued_total",
        "Total requests enqueued by priority"
    );
    describe_counter!(
        "antigravity_queue_dequeued_total",
        "Total requests dequeued by priority"
    );
}

/// Get current scheduler statistics
pub fn get_scheduler_stats() -> (u64, u64, u64, u64, usize, usize, usize) {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    (
        stats.enqueued.load(Ordering::Relaxed),
        stats.dequeued.load(Ordering::Relaxed),
        stats.dropped.load(Ordering::Relaxed),
        stats.starvation_boosts.load(Ordering::Relaxed),
        stats.queue_depth_high.load(Ordering::Relaxed),
        stats.queue_depth_normal.load(Ordering::Relaxed),
        stats.queue_depth_low.load(Ordering::Relaxed),
    )
}

fn record_enqueued(priority: PriorityLevel) {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    stats.enqueued.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_queue_enqueued_total", "priority" => priority.as_label()).increment(1);
}

fn record_dequeued(priority: PriorityLevel, wait_time: Duration) {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    stats.dequeued.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_queue_dequeued_total", "priority" => priority.as_label()).increment(1);

    // Record wait time histogram
    let wait_secs = wait_time.as_secs_f64();
    histogram!("antigravity_queue_wait_seconds", "priority" => priority.as_label())
        .record(wait_secs);
}

fn record_dropped(priority: PriorityLevel) {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    stats.dropped.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_queue_dropped_total", "priority" => priority.as_label()).increment(1);
}

fn record_starvation_boost() {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    stats.starvation_boosts.fetch_add(1, Ordering::Relaxed);
    counter!("antigravity_starvation_boosts_total").increment(1);
}

fn update_queue_depth(high: usize, normal: usize, low: usize) {
    let stats = SCHEDULER_STATS.get_or_init(SchedulerStats::default);
    stats.queue_depth_high.store(high, Ordering::Relaxed);
    stats.queue_depth_normal.store(normal, Ordering::Relaxed);
    stats.queue_depth_low.store(low, Ordering::Relaxed);

    gauge!("antigravity_queue_depth", "priority" => "high").set(high as f64);
    gauge!("antigravity_queue_depth", "priority" => "normal").set(normal as f64);
    gauge!("antigravity_queue_depth", "priority" => "low").set(low as f64);
}

/// Error types for scheduler operations
#[derive(Debug, Clone)]
pub enum SchedulerError {
    /// Queue is full - return 429 with retry delay
    QueueFull {
        /// Recommended retry delay
        retry_after: Duration,
        /// Current total queue depth
        current_depth: usize,
    },
    /// Scheduler is disabled
    Disabled,
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull {
                retry_after,
                current_depth,
            } => {
                write!(
                    f,
                    "Queue full ({} requests), retry after {}s",
                    current_depth,
                    retry_after.as_secs()
                )
            }
            Self::Disabled => write!(f, "Scheduler is disabled"),
        }
    }
}

impl std::error::Error for SchedulerError {}

/// Multi-Level Queue for a single priority level
#[derive(Debug)]
struct PriorityQueue<T> {
    /// The queue itself
    queue: VecDeque<QueuedRequest<T>>,
    /// Deficit counter for DRR
    deficit: u32,
    /// Weight for this queue (quantum per round)
    weight: u32,
}

impl<T> PriorityQueue<T> {
    fn new(weight: u32) -> Self {
        Self {
            queue: VecDeque::new(),
            deficit: 0,
            weight,
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn push_back(&mut self, request: QueuedRequest<T>) {
        self.queue.push_back(request);
    }

    fn pop_front(&mut self) -> Option<QueuedRequest<T>> {
        self.queue.pop_front()
    }

    /// Add quantum to deficit
    fn add_quantum(&mut self) {
        self.deficit += self.weight;
    }

    /// Consume deficit for a request
    fn consume_deficit(&mut self, amount: u32) -> bool {
        if self.deficit >= amount {
            self.deficit -= amount;
            true
        } else {
            false
        }
    }

    /// Reset deficit (when queue becomes empty)
    fn reset_deficit(&mut self) {
        self.deficit = 0;
    }
}

/// Priority Queue Scheduler with MLQ and DRR
///
/// Manages three priority queues (High, Normal, Low) and implements
/// Deficit Round Robin for fair scheduling across priorities.
pub struct PriorityScheduler<T> {
    config: SchedulerConfig,
    /// High priority queue
    high: Mutex<PriorityQueue<T>>,
    /// Normal priority queue
    normal: Mutex<PriorityQueue<T>>,
    /// Low priority queue
    low: Mutex<PriorityQueue<T>>,
    /// Current DRR round position (which queue to try next)
    round_position: RwLock<usize>,
}

impl<T: Send + Sync + 'static> PriorityScheduler<T> {
    /// Create a new scheduler with the given configuration
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            high: Mutex::new(PriorityQueue::new(config.high_weight)),
            normal: Mutex::new(PriorityQueue::new(config.normal_weight)),
            low: Mutex::new(PriorityQueue::new(config.low_weight)),
            round_position: RwLock::new(0),
            config,
        }
    }

    /// Check if the scheduler is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Classify priority based on headers, account tier, and model
    ///
    /// Priority order (first match wins):
    /// 1. Explicit `x-priority` header (0=High, 1=Normal, 2=Low)
    /// 2. Account tier: ULTRA -> High, PRO -> Normal, FREE -> Low
    /// 3. Model type: Fast models -> High, Large models -> Normal
    /// 4. Default: Normal
    pub fn classify_priority(
        &self,
        x_priority_header: Option<&str>,
        account_tier: Option<&str>,
        model: Option<&str>,
    ) -> PriorityLevel {
        // 1. Check explicit x-priority header
        if let Some(header_val) = x_priority_header {
            if let Ok(val) = header_val.parse::<u8>() {
                return PriorityLevel::from_value(val);
            }
        }

        // 2. Check account tier
        if let Some(tier) = account_tier {
            let tier_upper = tier.to_uppercase();
            match tier_upper.as_str() {
                "ULTRA" => return PriorityLevel::High,
                "PRO" => return PriorityLevel::Normal,
                "FREE" => return PriorityLevel::Low,
                _ => {}
            }
        }

        // 3. Check model type
        if let Some(model_name) = model {
            let model_lower = model_name.to_lowercase();

            // Fast/small models get high priority
            if model_lower.contains("haiku")
                || model_lower.contains("4o-mini")
                || model_lower.contains("flash")
                || model_lower.contains("instant")
            {
                return PriorityLevel::High;
            }

            // Large/slow models get normal priority (not low, to avoid starvation)
            if model_lower.contains("opus")
                || model_lower.contains("gpt-4")
                || model_lower.contains("ultra")
            {
                return PriorityLevel::Normal;
            }
        }

        // 4. Default to Normal
        PriorityLevel::Normal
    }

    /// Get the total queue depth across all priorities
    pub fn total_depth(&self) -> usize {
        self.high.lock().len() + self.normal.lock().len() + self.low.lock().len()
    }

    /// Get individual queue depths
    pub fn queue_depths(&self) -> (usize, usize, usize) {
        (
            self.high.lock().len(),
            self.normal.lock().len(),
            self.low.lock().len(),
        )
    }

    /// Enqueue a request with the given priority
    ///
    /// Returns `Err(SchedulerError::QueueFull)` if total depth exceeds limit
    pub fn enqueue(&self, request: QueuedRequest<T>) -> Result<(), SchedulerError> {
        if !self.config.enabled {
            return Err(SchedulerError::Disabled);
        }

        let total_depth = self.total_depth();
        if total_depth >= self.config.max_queue_depth {
            let priority = request.current_priority;
            record_dropped(priority);

            warn!(
                request_id = %request.request_id,
                priority = %priority,
                current_depth = %total_depth,
                max_depth = %self.config.max_queue_depth,
                "Queue full, request dropped"
            );

            return Err(SchedulerError::QueueFull {
                retry_after: Duration::from_secs(5),
                current_depth: total_depth,
            });
        }

        let priority = request.current_priority;
        match priority {
            PriorityLevel::High => self.high.lock().push_back(request),
            PriorityLevel::Normal => self.normal.lock().push_back(request),
            PriorityLevel::Low => self.low.lock().push_back(request),
        }

        record_enqueued(priority);

        let (high, normal, low) = self.queue_depths();
        update_queue_depth(high, normal, low);

        debug!(
            priority = %priority,
            queue_depth_high = %high,
            queue_depth_normal = %normal,
            queue_depth_low = %low,
            "Request enqueued"
        );

        Ok(())
    }

    /// Apply aging to all queues - boost priority of starving requests
    ///
    /// Requests that have waited longer than `aging_threshold_ms` get promoted
    /// to the next higher priority level.
    pub fn apply_aging(&self) {
        let threshold = Duration::from_millis(self.config.aging_threshold_ms);

        // Collect requests that need aging from low queue
        let mut low_to_promote: Vec<QueuedRequest<T>> = Vec::new();
        {
            let mut low = self.low.lock();
            let mut remaining = VecDeque::new();
            while let Some(mut req) = low.pop_front() {
                if req.needs_aging(threshold) {
                    req.apply_aging();
                    record_starvation_boost();
                    low_to_promote.push(req);
                } else {
                    remaining.push_back(req);
                }
            }
            low.queue = remaining;
        }

        // Collect requests that need aging from normal queue
        let mut normal_to_promote: Vec<QueuedRequest<T>> = Vec::new();
        {
            let mut normal = self.normal.lock();
            let mut remaining = VecDeque::new();
            while let Some(mut req) = normal.pop_front() {
                if req.needs_aging(threshold) {
                    req.apply_aging();
                    record_starvation_boost();
                    normal_to_promote.push(req);
                } else {
                    remaining.push_back(req);
                }
            }
            normal.queue = remaining;
        }

        // Promote low -> normal
        if !low_to_promote.is_empty() {
            let mut normal = self.normal.lock();
            for req in low_to_promote {
                normal.push_back(req);
            }
        }

        // Promote normal -> high
        if !normal_to_promote.is_empty() {
            let mut high = self.high.lock();
            for req in normal_to_promote {
                high.push_back(req);
            }
        }

        // Update queue depth metrics
        let (high, normal, low) = self.queue_depths();
        update_queue_depth(high, normal, low);
    }

    /// Dequeue the next request using Deficit Round Robin
    ///
    /// Applies aging first, then uses DRR to fairly select from queues.
    /// Higher priority queues have higher weight (more requests per round).
    pub fn dequeue(&self) -> Option<QueuedRequest<T>> {
        if !self.config.enabled {
            return None;
        }

        // Apply aging before dequeue
        self.apply_aging();

        // DRR: Try each queue in priority order, with deficit tracking
        let queues: [PriorityLevel; 3] = [PriorityLevel::High, PriorityLevel::Normal, PriorityLevel::Low];

        // Add quantum to all non-empty queues
        {
            let mut high = self.high.lock();
            let mut normal = self.normal.lock();
            let mut low = self.low.lock();

            if !high.is_empty() {
                high.add_quantum();
            }
            if !normal.is_empty() {
                normal.add_quantum();
            }
            if !low.is_empty() {
                low.add_quantum();
            }
        }

        // Get starting position for round-robin fairness
        let start_pos = *self.round_position.read();

        for offset in 0..3 {
            let idx = (start_pos + offset) % 3;
            let priority = queues[idx];

            let result = match priority {
                PriorityLevel::High => {
                    let mut queue = self.high.lock();
                    Self::try_dequeue_from(&mut queue)
                }
                PriorityLevel::Normal => {
                    let mut queue = self.normal.lock();
                    Self::try_dequeue_from(&mut queue)
                }
                PriorityLevel::Low => {
                    let mut queue = self.low.lock();
                    Self::try_dequeue_from(&mut queue)
                }
            };

            if let Some(request) = result {
                // Update round position for next dequeue
                *self.round_position.write() = (idx + 1) % 3;

                // Record metrics
                record_dequeued(request.current_priority, request.wait_time());

                // Update queue depths
                let (high, normal, low) = self.queue_depths();
                update_queue_depth(high, normal, low);

                debug!(
                    request_id = %request.request_id,
                    priority = %request.current_priority,
                    original_priority = %request.original_priority,
                    wait_ms = %request.wait_time().as_millis(),
                    "Request dequeued"
                );

                return Some(request);
            }
        }

        None
    }

    fn try_dequeue_from(queue: &mut PriorityQueue<T>) -> Option<QueuedRequest<T>> {
        if queue.is_empty() {
            queue.reset_deficit();
            return None;
        }

        // Check if we have enough deficit to serve a request
        // Each request costs 1 unit
        if queue.consume_deficit(1) {
            queue.pop_front()
        } else {
            None
        }
    }

    /// Get config for inspection
    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }
}

impl<T: Send + Sync + 'static> Default for PriorityScheduler<T> {
    fn default() -> Self {
        Self::new(SchedulerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_level_from_value() {
        assert_eq!(PriorityLevel::from_value(0), PriorityLevel::High);
        assert_eq!(PriorityLevel::from_value(1), PriorityLevel::Normal);
        assert_eq!(PriorityLevel::from_value(2), PriorityLevel::Low);
        assert_eq!(PriorityLevel::from_value(3), PriorityLevel::Low); // Out of range
        assert_eq!(PriorityLevel::from_value(255), PriorityLevel::Low);
    }

    #[test]
    fn test_priority_level_boosted() {
        assert_eq!(PriorityLevel::Low.boosted(), PriorityLevel::Normal);
        assert_eq!(PriorityLevel::Normal.boosted(), PriorityLevel::High);
        assert_eq!(PriorityLevel::High.boosted(), PriorityLevel::High); // Already highest
    }

    #[test]
    fn test_classify_priority_header() {
        let config = SchedulerConfig {
            enabled: true,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Explicit header takes precedence
        assert_eq!(
            scheduler.classify_priority(Some("0"), Some("FREE"), Some("opus")),
            PriorityLevel::High
        );
        assert_eq!(
            scheduler.classify_priority(Some("2"), Some("ULTRA"), Some("haiku")),
            PriorityLevel::Low
        );
    }

    #[test]
    fn test_classify_priority_tier() {
        let config = SchedulerConfig {
            enabled: true,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Tier classification
        assert_eq!(
            scheduler.classify_priority(None, Some("ULTRA"), None),
            PriorityLevel::High
        );
        assert_eq!(
            scheduler.classify_priority(None, Some("PRO"), None),
            PriorityLevel::Normal
        );
        assert_eq!(
            scheduler.classify_priority(None, Some("FREE"), None),
            PriorityLevel::Low
        );

        // Case insensitive
        assert_eq!(
            scheduler.classify_priority(None, Some("ultra"), None),
            PriorityLevel::High
        );
    }

    #[test]
    fn test_classify_priority_model() {
        let config = SchedulerConfig {
            enabled: true,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Fast models -> High
        assert_eq!(
            scheduler.classify_priority(None, None, Some("claude-3-haiku")),
            PriorityLevel::High
        );
        assert_eq!(
            scheduler.classify_priority(None, None, Some("gpt-4o-mini")),
            PriorityLevel::High
        );
        assert_eq!(
            scheduler.classify_priority(None, None, Some("gemini-1.5-flash")),
            PriorityLevel::High
        );

        // Large models -> Normal (not Low to avoid starvation)
        assert_eq!(
            scheduler.classify_priority(None, None, Some("claude-3-opus")),
            PriorityLevel::Normal
        );
        assert_eq!(
            scheduler.classify_priority(None, None, Some("gpt-4")),
            PriorityLevel::Normal
        );

        // Unknown models -> Normal
        assert_eq!(
            scheduler.classify_priority(None, None, Some("claude-sonnet-4")),
            PriorityLevel::Normal
        );
    }

    #[test]
    fn test_enqueue_dequeue() {
        let config = SchedulerConfig {
            enabled: true,
            max_queue_depth: 100,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Enqueue a request
        let request = QueuedRequest::new(
            "test payload".to_string(),
            "req-1".to_string(),
            PriorityLevel::Normal,
            1,
            false,
        );

        assert!(scheduler.enqueue(request).is_ok());
        assert_eq!(scheduler.total_depth(), 1);

        // Dequeue
        let dequeued = scheduler.dequeue();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().payload, "test payload");
        assert_eq!(scheduler.total_depth(), 0);
    }

    #[test]
    fn test_queue_full_backpressure() {
        let config = SchedulerConfig {
            enabled: true,
            max_queue_depth: 2,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Fill the queue
        for i in 0..2 {
            let request = QueuedRequest::new(
                format!("payload-{}", i),
                format!("req-{}", i),
                PriorityLevel::Normal,
                1,
                false,
            );
            assert!(scheduler.enqueue(request).is_ok());
        }

        // Third request should fail
        let request = QueuedRequest::new(
            "payload-overflow".to_string(),
            "req-overflow".to_string(),
            PriorityLevel::Normal,
            1,
            false,
        );

        let result = scheduler.enqueue(request);
        assert!(result.is_err());

        match result.unwrap_err() {
            SchedulerError::QueueFull { current_depth, .. } => {
                assert_eq!(current_depth, 2);
            }
            _ => panic!("Expected QueueFull error"),
        }
    }

    #[test]
    fn test_priority_ordering() {
        let config = SchedulerConfig {
            enabled: true,
            max_queue_depth: 100,
            high_weight: 5,
            normal_weight: 2,
            low_weight: 1,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Enqueue in reverse priority order
        for (priority, label) in [
            (PriorityLevel::Low, "low"),
            (PriorityLevel::Normal, "normal"),
            (PriorityLevel::High, "high"),
        ] {
            let request = QueuedRequest::new(
                label.to_string(),
                format!("req-{}", label),
                priority,
                1,
                false,
            );
            scheduler.enqueue(request).unwrap();
        }

        // High priority should come first due to higher weight
        let first = scheduler.dequeue().unwrap();
        assert_eq!(first.payload, "high");
    }

    #[test]
    fn test_queued_request_needs_aging() {
        let mut request: QueuedRequest<String> = QueuedRequest::new(
            "test".to_string(),
            "req-1".to_string(),
            PriorityLevel::Low,
            1,
            false,
        );

        // Should not need aging immediately
        assert!(!request.needs_aging(Duration::from_millis(500)));

        // High priority should never need aging
        request.current_priority = PriorityLevel::High;
        assert!(!request.needs_aging(Duration::from_millis(0)));
    }

    #[test]
    fn test_scheduler_disabled() {
        let config = SchedulerConfig {
            enabled: false,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        let request = QueuedRequest::new(
            "test".to_string(),
            "req-1".to_string(),
            PriorityLevel::Normal,
            1,
            false,
        );

        let result = scheduler.enqueue(request);
        assert!(matches!(result, Err(SchedulerError::Disabled)));

        assert!(scheduler.dequeue().is_none());
    }

    #[test]
    fn test_queued_request_with_metadata() {
        let request: QueuedRequest<String> = QueuedRequest::new(
            "test".to_string(),
            "req-1".to_string(),
            PriorityLevel::Normal,
            1,
            false,
        )
        .with_account("acc-123".to_string(), Some("ULTRA".to_string()))
        .with_model("claude-3-opus".to_string());

        assert_eq!(request.account_id, Some("acc-123".to_string()));
        assert_eq!(request.account_tier, Some("ULTRA".to_string()));
        assert_eq!(request.model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_queue_depths() {
        let config = SchedulerConfig {
            enabled: true,
            max_queue_depth: 100,
            ..Default::default()
        };
        let scheduler: PriorityScheduler<String> = PriorityScheduler::new(config);

        // Enqueue to different queues
        for (priority, count) in [
            (PriorityLevel::High, 3),
            (PriorityLevel::Normal, 2),
            (PriorityLevel::Low, 1),
        ] {
            for i in 0..count {
                let request = QueuedRequest::new(
                    format!("{:?}-{}", priority, i),
                    format!("req-{:?}-{}", priority, i),
                    priority,
                    1,
                    false,
                );
                scheduler.enqueue(request).unwrap();
            }
        }

        let (high, normal, low) = scheduler.queue_depths();
        assert_eq!(high, 3);
        assert_eq!(normal, 2);
        assert_eq!(low, 1);
        assert_eq!(scheduler.total_depth(), 6);
    }
}
