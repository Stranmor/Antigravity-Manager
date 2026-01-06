// Request ID Middleware - Unique request tracing with UUID
//
// This middleware:
// 1. Generates a unique UUID for each incoming request
// 2. Stores the ID in request extensions for handler access
// 3. Creates a tracing span with the request ID for log correlation
// 4. Adds X-Request-ID header to responses

use axum::{
    extract::Request,
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use tracing::{info_span, Instrument};
use uuid::Uuid;

/// Header name for the request ID in responses
pub const X_REQUEST_ID_HEADER: &str = "X-Request-ID";

/// Request ID extension for handlers to access the current request's ID
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

impl RequestId {
    /// Create a new request ID
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Get the request ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Request tracing middleware
///
/// Generates a unique request ID for each request and:
/// - Stores it in request extensions (accessible via `Extension<RequestId>`)
/// - Creates a tracing span with the ID for log correlation
/// - Adds X-Request-ID header to the response
///
/// # Usage in handlers
/// ```ignore
/// use axum::Extension;
/// use crate::proxy::middleware::request_id::RequestId;
///
/// async fn my_handler(Extension(request_id): Extension<RequestId>) -> impl IntoResponse {
///     tracing::info!(request_id = %request_id, "Processing request");
///     // ...
/// }
/// ```
pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    // Generate unique request ID
    let request_id = RequestId::new();
    let id_str = request_id.0.clone();

    // Store in request extensions for handler access
    request.extensions_mut().insert(request_id);

    // Create tracing span with request ID
    // Note: otel.kind = "server" marks this as an incoming server request for OTEL
    let span = info_span!(
        "request",
        request_id = %id_str,
        method = %request.method(),
        uri = %request.uri().path(),
        otel.kind = "server",
    );

    // Execute the request within the span
    let mut response = next.run(request).instrument(span).await;

    // Add X-Request-ID header to response
    if let Ok(header_value) = HeaderValue::from_str(&id_str) {
        response.headers_mut().insert(X_REQUEST_ID_HEADER, header_value);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_generation() {
        let id1 = RequestId::new();
        let id2 = RequestId::new();

        // Each request ID should be unique
        assert_ne!(id1.as_str(), id2.as_str());

        // Should be valid UUID format (36 chars with hyphens)
        assert_eq!(id1.as_str().len(), 36);
        assert_eq!(id2.as_str().len(), 36);

        // Verify UUID parsing works
        assert!(Uuid::parse_str(id1.as_str()).is_ok());
        assert!(Uuid::parse_str(id2.as_str()).is_ok());
    }

    #[test]
    fn test_request_id_display() {
        let id = RequestId::new();
        let display_str = format!("{id}");
        assert_eq!(display_str, id.as_str());
    }

    #[test]
    fn test_request_id_default() {
        let id1 = RequestId::default();
        let id2 = RequestId::default();

        // Default should also generate unique IDs
        assert_ne!(id1.as_str(), id2.as_str());
        assert_eq!(id1.as_str().len(), 36);
    }

    #[test]
    fn test_x_request_id_header_constant() {
        assert_eq!(X_REQUEST_ID_HEADER, "X-Request-ID");
    }
}
