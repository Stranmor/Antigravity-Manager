// SSE Stream Resilience Module
//
// Provides error recovery, metrics tracking, and keepalive support for SSE streams.
// SOTA 2026: Zero-copy where possible, atomic metrics, graceful degradation.
//
// Components:
// - StreamMetrics: Track duration, bytes, errors, disconnections
// - PartialChunkBuffer: Handle incomplete SSE chunks across network boundaries
// - Heartbeat generator: Keep connections alive during slow responses
// - AbortHandler: Uses SmallVec to avoid heap allocations for typical callback counts

use bytes::{Bytes, BytesMut};
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

// === Configuration Constants ===
/// Heartbeat interval for keepalive comments (SSE spec: lines starting with : are comments)
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Maximum time to wait for a complete SSE chunk before considering it partial
pub const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum size for partial chunk buffer before forcing a flush
pub const MAX_PARTIAL_BUFFER_SIZE: usize = 65536; // 64KB
/// Default buffer capacity for stream processing
pub const DEFAULT_BUFFER_CAPACITY: usize = 8192;

// === Metrics Registration ===
static METRICS_REGISTERED: OnceLock<()> = OnceLock::new();

/// Initialize streaming metrics descriptors.
/// Safe to call multiple times - only registers once.
pub fn init_streaming_metrics() {
    METRICS_REGISTERED.get_or_init(|| {
        describe_counter!(
            "antigravity_stream_total",
            "Total number of SSE streams initiated"
        );
        describe_counter!(
            "antigravity_stream_completed",
            "Number of SSE streams completed successfully"
        );
        describe_counter!(
            "antigravity_stream_errors_total",
            "Total number of SSE stream errors"
        );
        describe_counter!(
            "antigravity_stream_premature_disconnections",
            "Number of streams terminated prematurely by client"
        );
        describe_histogram!(
            "antigravity_stream_duration_seconds",
            "SSE stream duration in seconds"
        );
        describe_counter!(
            "antigravity_stream_bytes_total",
            "Total bytes transferred over SSE streams"
        );
        describe_gauge!(
            "antigravity_stream_active",
            "Number of currently active SSE streams"
        );
        describe_counter!(
            "antigravity_stream_heartbeats_sent",
            "Number of keepalive heartbeats sent"
        );
        describe_counter!(
            "antigravity_stream_partial_chunks_recovered",
            "Number of partial SSE chunks successfully recovered"
        );
    });
}

// === Stream Metrics Tracker ===

/// Tracks metrics for a single SSE stream session.
/// Thread-safe via atomic operations.
pub struct StreamMetrics {
    start_time: Instant,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    chunks_sent: AtomicUsize,
    chunks_received: AtomicUsize,
    errors: AtomicUsize,
    partial_chunks_recovered: AtomicUsize,
    heartbeats_sent: AtomicUsize,
    is_completed: AtomicBool,
    is_aborted: AtomicBool,
    provider: String,
    model: String,
    last_activity: AtomicU64, // Epoch millis for timeout detection
}

impl StreamMetrics {
    /// Create a new stream metrics tracker.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        init_streaming_metrics();

        // Increment active streams gauge
        gauge!("antigravity_stream_active").increment(1.0);
        counter!("antigravity_stream_total").increment(1);

