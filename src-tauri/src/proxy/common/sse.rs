use axum::{
    body::Body,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::proxy::error::ProxyError;
use crate::proxy::middleware::monitor::X_RESOLVED_MODEL_HEADER;

pub fn build_sse_response(body: Body, resolved_model: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header(X_RESOLVED_MODEL_HEADER, resolved_model)
        .body(body)
        .unwrap_or_else(|e| {
            tracing::error!("Failed to build SSE response: {}", e);
            ProxyError::response_build_error(format!("SSE response build failed: {e}"))
                .into_response()
        })
}
