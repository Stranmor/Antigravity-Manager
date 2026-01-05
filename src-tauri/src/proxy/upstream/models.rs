//! Upstream API Models
//!
//! This module contains data structures for upstream API model management.
//! Currently reserved for future implementation of dynamic model discovery.

/// Upstream model registry for dynamic model discovery.
///
/// This struct will hold cached model information from upstream APIs,
/// enabling runtime model availability checks and capability detection.
///
/// # Future Implementation
/// - Model capability flags (vision, tools, streaming, etc.)
/// - Rate limit metadata per model
/// - Context window sizes
/// - Deprecation status
#[allow(dead_code)]
pub struct UpstreamModels {
    // Reserved for Phase 3: Dynamic Model Discovery
    // Will include fields like:
    // - available_models: HashMap<String, ModelInfo>
    // - last_refresh: SystemTime
    // - cache_ttl: Duration
}

impl Default for UpstreamModels {
    fn default() -> Self {
        Self::new()
    }
}

impl UpstreamModels {
    /// Create a new empty upstream models registry.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {}
    }
}