        let now = Instant::now();
        Self {
            start_time: now,
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            chunks_sent: AtomicUsize::new(0),
            chunks_received: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            partial_chunks_recovered: AtomicUsize::new(0),
            heartbeats_sent: AtomicUsize::new(0),
            is_completed: AtomicBool::new(false),
            is_aborted: AtomicBool::new(false),
            provider: provider.into(),
            model: model.into(),
            last_activity: AtomicU64::new(epoch_millis()),
        }
    }

    /// Record bytes sent to client.
    #[inline]
    pub fn record_bytes_sent(&self, bytes: u64) {
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
        self.chunks_sent.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// Record bytes received from upstream.
    #[inline]
    pub fn record_bytes_received(&self, bytes: u64) {
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
        self.chunks_received.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// Record a stream error.
    #[inline]
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        counter!("antigravity_stream_errors_total").increment(1);
    }

    /// Record successful recovery of a partial chunk.
    #[inline]
    pub fn record_partial_chunk_recovered(&self) {
        self.partial_chunks_recovered.fetch_add(1, Ordering::Relaxed);
        counter!("antigravity_stream_partial_chunks_recovered").increment(1);
    }

    /// Record a heartbeat sent.
    #[inline]
    pub fn record_heartbeat(&self) {
        self.heartbeats_sent.fetch_add(1, Ordering::Relaxed);
        counter!("antigravity_stream_heartbeats_sent").increment(1);
        self.touch();
    }

    /// Update last activity timestamp.
    #[inline]
    fn touch(&self) {
        self.last_activity.store(epoch_millis(), Ordering::Relaxed);
    }

    /// Check if the stream has timed out (no activity for CHUNK_TIMEOUT).
    pub fn is_timed_out(&self) -> bool {
        let last = self.last_activity.load(Ordering::Relaxed);
        let now = epoch_millis();
        now.saturating_sub(last) > CHUNK_TIMEOUT.as_millis() as u64
    }

    /// Get time since last activity.
    pub fn time_since_last_activity(&self) -> Duration {
        let last = self.last_activity.load(Ordering::Relaxed);
        let now = epoch_millis();
        Duration::from_millis(now.saturating_sub(last))
    }

    /// Mark stream as successfully completed.
    pub fn mark_completed(&self) {
        if !self.is_completed.swap(true, Ordering::AcqRel) {
            counter!("antigravity_stream_completed").increment(1);
        }
    }

    /// Mark stream as aborted (premature disconnection).
    pub fn mark_aborted(&self) {
        if !self.is_aborted.swap(true, Ordering::AcqRel) {
            counter!("antigravity_stream_premature_disconnections").increment(1);
            debug!(
                "[StreamMetrics] Stream aborted after {:.2}s, {} bytes sent",
                self.duration().as_secs_f64(),
                self.bytes_sent.load(Ordering::Relaxed)
            );
        }
    }

    /// Get stream duration.
    pub fn duration(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get total bytes sent.
    pub fn total_bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get error count.
    pub fn error_count(&self) -> usize {
        self.errors.load(Ordering::Relaxed)
    }
}

impl Drop for StreamMetrics {
    fn drop(&mut self) {
        // Decrement active streams gauge
        gauge!("antigravity_stream_active").decrement(1.0);

        // Record final metrics
        let duration_secs = self.start_time.elapsed().as_secs_f64();
        let bytes_total = self.bytes_sent.load(Ordering::Relaxed)
            + self.bytes_received.load(Ordering::Relaxed);

        let labels = [
            ("provider", self.provider.clone()),
            ("model", self.model.clone()),
        ];

        histogram!("antigravity_stream_duration_seconds", &labels).record(duration_secs);
        counter!("antigravity_stream_bytes_total", &labels).increment(bytes_total);

        debug!(
            "[StreamMetrics] Stream ended: duration={:.2}s, sent={} bytes, received={} bytes, errors={}, heartbeats={}",
            duration_secs,
            self.bytes_sent.load(Ordering::Relaxed),
            self.bytes_received.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.heartbeats_sent.load(Ordering::Relaxed)
        );
    }
}

/// Type alias for stack-optimized SSE line buffers.
/// Most SSE chunks contain 1-4 complete lines.
pub type LineVec = SmallVec<[Bytes; 4]>;

// === Partial Chunk Buffer ===

/// Handles incomplete SSE chunks that may be split across network packets.
/// Implements graceful recovery by buffering partial data until complete.
pub struct PartialChunkBuffer {
    buffer: BytesMut,
    last_update: Instant,
    recovery_attempts: usize,
}

impl PartialChunkBuffer {
    /// Create a new partial chunk buffer.
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(DEFAULT_BUFFER_CAPACITY),
            last_update: Instant::now(),
            recovery_attempts: 0,
        }
    }

    /// Append data to the buffer.
    pub fn append(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        self.last_update = Instant::now();
    }

    /// Check if buffer has pending data.
    pub fn has_pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Get pending data size.
    pub fn pending_size(&self) -> usize {
        self.buffer.len()
    }

    /// Check if buffer has timed out waiting for completion.
    pub fn is_stale(&self) -> bool {
        self.last_update.elapsed() > CHUNK_TIMEOUT
    }

    /// Check if buffer is oversized (potential memory issue).
    pub fn is_oversized(&self) -> bool {
        self.buffer.len() > MAX_PARTIAL_BUFFER_SIZE
    }

    /// Attempt to extract complete SSE lines from the buffer.
    /// Returns complete lines and leaves incomplete data in buffer.
    /// Uses SmallVec for stack allocation in common cases.
    pub fn extract_complete_lines(&mut self) -> LineVec {
        let mut lines = LineVec::new();
        let mut search_start = 0;

        // Find all complete lines (ending with \n)
        while let Some(pos) = self.buffer[search_start..].iter().position(|&b| b == b'\n') {
            let absolute_pos = search_start + pos + 1;
            let line = self.buffer.split_to(absolute_pos).freeze();
            lines.push(line);
            search_start = 0; // Reset after split
        }

        if !lines.is_empty() {
            self.recovery_attempts = 0; // Reset on success
        }

        lines
    }

    /// Force flush the buffer, returning any remaining data.
    /// Use when timeout or oversized conditions are met.
    pub fn force_flush(&mut self) -> Option<Bytes> {
        self.recovery_attempts += 1;

        if self.buffer.is_empty() {
            return None;
        }

        warn!(
            "[PartialChunkBuffer] Force flushing {} bytes after {} recovery attempts",
            self.buffer.len(),
            self.recovery_attempts
        );

        let data = self.buffer.split().freeze();
        Some(data)
    }

    /// Clear the buffer entirely.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.recovery_attempts = 0;
    }

    /// Get number of recovery attempts.
    pub fn recovery_attempts(&self) -> usize {
        self.recovery_attempts
    }
}

