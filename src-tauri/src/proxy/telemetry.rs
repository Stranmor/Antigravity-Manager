//! OpenTelemetry distributed tracing for Antigravity Manager proxy.
//!
//! This module provides OTLP-compatible distributed tracing that integrates
//! with the existing tracing infrastructure. When the `otel` feature is enabled,
//! traces are exported to an OTLP collector (Jaeger, Grafana Tempo, etc.).
//!
//! # Configuration
//!
//! Environment variables:
//! - `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP collector endpoint (default: `http://localhost:4317`)
//! - `OTEL_SERVICE_NAME`: Service name for traces (default: `antigravity-proxy`)
//! - `OTEL_ENABLED`: Enable/disable OTEL export (default: `true` when feature enabled)
//!
//! # Usage
//!
//! ```rust,ignore
//! // Initialize at startup
//! let _guard = telemetry::init_telemetry()?;
//!
//! // Use normal tracing macros - spans are automatically exported
//! tracing::info_span!("my_operation", key = "value").in_scope(|| {
//!     // ... your code
//! });
//!
//! // On shutdown, guard drop will flush traces
//! ```

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel")]
use opentelemetry::{KeyValue, InstrumentationScope};
#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use opentelemetry_sdk::trace::TracerProvider;

/// Default OTLP endpoint for local development
#[cfg(feature = "otel")]
const DEFAULT_OTLP_ENDPOINT: &str = "http://localhost:4317";

/// Default service name for traces
#[cfg(feature = "otel")]
const DEFAULT_SERVICE_NAME: &str = "antigravity-proxy";

/// Guard that ensures graceful shutdown of the tracer provider.
/// Drop this to flush pending spans before application exit.
#[cfg(feature = "otel")]
pub struct TelemetryGuard {
    provider: TracerProvider,
}

#[cfg(feature = "otel")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        tracing::info!("Shutting down OpenTelemetry tracer provider...");
        if let Err(e) = self.provider.shutdown() {
            tracing::error!("Failed to shutdown OpenTelemetry provider: {:?}", e);
        } else {
            tracing::info!("OpenTelemetry tracer provider shutdown complete");
        }
    }
}

/// Configuration for OpenTelemetry telemetry
#[cfg(feature = "otel")]
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// OTLP exporter endpoint (gRPC)
    pub endpoint: String,
    /// Service name for traces
    pub service_name: String,
    /// Enable tracing export
    pub enabled: bool,
}

#[cfg(feature = "otel")]
impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_OTLP_ENDPOINT.to_string()),
            service_name: std::env::var("OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| DEFAULT_SERVICE_NAME.to_string()),
            enabled: std::env::var("OTEL_ENABLED")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true),
        }
    }
}

/// Initialize OpenTelemetry tracing with OTLP exporter.
///
/// This function sets up the OpenTelemetry pipeline with:
/// - OTLP gRPC exporter for traces
/// - Integration with the tracing crate via tracing-opentelemetry
/// - Batch span processor for efficient export
///
/// Returns a guard that should be kept alive for the duration of the application.
/// When dropped, it will gracefully shutdown the tracer and flush pending spans.
///
/// # Errors
///
/// Returns an error if the OTLP exporter or tracer provider fails to initialize.
#[cfg(feature = "otel")]
pub fn init_telemetry() -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    init_telemetry_with_config(TelemetryConfig::default())
}

/// Initialize OpenTelemetry with custom configuration.
///
/// NOTE: This function now sets up OTEL WITHOUT trying to create a new tracing subscriber.
/// The server logger is expected to already have initialized the global tracing subscriber.
/// We only set up the OTEL tracer provider and register it globally for direct OTEL API usage.
#[cfg(feature = "otel")]
pub fn init_telemetry_with_config(
    config: TelemetryConfig,
) -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        tracing::info!("OpenTelemetry tracing is disabled");
        // Return a dummy provider that does nothing
        let provider = TracerProvider::builder().build();
        return Ok(TelemetryGuard { provider });
    }

    tracing::info!(
        "Initializing OpenTelemetry tracing: endpoint={}, service={}",
        config.endpoint,
        config.service_name
    );

    // Configure OTLP exporter
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()?;

    // Build resource with service name
    let resource = opentelemetry_sdk::Resource::new(vec![
        KeyValue::new("service.name", config.service_name.clone()),
    ]);

    // Build tracer provider with batch processor
    let provider = TracerProvider::builder()
        .with_batch_exporter(otlp_exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();

    // Set the provider globally so OTEL API calls (opentelemetry::global::tracer()) work
    // This does NOT conflict with tracing-subscriber - it's a separate system
    opentelemetry::global::set_tracer_provider(provider.clone());

    tracing::info!(
        "OpenTelemetry provider registered globally. Exporting to: {}",
        config.endpoint
    );

    Ok(TelemetryGuard { provider })
}

