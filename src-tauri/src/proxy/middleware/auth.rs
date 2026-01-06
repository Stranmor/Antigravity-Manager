// API Key 认证中间件
use axum::{
    extract::State,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::proxy::{ProxyAuthMode, ProxySecurityConfig};
use crate::proxy::middleware::RequestId;

/// API Key 认证中间件
pub async fn auth_middleware(
    State(security): State<Arc<RwLock<ProxySecurityConfig>>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    // Extract request ID from extensions (set by request_id_middleware)
    let request_id = request
        .extensions()
        .get::<RequestId>().map_or_else(|| "unknown".to_string(), |id| id.as_str().to_string());

    // 过滤心跳和健康检查请求,避免日志噪音
    if !path.contains("event_logging") && path != "/healthz" {
        tracing::info!(request_id = %request_id, "Request: {} {}", method, path);
    } else {
        tracing::trace!(request_id = %request_id, "Heartbeat: {} {}", method, path);
    }

    // Allow CORS preflight regardless of auth policy.
    if method == axum::http::Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    let security = security.read().await.clone();
    let effective_mode = security.effective_auth_mode();

    if matches!(effective_mode, ProxyAuthMode::Off) {
        return Ok(next.run(request).await);
    }

    if matches!(effective_mode, ProxyAuthMode::AllExceptHealth) && path == "/healthz" {
        return Ok(next.run(request).await);
    }

    // 从 header 中提取 API key
    let api_key = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or(Some(s)))
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|h| h.to_str().ok())
        });

    if security.api_key.is_empty() {
        tracing::error!(request_id = %request_id, "Proxy auth is enabled but api_key is empty; denying request");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Constant-time compare is unnecessary here, but keep strict equality and avoid leaking values.
    let authorized = api_key.is_some_and(|k| k == security.api_key);

    if authorized {
        Ok(next.run(request).await)
    } else {
        tracing::warn!(request_id = %request_id, "Unauthorized request attempt to {}", path);
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    // 移除未使用的 use super::*;

    #[test]
    fn test_auth_placeholder() {
        // Placeholder test - module exists and compiles
    }
}