impl Default for PartialChunkBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// === Heartbeat Generator ===

/// Generates SSE keepalive comments to prevent connection timeouts.
/// SSE spec allows comment lines starting with ':' for keepalive.
pub struct HeartbeatGenerator {
    last_heartbeat: Instant,
    interval: Duration,
}

impl HeartbeatGenerator {
    /// Create a new heartbeat generator with default interval.
    pub fn new() -> Self {
        Self {
            last_heartbeat: Instant::now(),
            interval: HEARTBEAT_INTERVAL,
        }
    }

    /// Create with custom interval.
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            last_heartbeat: Instant::now(),
            interval,
        }
    }

    /// Check if a heartbeat should be sent now.
    pub fn should_send(&self) -> bool {
        self.last_heartbeat.elapsed() >= self.interval
    }

    /// Generate a heartbeat comment and reset timer.
    /// Returns SSE comment format: ": keepalive\n\n"
    pub fn generate(&mut self) -> Bytes {
        self.last_heartbeat = Instant::now();
        Bytes::from_static(b": keepalive\n\n")
    }

    /// Generate with timestamp for debugging.
    pub fn generate_with_timestamp(&mut self) -> Bytes {
        self.last_heartbeat = Instant::now();
        let ts = epoch_millis();
        Bytes::from(format!(": keepalive ts={ts}\n\n"))
    }

    /// Reset the timer without generating.
    pub fn reset(&mut self) {
        self.last_heartbeat = Instant::now();
    }

    /// Get time until next heartbeat is due.
    pub fn time_until_next(&self) -> Duration {
        self.interval.saturating_sub(self.last_heartbeat.elapsed())
    }
}

impl Default for HeartbeatGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// === Stream Abort Detection ===

/// Type alias for abort callback storage.
/// Most streams have 0-2 cleanup callbacks, so 2 elements fits on stack.
type AbortCallbackVec = SmallVec<[Box<dyn FnOnce() + Send>; 2]>;

/// Handles graceful cleanup when a stream is aborted by the client.
/// Uses SmallVec to avoid heap allocation for the common case of 0-2 callbacks.
pub struct AbortHandler {
    is_aborted: AtomicBool,
    cleanup_callbacks: std::sync::Mutex<AbortCallbackVec>,
}

impl AbortHandler {
    /// Create a new abort handler.
    pub fn new() -> Self {
        Self {
            is_aborted: AtomicBool::new(false),
            cleanup_callbacks: std::sync::Mutex::new(SmallVec::new()),
        }
    }

    /// Check if stream has been aborted.
    pub fn is_aborted(&self) -> bool {
        self.is_aborted.load(Ordering::Acquire)
    }

    /// Signal that the stream has been aborted.
    pub fn abort(&self) {
        if !self.is_aborted.swap(true, Ordering::AcqRel) {
            debug!("[AbortHandler] Stream abort signaled, running cleanup callbacks");
            if let Ok(mut callbacks) = self.cleanup_callbacks.lock() {
                for callback in callbacks.drain(..) {
                    callback();
                }
            }
        }
    }

    /// Register a cleanup callback to run on abort.
    pub fn on_abort<F: FnOnce() + Send + 'static>(&self, callback: F) {
        if self.is_aborted() {
            // Already aborted, run immediately
            callback();
        } else if let Ok(mut callbacks) = self.cleanup_callbacks.lock() {
            callbacks.push(Box::new(callback));
        }
    }
}