/// Initialize OpenTelemetry as an additional layer on existing subscriber.
///
/// Use this when you already have a tracing subscriber configured and want to
/// add OTEL export on top of it.
#[cfg(feature = "otel")]
pub fn create_otel_layer(
    config: TelemetryConfig,
) -> Result<
    (
        tracing_opentelemetry::OpenTelemetryLayer<tracing_subscriber::Registry, opentelemetry_sdk::trace::Tracer>,
        TelemetryGuard,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    if !config.enabled {
        tracing::info!("OpenTelemetry tracing is disabled");
        let provider = TracerProvider::builder().build();
        let scope = InstrumentationScope::builder("noop").build();
        let tracer = provider.tracer_with_scope(scope);
        let layer = tracing_opentelemetry::layer().with_tracer(tracer);
        return Ok((layer, TelemetryGuard { provider }));
    }

    tracing::info!(
        "Creating OpenTelemetry layer: endpoint={}, service={}",
        config.endpoint,
        config.service_name
    );

    // Configure OTLP exporter
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()?;

    // Build resource with service name
    let resource = opentelemetry_sdk::Resource::new(vec![
        KeyValue::new("service.name", config.service_name.clone()),
    ]);

    // Build tracer provider with batch processor
    let provider = TracerProvider::builder()
        .with_batch_exporter(otlp_exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();

    // Create tracer from provider using InstrumentationScope
    let scope = InstrumentationScope::builder(config.service_name.clone()).build();
    let tracer = provider.tracer_with_scope(scope);

    // Create OpenTelemetry tracing layer
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Ok((otel_layer, TelemetryGuard { provider }))
}

// ============================================================================
// No-op implementations when otel feature is disabled
// ============================================================================

/// Guard that does nothing when OTEL is disabled
#[cfg(not(feature = "otel"))]
pub struct TelemetryGuard;

#[cfg(not(feature = "otel"))]
impl TelemetryGuard {
    /// Create a no-op guard
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "otel"))]
impl Default for TelemetryGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// No-op configuration when OTEL is disabled
#[cfg(not(feature = "otel"))]
#[derive(Debug, Clone, Default)]
pub struct TelemetryConfig;

/// No-op initialization when OTEL feature is disabled.
/// Returns immediately without setting up any tracing infrastructure.
#[cfg(not(feature = "otel"))]
pub fn init_telemetry() -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!("OpenTelemetry tracing is not enabled (compile without 'otel' feature)");
    Ok(TelemetryGuard)
}

/// No-op initialization with config when OTEL feature is disabled.
#[cfg(not(feature = "otel"))]
pub fn init_telemetry_with_config(
    _config: TelemetryConfig,
) -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    init_telemetry()
}

// ============================================================================
// Helper macros for instrumented operations
// ============================================================================

/// Create a span for request receive phase
#[macro_export]
macro_rules! span_request_receive {
    ($request_id:expr, $provider:expr, $path:expr) => {
        tracing::info_span!(
            "request_receive",
            request_id = %$request_id,
            provider = %$provider,
            path = %$path,
            otel.kind = "server"
        )
    };
}

/// Create a span for account selection phase
#[macro_export]
macro_rules! span_account_selection {
    ($request_type:expr, $attempt:expr) => {
        tracing::info_span!(
            "account_selection",
            request_type = %$request_type,
            attempt = %$attempt,
            otel.kind = "internal"
        )
    };
}

/// Create a span for upstream API call
#[macro_export]
macro_rules! span_upstream_call {
    ($provider:expr, $model:expr, $account_id:expr, $method:expr) => {
        tracing::info_span!(
            "upstream_call",
            provider = %$provider,
            model = %$model,
            account_id = %$account_id,
            method = %$method,
            otel.kind = "client"
        )
    };
}

/// Create a span for response transformation
#[macro_export]
macro_rules! span_response_transform {
    ($provider:expr, $model:expr) => {
        tracing::info_span!(
            "response_transform",
            provider = %$provider,
            model = %$model,
            otel.kind = "internal"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_guard_creation() {
        // Should not panic
        let _guard = TelemetryGuard::default();
    }

    #[test]
    fn test_telemetry_config_default() {
        let config = TelemetryConfig::default();
        #[cfg(feature = "otel")]
        {
            assert!(!config.endpoint.is_empty());
            assert!(!config.service_name.is_empty());
        }
        let _ = config; // Suppress unused warning when otel is disabled
    }

    #[test]
    fn test_init_telemetry_without_feature() {
        // When otel feature is disabled, this should succeed immediately
        #[cfg(not(feature = "otel"))]
        {
            let result = init_telemetry();
            assert!(result.is_ok());
        }
    }
}