impl Default for AbortHandler {
    fn default() -> Self {
        Self::new()
    }
}

// === Utility Functions ===

/// Get current epoch time in milliseconds.
#[inline]
fn epoch_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Create an SSE error event for graceful error reporting to clients.
pub fn create_sse_error_event(error: &str, code: Option<&str>) -> Bytes {
    let code_str = code.unwrap_or("stream_error");
    let json = serde_json::json!({
        "type": "error",
        "error": {
            "type": code_str,
            "message": error
        }
    });
    let json_str = serde_json::to_string(&json).unwrap_or_default();
    Bytes::from(format!("event: error\ndata: {json_str}\n\n"))
}

/// Create an SSE comment for debugging/logging purposes.
#[inline]
pub fn create_sse_comment(message: &str) -> Bytes {
    Bytes::from(format!(": {message}\n\n"))
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_metrics_creation() {
        let metrics = StreamMetrics::new("test_provider", "test_model");
        assert!(!metrics.is_timed_out());
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.total_bytes_sent(), 0);
    }

    #[test]
    fn test_stream_metrics_recording() {
        let metrics = StreamMetrics::new("test", "model");
        metrics.record_bytes_sent(100);
        metrics.record_bytes_sent(200);
        assert_eq!(metrics.total_bytes_sent(), 300);

        metrics.record_error();
        metrics.record_error();
        assert_eq!(metrics.error_count(), 2);
    }

    #[test]
    fn test_partial_chunk_buffer() {
        let mut buffer = PartialChunkBuffer::new();
        assert!(!buffer.has_pending());

        buffer.append(b"data: {\"test\":");
        assert!(buffer.has_pending());

        // No complete line yet
        let lines = buffer.extract_complete_lines();
        assert!(lines.is_empty());
        assert!(buffer.has_pending());

        // Complete the line
        buffer.append(b"\"value\"}\n");
        let lines = buffer.extract_complete_lines();
        assert_eq!(lines.len(), 1);
        assert!(!buffer.has_pending());
    }

    #[test]
    fn test_partial_chunk_multiple_lines() {
        let mut buffer = PartialChunkBuffer::new();
        buffer.append(b"line1\nline2\npartial");

        let lines = buffer.extract_complete_lines();
        assert_eq!(lines.len(), 2);
        assert!(buffer.has_pending());
        assert_eq!(buffer.pending_size(), 7); // "partial"
    }

    #[test]
    fn test_heartbeat_generator() {
        let mut hb = HeartbeatGenerator::with_interval(Duration::from_millis(10));

        // Should not send immediately
        assert!(!hb.should_send());

        // Wait and check
        std::thread::sleep(Duration::from_millis(15));
        assert!(hb.should_send());

        // Generate resets timer
        let _ = hb.generate();
        assert!(!hb.should_send());
    }

    #[test]
    fn test_heartbeat_format() {
        let mut hb = HeartbeatGenerator::new();
        let heartbeat = hb.generate();
        let s = String::from_utf8(heartbeat.to_vec()).unwrap();
        assert!(s.starts_with(": "));
        assert!(s.ends_with("\n\n"));
    }

    #[test]
    fn test_abort_handler() {
        let handler = AbortHandler::new();
        assert!(!handler.is_aborted());

        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();
        handler.on_abort(move || {
            flag_clone.store(true, Ordering::Release);
        });

        handler.abort();
        assert!(handler.is_aborted());
        // Give callback time to run
        std::thread::sleep(Duration::from_millis(10));
        assert!(flag.load(Ordering::Acquire));
    }

    #[test]
    fn test_sse_error_event() {
        let event = create_sse_error_event("Something went wrong", Some("timeout"));
        let s = String::from_utf8(event.to_vec()).unwrap();
        assert!(s.contains("event: error"));
        assert!(s.contains("timeout"));
        assert!(s.contains("Something went wrong"));
    }

    #[test]
    fn test_sse_comment() {
        let comment = create_sse_comment("debug info");
        let s = String::from_utf8(comment.to_vec()).unwrap();
        assert!(s.starts_with(": debug info"));
        assert!(s.ends_with("\n\n"));
    }

    #[test]
    fn test_force_flush() {
        let mut buffer = PartialChunkBuffer::new();
        buffer.append(b"incomplete data without newline");

        assert!(buffer.has_pending());
        let flushed = buffer.force_flush();
        assert!(flushed.is_some());
        assert!(!buffer.has_pending());
        assert_eq!(buffer.recovery_attempts(), 1);
    }
}
