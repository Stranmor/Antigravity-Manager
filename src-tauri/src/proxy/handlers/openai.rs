// OpenAI Handler
//
// This module handles OpenAI API requests, transforming them to Gemini API format
// and managing account selection, rate limiting, and error handling.

use axum::{
    extract::Json, extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension
};
use base64::Engine as _;
use serde_json::{json, Value};
use tracing::{debug, error, info, info_span};

use crate::proxy::mappers::openai::{
    transform_openai_request, transform_openai_response, OpenAIRequest,
};
use crate::proxy::server::AppState;
use crate::proxy::handlers::common::WithResolvedModel;
use crate::proxy::handlers::helpers::{
    handle_overload_retry, is_overload_error, format_final_error,
    record_success, record_failure,
};
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::common::perf::{time_request_transform, time_response_transform, time_upstream_call};
use crate::proxy::common::retry::{
    apply_jitter, MAX_RETRY_ATTEMPTS,
};
use crate::proxy::common::sse::build_sse_response;
use crate::proxy::common::background_task;
use crate::proxy::session_manager::SessionManager;

// ===== Text Extraction Helper =====

fn extract_text_from_openai_request(request: &OpenAIRequest) -> Option<String> {
    use crate::proxy::mappers::openai::{OpenAIContent, OpenAIContentBlock};

    let mut text_parts = Vec::new();

    for message in &request.messages {
        if let Some(content) = &message.content {
            match content {
                OpenAIContent::String(s) => {
                    text_parts.push(s.clone());
                }
                OpenAIContent::Array(blocks) => {
                    for block in blocks {
                        if let OpenAIContentBlock::Text { text } = block {
                            text_parts.push(text.clone());
                        }
                    }
                }
            }
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

// ===== Model Routing =====

async fn resolve_openai_model(
    state: &AppState,
    request: &OpenAIRequest,
) -> (String, crate::proxy::mappers::common_utils::RequestConfig) {
    let mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
        &request.model,
        &*state.custom_mapping.read().await,
        &*state.openai_mapping.read().await,
        &*state.anthropic_mapping.read().await,
        false,  // OpenAI requests don't apply Claude family mapping
    );

    let tools_val: Option<Vec<Value>> = request.tools.clone();
    let config = crate::proxy::mappers::common_utils::resolve_request_config(
        &request.model,
        &mapped_model,
        &tools_val,
    );

    (mapped_model, config)
}

// ===== Streaming Response Handler =====

fn handle_openai_streaming(
    response: reqwest::Response,
    model: String,
    mapped_model: &str,
) -> Response {
    use crate::proxy::mappers::openai::streaming::create_openai_sse_stream;
    use axum::body::Body;

    let gemini_stream = response.bytes_stream();
    let openai_stream = create_openai_sse_stream(Box::pin(gemini_stream), model);
    let body = Body::from_stream(openai_stream);

    build_sse_response(body, mapped_model)
}

// ===== Non-Streaming Response Handler =====

async fn handle_openai_non_streaming(
    response: reqwest::Response,
    state: &AppState,
    request_id: &RequestId,
    trace_id: &str,
    account_id: &str,
    mapped_model: &str,
    openai_req: &OpenAIRequest,
    coalesce_sender: Option<crate::proxy::common::coalescing::CoalesceSender<Value>>,
) -> Response {
    let gemini_resp: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            if let Some(sender) = coalesce_sender {
                sender.fail();
            }
            return ProxyError::parse_error(format!("Parse error: {e}"))
                .with_request_id(request_id.clone())
                .into_response();
        }
    };

    let response_transform_span = info_span!(
        "response_transform",
        provider = "openai",
        model = %mapped_model,
        account_id = %account_id,
        otel.kind = "internal",
    );

    let _response_timer = time_response_transform("openai", mapped_model);
    let openai_response = {
        let _guard = response_transform_span.enter();
        let transform_start = std::time::Instant::now();
        let resp = transform_openai_response(&gemini_resp);
        let latency_ms = transform_start.elapsed().as_millis();
        info!(latency_ms = %latency_ms, "Response transformation completed");
        resp
    };

    // [COALESCING] Broadcast successful response
    if let Some(sender) = coalesce_sender {
        if let Ok(response_value) = serde_json::to_value(&openai_response) {
            sender.send(response_value);
        } else {
            sender.fail();
        }
    }

    // Sampled logging
    if state.sampler.should_sample() {
        use crate::proxy::common::sampling::SampledRequestBuilder;

        let request_json = serde_json::to_string(openai_req).unwrap_or_default();
        let (req_excerpt, req_truncated) = state.sampler.truncate_body(&request_json);

        let response_json = serde_json::to_string(&openai_response).unwrap_or_default();
        let (resp_excerpt, resp_truncated) = state.sampler.truncate_body(&response_json);

        let sampled = SampledRequestBuilder::new(trace_id, "POST", "/v1/chat/completions")
            .model(mapped_model)
            .account_id(account_id)
            .request_body(req_excerpt, req_truncated)
            .response_body(resp_excerpt, resp_truncated)
            .status_code(200)
            .build();

        state.sampler.log_sampled_request(&sampled);
    }

    Json(openai_response).with_resolved_model(mapped_model)
}

// ===== Rate Limit Handling =====

fn handle_openai_rate_limit(
    token_manager: &std::sync::Arc<crate::proxy::token_manager::TokenManager>,
    account_id: &str,
    email: &str,
    status_code: u16,
    retry_after: Option<&str>,
    error_text: &str,
    config: &crate::proxy::mappers::common_utils::RequestConfig,
) {
    token_manager.mark_rate_limited(email, status_code, retry_after, error_text);

    // Persist rate limit event to database
    if status_code == 429 {
        let account_id_owned = account_id.to_string();
        let quota_group = config.request_type.clone();
        let retry_after_secs = retry_after.and_then(|r| r.parse::<i32>().ok());
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
                tracing::warn!("Failed to persist rate limit event: {}", e);
            }
        });
    }
}

// ===== Main Chat Completions Handler =====

pub async fn handle_chat_completions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut openai_req: OpenAIRequest = match serde_json::from_value(body) {
        Ok(req) => req,
        Err(e) => return ProxyError::invalid_request(format!("Invalid request: {e}"))
            .with_request_id(request_id)
            .into_response(),
    };

    // [COALESCING] Try to deduplicate identical concurrent requests
    let coalesce_sender = if !openai_req.stream && state.coalescer.is_enabled() {
        let fingerprint = crate::proxy::common::coalescing::calculate_openai_fingerprint(&openai_req);
        match state.coalescer.get_or_create(fingerprint) {
            crate::proxy::common::coalescing::CoalesceResult::Primary(s) => Some(s),
            crate::proxy::common::coalescing::CoalesceResult::Coalesced(r) => {
                debug!("[{}] Identical OpenAI request in-flight, waiting for coalesced result...", request_id.as_str());
                match r.recv().await {
                    Ok(shared_response) => {
                        info!("[{}] Received coalesced result from primary OpenAI request", request_id.as_str());
                        return Json((*shared_response).clone()).into_response();
                    }
                    Err(e) => {
                        tracing::warn!("[{}] Coalesced OpenAI request failed ({e}), falling back to individual execution", request_id.as_str());
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    // Safety: Ensure messages is not empty
    if openai_req.messages.is_empty() {
        debug!("Received request with empty messages, injecting fallback...");
        openai_req.messages.push(crate::proxy::mappers::openai::OpenAIMessage {
            role: "user".to_string(),
            content: Some(crate::proxy::mappers::openai::OpenAIContent::String(" ".to_string())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    debug!("Received OpenAI request for model: {}", openai_req.model);

    // Background task detection and model downgrade
    let background_task_type = extract_text_from_openai_request(&openai_req)
        .and_then(|text| background_task::detect_from_text(&text));

    if let Some(task_type) = background_task_type {
        let downgraded_model = background_task::select_model(task_type);
        debug!(
            "[OpenAI] Background task detected: {:?}, downgrading model from '{}' to '{}'",
            task_type, openai_req.model, downgraded_model
        );
        openai_req.model = downgraded_model.to_string();
    }

    // Setup retry loop
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager.clone();
    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();
    let mut overload_retry_count: usize = 0;
    let trace_id = request_id.as_str();

    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }

        // Resolve model route and config
        let (mapped_model, config) = resolve_openai_model(&state, &openai_req).await;

        // [SCHEDULING] Classify request priority (observability only when scheduler enabled)
        if state.scheduler.is_enabled() {
            let x_priority = headers.get("x-priority").and_then(|h| h.to_str().ok());
            let priority = state.scheduler.classify_priority(x_priority, None, Some(&mapped_model));
            debug!("[{}] Request classified as priority: {}", trace_id, priority);
        }

        // Extract session ID for sticky scheduling
        let session_id = SessionManager::extract_openai_session_id(&openai_req);

        // Account selection span
        let account_selection_span = info_span!(
            "account_selection",
            request_type = %config.request_type,
            force_rotate = %(attempt > 0),
            attempt = %attempt,
            otel.kind = "internal",
        );

        let (access_token, project_id, email, account_id) = {
            let _guard = account_selection_span.enter();
            match token_manager.get_token(&config.request_type, attempt > 0, Some(&session_id)).await {
                Ok(t) => {
                    info!(account_id = %t.3, email = %t.2, "Account selected successfully");
                    t
                },
                Err(e) => {
                    error!(error = %e, "Account selection failed");
                    return ProxyError::token_error(format!("Token error: {e}"))
                        .with_request_id(request_id)
                        .into_response();
                }
            }
        };

        info!("Using account: {} (type: {})", email, config.request_type);

        // Circuit breaker check
        if let Err(retry_after) = state.circuit_breaker.should_allow(&account_id) {
            tracing::warn!(
                "[{}] Circuit breaker OPEN for account {} - skipping (retry in {:?})",
                trace_id, email, retry_after
            );
            attempt += 1;
            last_error = format!("Circuit breaker open for account {email}");
            continue;
        }

        // Transform request
        let _transform_timer = time_request_transform("openai", &mapped_model);
        let gemini_body = transform_openai_request(&openai_req, &project_id, &mapped_model);

        if let Ok(body_json) = serde_json::to_string_pretty(&gemini_body) {
            debug!("[OpenAI-Request] Transformed Gemini Body:\n{}", body_json);
        }

        // Upstream call
        let is_stream = openai_req.stream;
        let method = if is_stream { "streamGenerateContent" } else { "generateContent" };
        let query_string = if is_stream { Some("alt=sse") } else { None };

        let upstream_span = info_span!(
            "upstream_call",
            provider = "gemini",
            model = %mapped_model,
            account_id = %account_id,
            method = %method,
            stream = %is_stream,
            otel.kind = "client",
        );

        let _upstream_timer = time_upstream_call("openai", &mapped_model);
        let response = {
            let _guard = upstream_span.enter();
            let call_start = std::time::Instant::now();

            match upstream.call_v1_internal(method, &access_token, gemini_body, query_string, state.request_timeout).await {
                Ok(r) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    info!(latency_ms = %latency_ms, status = %r.status(), "Upstream call completed");
                    r
                },
                Err(e) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    error!(latency_ms = %latency_ms, error = %e, "Upstream call failed");
                    last_error.clone_from(&e);
                    debug!("OpenAI Request failed on attempt {}/{}: {}", attempt + 1, max_attempts, e);
                    continue;
                }
            }
        };

        let status = response.status();

        // Handle success
        if status.is_success() {
            record_success(&state, &account_id);

            if is_stream {
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return handle_openai_streaming(response, openai_req.model.clone(), &mapped_model);
            }

            return handle_openai_non_streaming(
                response,
                &state,
                &request_id,
                trace_id,
                &account_id,
                &mapped_model,
                &openai_req,
                coalesce_sender,
            ).await;
        }

        // Handle errors
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_else(|_| format!("HTTP {status_code}"));
        last_error = format!("HTTP {status_code}: {error_text}");

        tracing::error!("[OpenAI-Upstream] Error Response {}: {}", status_code, error_text);

        // Record failure in circuit breaker
        record_failure(&state, &account_id, status_code, &error_text);

        // Rate limit handling
        if status_code == 429 || status_code == 529 || status_code == 503 || status_code == 500 {
            handle_openai_rate_limit(
                &token_manager,
                &account_id,
                &email,
                status_code,
                retry_after.as_deref(),
                &error_text,
                &config,
            );

            // Handle overload errors
            if is_overload_error(status_code, &error_text)
                && handle_overload_retry(&mut overload_retry_count, trace_id, &email, "OpenAI").await
            {
                continue;
            }

            // Parse RetryInfo from Google Cloud
            if let Some(delay_ms) = crate::proxy::upstream::retry::parse_retry_delay(&error_text) {
                let actual_delay = delay_ms.saturating_add(200).min(10_000);
                let jittered_delay = apply_jitter(actual_delay);
                tracing::warn!(
                    "[{}] OpenAI Upstream {} on {} attempt {}/{}, waiting {}ms then retrying",
                    trace_id, status_code, email, attempt + 1, max_attempts, jittered_delay
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(jittered_delay)).await;
                attempt += 1;
                continue;
            }

            // Check for quota exhausted
            if error_text.contains("QUOTA_EXHAUSTED") {
                error!(
                    "[{}] OpenAI Quota exhausted (429) on account {} attempt {}/{}, stopping to protect pool.",
                    trace_id, email, attempt + 1, max_attempts
                );
                return ProxyError::RateLimited(error_text, Some(request_id)).into_response();
            }

            // Rotate account for other rate limit errors
            tracing::warn!(
                "[{}] OpenAI Upstream {} on {} attempt {}/{}, rotating account",
                trace_id, status_code, email, attempt + 1, max_attempts
            );
            attempt += 1;
            continue;
        }

        // Handle auth errors (403/401)
        if status_code == 403 || status_code == 401 {
            state.health_monitor.record_error(&account_id, status_code, &error_text).await;

            tracing::warn!(
                "[{}] OpenAI Upstream {} on account {} attempt {}/{}, rotating account",
                trace_id, status_code, email, attempt + 1, max_attempts
            );
            attempt += 1;
            continue;
        }

        // Non-retryable error
        error!(
            "[{}] OpenAI Upstream non-retryable error {} on account {}: {}",
            trace_id, status_code, email, error_text
        );
        if let Some(sender) = coalesce_sender {
            sender.fail();
        }
        return ProxyError::upstream_error(status_code, error_text)
            .with_request_id(request_id)
            .into_response();
    }

    if let Some(sender) = coalesce_sender {
        sender.fail();
    }

    ProxyError::Overloaded(
        format_final_error(max_attempts, overload_retry_count, &last_error),
        Some(request_id),
    )
    .into_response()
}

// ===== Legacy Completions Handler =====
// Handles /v1/completions and /v1/responses (Codex-style) endpoints

pub async fn handle_completions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    info!("Received /v1/completions or /v1/responses payload: {:?}", body);

    let is_codex_style = body.get("input").is_some() && body.get("instructions").is_some();

    // Convert payload to messages format
    body = convert_completions_to_chat_format(body, is_codex_style);

    let mut openai_req: OpenAIRequest = match serde_json::from_value(body.clone()) {
        Ok(req) => req,
        Err(e) => return ProxyError::invalid_request(format!("Invalid request: {e}"))
            .with_request_id(request_id)
            .into_response(),
    };

    // [COALESCING] Try to deduplicate identical concurrent requests
    let coalesce_sender = if !openai_req.stream && state.coalescer.is_enabled() {
        let fingerprint = crate::proxy::common::coalescing::calculate_openai_fingerprint(&openai_req);
        match state.coalescer.get_or_create(fingerprint) {
            crate::proxy::common::coalescing::CoalesceResult::Primary(s) => Some(s),
            crate::proxy::common::coalescing::CoalesceResult::Coalesced(r) => {
                debug!("[{}] Identical legacy completion request in-flight, waiting for coalesced result...", request_id.as_str());
                match r.recv().await {
                    Ok(shared_response) => {
                        info!("[{}] Received coalesced result from primary legacy completion request", request_id.as_str());
                        return Json((*shared_response).clone()).into_response();
                    }
                    Err(e) => {
                        tracing::warn!("[{}] Coalesced legacy completion request failed ({e}), falling back to individual execution", request_id.as_str());
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    // Safety: Inject empty message if needed
    if openai_req.messages.is_empty() {
        openai_req.messages.push(crate::proxy::mappers::openai::OpenAIMessage {
            role: "user".to_string(),
            content: Some(crate::proxy::mappers::openai::OpenAIContent::String(" ".to_string())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    // Setup retry loop
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager.clone();
    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();
    let mut overload_retry_count: usize = 0;
    let trace_id = request_id.as_str();

    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }

        let (mapped_model, config) = resolve_openai_model(&state, &openai_req).await;

        // [SCHEDULING] Classify request priority
        if state.scheduler.is_enabled() {
            let x_priority = headers.get("x-priority").and_then(|h| h.to_str().ok());
            let priority = state.scheduler.classify_priority(x_priority, None, Some(&mapped_model));
            debug!("[{}] Request classified as priority: {}", trace_id, priority);
        }

        let account_selection_span = info_span!(
            "account_selection",
            request_type = %config.request_type,
            attempt = %attempt,
            otel.kind = "internal",
        );

        let (access_token, project_id, email, account_id) = {
            let _guard = account_selection_span.enter();
            match token_manager.get_token(&config.request_type, false, None).await {
                Ok(t) => {
                    info!(account_id = %t.3, email = %t.2, "Account selected successfully");
                    t
                },
                Err(e) => {
                    error!(error = %e, "Account selection failed");
                    if let Some(sender) = coalesce_sender {
                        sender.fail();
                    }
                    return ProxyError::token_error(format!("Token error: {e}"))
                        .with_request_id(request_id)
                        .into_response();
                }
            }
        };

        info!("Using account: {} (type: {})", email, config.request_type);

        let gemini_body = transform_openai_request(&openai_req, &project_id, &mapped_model);

        if let Ok(body_json) = serde_json::to_string_pretty(&gemini_body) {
            debug!("[Codex-Request] Transformed Gemini Body:\n{}", body_json);
        }

        let is_stream = openai_req.stream;
        let method = if is_stream { "streamGenerateContent" } else { "generateContent" };
        let query_string = if is_stream { Some("alt=sse") } else { None };

        let upstream_span = info_span!(
            "upstream_call",
            provider = "gemini",
            model = %mapped_model,
            account_id = %account_id,
            method = %method,
            stream = %is_stream,
            otel.kind = "client",
        );

        let response = {
            let _guard = upstream_span.enter();
            let call_start = std::time::Instant::now();

            match upstream.call_v1_internal(method, &access_token, gemini_body, query_string, state.request_timeout).await {
                Ok(r) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    info!(latency_ms = %latency_ms, status = %r.status(), "Upstream call completed");
                    r
                },
                Err(e) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    error!(latency_ms = %latency_ms, error = %e, "Upstream call failed");
                    last_error.clone_from(&e);
                    continue;
                }
            }
        };

        let status = response.status();

        if status.is_success() {
            if is_stream {
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return handle_legacy_streaming(response, openai_req.model.clone(), &mapped_model, is_codex_style);
            }

            return handle_legacy_non_streaming(
                response,
                &state,
                &request_id,
                trace_id,
                &account_id,
                &mapped_model,
                coalesce_sender,
            ).await;
        }

        // Handle errors
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_default();
        last_error = format!("HTTP {status_code}: {error_text}");

        // Handle overload
        if status_code == 529 || status_code == 503 || status_code == 500 {
            token_manager.mark_rate_limited(&email, status_code, retry_after.as_deref(), &error_text);

            if is_overload_error(status_code, &error_text)
                && handle_overload_retry(&mut overload_retry_count, trace_id, &email, "Codex").await
            {
                continue;
            }
        }

        if status_code == 429 || status_code == 403 || status_code == 401 {
            handle_openai_rate_limit(
                &token_manager,
                &account_id,
                &email,
                status_code,
                retry_after.as_deref(),
                &error_text,
                &config,
            );
            attempt += 1;
            continue;
        }

        if let Some(sender) = coalesce_sender {
            sender.fail();
        }
        return ProxyError::upstream_error(status_code, error_text)
            .with_request_id(request_id)
            .into_response();
    }

    if let Some(sender) = coalesce_sender {
        sender.fail();
    }

    ProxyError::Overloaded(
        format_final_error(max_attempts, overload_retry_count, &last_error),
        Some(request_id),
    )
    .into_response()
}

// ===== Legacy Streaming Handler =====

fn handle_legacy_streaming(
    response: reqwest::Response,
    model: String,
    mapped_model: &str,
    is_codex_style: bool,
) -> Response {
    use axum::body::Body;

    let gemini_stream = response.bytes_stream();
    let body = if is_codex_style {
        use crate::proxy::mappers::openai::streaming::create_codex_sse_stream;
        let s = create_codex_sse_stream(Box::pin(gemini_stream), model);
        Body::from_stream(s)
    } else {
        use crate::proxy::mappers::openai::streaming::create_legacy_sse_stream;
        let s = create_legacy_sse_stream(Box::pin(gemini_stream), model);
        Body::from_stream(s)
    };

    build_sse_response(body, mapped_model)
}

// ===== Legacy Non-Streaming Handler =====

async fn handle_legacy_non_streaming(
    response: reqwest::Response,
    state: &AppState,
    request_id: &RequestId,
    trace_id: &str,
    account_id: &str,
    mapped_model: &str,
    coalesce_sender: Option<crate::proxy::common::coalescing::CoalesceSender<Value>>,
) -> Response {
    let gemini_resp: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            if let Some(sender) = coalesce_sender {
                sender.fail();
            }
            return ProxyError::parse_error(format!("Parse error: {e}"))
                .with_request_id(request_id.clone())
                .into_response();
        }
    };

    let response_transform_span = info_span!(
        "response_transform",
        provider = "openai-legacy",
        model = %mapped_model,
        account_id = %account_id,
        otel.kind = "internal",
    );

    let chat_resp = {
        let _guard = response_transform_span.enter();
        let transform_start = std::time::Instant::now();
        let resp = transform_openai_response(&gemini_resp);
        let latency_ms = transform_start.elapsed().as_millis();
        info!(latency_ms = %latency_ms, "Response transformation completed");
        resp
    };

    // Map Chat Response -> Legacy Completions Response
    let choices = chat_resp.choices.iter().map(|c| {
        json!({
            "text": match &c.message.content {
                Some(crate::proxy::mappers::openai::OpenAIContent::String(s)) => s.clone(),
                _ => String::new()
            },
            "index": c.index,
            "logprobs": null,
            "finish_reason": c.finish_reason
        })
    }).collect::<Vec<_>>();

    let legacy_resp = json!({
        "id": chat_resp.id,
        "object": "text_completion",
        "created": chat_resp.created,
        "model": chat_resp.model,
        "choices": choices
    });

    // [COALESCING] Broadcast successful response
    if let Some(sender) = coalesce_sender {
        sender.send(legacy_resp.clone());
    }

    // Sampled logging
    if state.sampler.should_sample() {
        use crate::proxy::common::sampling::SampledRequestBuilder;

        let request_json = format!("{{\"model\": \"{mapped_model}\"}}");
        let (req_excerpt, req_truncated) = state.sampler.truncate_body(&request_json);

        let response_json = serde_json::to_string(&legacy_resp).unwrap_or_default();
        let (resp_excerpt, resp_truncated) = state.sampler.truncate_body(&response_json);

        let sampled = SampledRequestBuilder::new(trace_id, "POST", "/v1/completions")
            .model(mapped_model)
            .account_id(account_id)
            .request_body(req_excerpt, req_truncated)
            .response_body(resp_excerpt, resp_truncated)
            .status_code(200)
            .build();

        state.sampler.log_sampled_request(&sampled);
    }

    axum::Json(legacy_resp).with_resolved_model(mapped_model)
}

// ===== Completions Format Conversion =====

fn convert_completions_to_chat_format(mut body: Value, is_codex_style: bool) -> Value {
    if is_codex_style {
        body = convert_codex_to_chat_format(body);
    } else if let Some(prompt_val) = body.get("prompt") {
        // Legacy OpenAI Style: prompt -> Chat
        let prompt_str = match prompt_val {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => prompt_val.to_string(),
        };
        let messages = json!([ { "role": "user", "content": prompt_str } ]);
        if let Some(obj) = body.as_object_mut() {
            obj.remove("prompt");
            obj.insert("messages".to_string(), messages);
        }
    }
    body
}

fn convert_codex_to_chat_format(mut body: Value) -> Value {
    let instructions = body
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let input_items = body.get("input").and_then(|v| v.as_array());

    let mut messages = Vec::new();

    // System Instructions
    if !instructions.is_empty() {
        messages.push(json!({ "role": "system", "content": instructions }));
    }

    let mut call_id_to_name = std::collections::HashMap::new();

    // Pass 1: Build Call ID to Name Map
    if let Some(items) = input_items {
        for item in items {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "function_call" | "local_shell_call" | "web_search_call" => {
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("id").and_then(|v| v.as_str()))
                        .unwrap_or("unknown");

                    let name = if item_type == "local_shell_call" {
                        "shell"
                    } else if item_type == "web_search_call" {
                        "google_search"
                    } else {
                        item.get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                    };

                    call_id_to_name.insert(call_id.to_string(), name.to_string());
                    tracing::debug!("Mapped call_id {} to name {}", call_id, name);
                }
                _ => {}
            }
        }
    }

    // Pass 2: Map Input Items to Messages
    if let Some(items) = input_items {
        for item in items {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    messages.push(convert_codex_message_item(item));
                }
                "function_call" | "local_shell_call" | "web_search_call" => {
                    messages.push(convert_codex_function_call_item(item, item_type));
                }
                "function_call_output" | "custom_tool_call_output" => {
                    messages.push(convert_codex_function_output_item(item, &call_id_to_name));
                }
                _ => {}
            }
        }
    }

    if let Some(obj) = body.as_object_mut() {
        obj.insert("messages".to_string(), json!(messages));
    }
    body
}

fn convert_codex_message_item(item: &Value) -> Value {
    let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    let content = item.get("content").and_then(|v| v.as_array());
    let mut text_parts = Vec::new();
    let mut image_parts: Vec<Value> = Vec::new();

    if let Some(parts) = content {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                text_parts.push(text.to_string());
            } else if part.get("type").and_then(|v| v.as_str()) == Some("input_image") {
                if let Some(image_url) = part.get("image_url").and_then(|v| v.as_str()) {
                    image_parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": image_url }
                    }));
                    debug!("[Codex] Found input_image: {}", image_url);
                }
            } else if part.get("type").and_then(|v| v.as_str()) == Some("image_url") {
                if let Some(url_obj) = part.get("image_url") {
                    image_parts.push(json!({
                        "type": "image_url",
                        "image_url": url_obj.clone()
                    }));
                }
            }
        }
    }

    if image_parts.is_empty() {
        json!({
            "role": role,
            "content": text_parts.join("\n")
        })
    } else {
        let mut content_blocks: Vec<Value> = Vec::new();
        if !text_parts.is_empty() {
            content_blocks.push(json!({
                "type": "text",
                "text": text_parts.join("\n")
            }));
        }
        content_blocks.extend(image_parts);
        json!({
            "role": role,
            "content": content_blocks
        })
    }
}

fn convert_codex_function_call_item(item: &Value, item_type: &str) -> Value {
    let mut name = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let mut args_str = item
        .get("arguments")
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();
    let call_id = item
        .get("call_id")
        .and_then(|v| v.as_str())
        .or_else(|| item.get("id").and_then(|v| v.as_str()))
        .unwrap_or("unknown");

    if item_type == "local_shell_call" {
        name = "shell";
        if let Some(action) = item.get("action") {
            if let Some(exec) = action.get("exec") {
                let mut args_obj = serde_json::Map::new();
                if let Some(cmd) = exec.get("command") {
                    let cmd_val = if cmd.is_string() {
                        json!([cmd])
                    } else {
                        cmd.clone()
                    };
                    args_obj.insert("command".to_string(), cmd_val);
                }
                if let Some(wd) = exec.get("working_directory").or(exec.get("workdir")) {
                    args_obj.insert("workdir".to_string(), wd.clone());
                }
                args_str = serde_json::to_string(&args_obj).unwrap_or("{}".to_string());
            }
        }
    } else if item_type == "web_search_call" {
        name = "google_search";
        if let Some(action) = item.get("action") {
            let mut args_obj = serde_json::Map::new();
            if let Some(q) = action.get("query") {
                args_obj.insert("query".to_string(), q.clone());
            }
            args_str = serde_json::to_string(&args_obj).unwrap_or("{}".to_string());
        }
    }

    json!({
        "role": "assistant",
        "tool_calls": [
            {
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args_str
                }
            }
        ]
    })
}

fn convert_codex_function_output_item(item: &Value, call_id_to_name: &std::collections::HashMap<String, String>) -> Value {
    let call_id = item
        .get("call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let output = item.get("output");
    let output_str = if let Some(o) = output {
        if o.is_string() {
            o.as_str().unwrap_or_default().to_string()
        } else if let Some(content) = o.get("content").and_then(|v| v.as_str()) {
            content.to_string()
        } else {
            o.to_string()
        }
    } else {
        String::new()
    };

    let name = call_id_to_name.get(call_id).cloned().unwrap_or_else(|| {
        tracing::warn!("Unknown tool name for call_id {}, defaulting to 'shell'", call_id);
        "shell".to_string()
    });

    json!({
        "role": "tool",
        "tool_call_id": call_id,
        "name": name,
        "content": output_str
    })
}

// ===== List Models =====

pub async fn handle_list_models(State(state): State<AppState>) -> impl IntoResponse {
    use crate::proxy::common::model_mapping::get_all_dynamic_models;

    let model_ids = get_all_dynamic_models(
        &state.openai_mapping,
        &state.custom_mapping,
        &state.anthropic_mapping,
    ).await;

    let data: Vec<_> = model_ids.into_iter().map(|id| {
        json!({
            "id": id,
            "object": "model",
            "created": 1706745600,
            "owned_by": "antigravity"
        })
    }).collect();

    Json(json!({
        "object": "list",
        "data": data
    }))
}

// ===== Image Generation =====

pub async fn handle_images_generations(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    let Some(prompt) = body.get("prompt").and_then(|v| v.as_str()) else {
        return ProxyError::invalid_request("Missing 'prompt' field")
            .with_request_id(request_id)
            .into_response();
    };

    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("gemini-3-pro-image");
    let n = body.get("n").and_then(serde_json::Value::as_u64).unwrap_or(1) as usize;
    let size = body.get("size").and_then(|v| v.as_str()).unwrap_or("1024x1024");
    let response_format = body.get("response_format").and_then(|v| v.as_str()).unwrap_or("b64_json");
    let quality = body.get("quality").and_then(|v| v.as_str()).unwrap_or("standard");
    let style = body.get("style").and_then(|v| v.as_str()).unwrap_or("vivid");

    info!(
        "[Images] Received request: model={}, prompt={:.50}..., n={}, size={}, quality={}, style={}",
        model, prompt, n, size, quality, style
    );

    let aspect_ratio = match size {
        "1792x768" | "2560x1080" => "21:9",
        "1792x1024" | "1920x1080" => "16:9",
        "1024x1792" | "1080x1920" => "9:16",
        "1024x768" | "1280x960" => "4:3",
        "768x1024" | "960x1280" => "3:4",
        _ => "1:1",
    };

    let mut final_prompt = prompt.to_string();
    if quality == "hd" {
        final_prompt.push_str(", (high quality, highly detailed, 4k resolution, hdr)");
    }
    match style {
        "vivid" => final_prompt.push_str(", (vivid colors, dramatic lighting, rich details)"),
        "natural" => final_prompt.push_str(", (natural lighting, realistic, photorealistic)"),
        _ => {}
    }

    let upstream = state.upstream.clone();
    let token_manager = state.token_manager.clone();

    let (access_token, project_id, email, _account_id) = match token_manager.get_token("image_gen", false, None).await {
        Ok(t) => t,
        Err(e) => {
            return ProxyError::token_error(format!("Token error: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    info!("Using account: {} for image generation", email);

    let mut tasks = Vec::new();
    for _ in 0..n {
        let upstream = upstream.clone();
        let access_token = access_token.clone();
        let project_id = project_id.clone();
        let final_prompt = final_prompt.clone();
        let aspect_ratio = aspect_ratio.to_string();
        let request_timeout = state.request_timeout;

        tasks.push(tokio::spawn(async move {
            let gemini_body = json!({
                "project": project_id,
                "requestId": format!("img-{}", uuid::Uuid::new_v4()),
                "model": "gemini-3-pro-image",
                "userAgent": "antigravity",
                "requestType": "image_gen",
                "request": {
                    "contents": [{"role": "user", "parts": [{"text": final_prompt}]}],
                    "generationConfig": {
                        "candidateCount": 1,
                        "imageConfig": {"aspectRatio": aspect_ratio}
                    },
                    "safetySettings": [
                        { "category": "HARM_CATEGORY_HARASSMENT", "threshold": "OFF" },
                        { "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "OFF" },
                        { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "OFF" },
                        { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "OFF" },
                        { "category": "HARM_CATEGORY_CIVIC_INTEGRITY", "threshold": "OFF" },
                    ]
                }
            });

            match upstream.call_v1_internal("generateContent", &access_token, gemini_body, None, request_timeout).await {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let err_text = response.text().await.unwrap_or_default();
                        return Err(format!("Upstream error {status}: {err_text}"));
                    }
                    match response.json::<Value>().await {
                        Ok(json) => Ok(json),
                        Err(e) => Err(format!("Parse error: {e}")),
                    }
                }
                Err(e) => Err(format!("Network error: {e}")),
            }
        }));
    }

    let (images, errors) = collect_image_results(tasks, response_format).await;

    if images.is_empty() {
        let error_msg = if errors.is_empty() { "No images generated".to_string() } else { errors.join("; ") };
        tracing::error!("[Images] All {} requests failed. Errors: {}", n, error_msg);
        return ProxyError::UpstreamError { status: 502, message: error_msg, request_id: Some(request_id) }.into_response();
    }

    if !errors.is_empty() {
        tracing::warn!("[Images] Partial success: {} out of {} requests succeeded. Errors: {}", images.len(), n, errors.join("; "));
    }

    tracing::info!("[Images] Successfully generated {} out of {} requested image(s)", images.len(), n);

    Json(json!({
        "created": time::OffsetDateTime::now_utc().unix_timestamp(),
        "data": images
    })).into_response()
}

// ===== Image Edits =====

pub async fn handle_images_edits(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    tracing::info!("[Images] Received edit request");

    let mut image_data = None;
    let mut mask_data = None;
    let mut prompt = String::new();
    let mut n = 1;
    let mut size = "1024x1024".to_string();
    let mut response_format = "b64_json".to_string();
    let mut model = "gemini-3-pro-image".to_string();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return ProxyError::invalid_request(format!("Multipart error: {e}"))
                .with_request_id(request_id)
                .into_response(),
        };
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "image" => {
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(e) => return ProxyError::invalid_request(format!("Image read error: {e}"))
                        .with_request_id(request_id)
                        .into_response(),
                };
                image_data = Some(base64::engine::general_purpose::STANDARD.encode(data));
            }
            "mask" => {
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(e) => return ProxyError::invalid_request(format!("Mask read error: {e}"))
                        .with_request_id(request_id)
                        .into_response(),
                };
                mask_data = Some(base64::engine::general_purpose::STANDARD.encode(data));
            }
            "prompt" => {
                prompt = match field.text().await {
                    Ok(t) => t,
                    Err(e) => return ProxyError::invalid_request(format!("Prompt read error: {e}"))
                        .with_request_id(request_id)
                        .into_response(),
                };
            }
            "n" => { if let Ok(val) = field.text().await { n = val.parse().unwrap_or(1); } }
            "size" => { if let Ok(val) = field.text().await { size = val; } }
            "response_format" => { if let Ok(val) = field.text().await { response_format = val; } }
            "model" => { if let Ok(val) = field.text().await { if !val.is_empty() { model = val; } } }
            _ => {}
        }
    }

    if image_data.is_none() {
        return ProxyError::invalid_request("Missing image").with_request_id(request_id).into_response();
    }
    if prompt.is_empty() {
        return ProxyError::invalid_request("Missing prompt").with_request_id(request_id).into_response();
    }

    tracing::info!(
        "[Images] Edit Request: model={}, prompt={}, n={}, size={}, mask={}, response_format={}",
        model, prompt, n, size, mask_data.is_some(), response_format
    );

    let upstream = state.upstream.clone();
    let token_manager = state.token_manager.clone();

    let (access_token, project_id, _email, _account_id) = match token_manager.get_token("image_gen", false, None).await {
        Ok(t) => t,
        Err(e) => {
            return ProxyError::token_error(format!("Token error: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    let mut contents_parts = Vec::new();
    contents_parts.push(json!({"text": format!("Edit this image: {}", prompt)}));

    if let Some(data) = image_data {
        contents_parts.push(json!({"inlineData": {"mimeType": "image/png", "data": data}}));
    }

    if let Some(data) = mask_data {
        contents_parts.push(json!({"inlineData": {"mimeType": "image/png", "data": data}}));
    }

    let gemini_body = json!({
        "project": project_id,
        "requestId": format!("img-edit-{}", uuid::Uuid::new_v4()),
        "model": model,
        "userAgent": "antigravity",
        "requestType": "image_gen",
        "request": {
            "contents": [{"role": "user", "parts": contents_parts}],
            "generationConfig": {
                "candidateCount": 1,
                "maxOutputTokens": 8192,
                "stopSequences": [],
                "temperature": 1.0,
                "topP": 0.95,
                "topK": 40
            },
            "safetySettings": [
                { "category": "HARM_CATEGORY_HARASSMENT", "threshold": "OFF" },
                { "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "OFF" },
                { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "OFF" },
                { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "OFF" },
                { "category": "HARM_CATEGORY_CIVIC_INTEGRITY", "threshold": "OFF" },
            ]
        }
    });

    let mut tasks = Vec::new();
    for _ in 0..n {
        let upstream = upstream.clone();
        let access_token = access_token.clone();
        let body = gemini_body.clone();
        let request_timeout = state.request_timeout;

        tasks.push(tokio::spawn(async move {
            match upstream.call_v1_internal("generateContent", &access_token, body, None, request_timeout).await {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let err_text = response.text().await.unwrap_or_default();
                        return Err(format!("Upstream error {status}: {err_text}"));
                    }
                    match response.json::<Value>().await {
                        Ok(json) => Ok(json),
                        Err(e) => Err(format!("Parse error: {e}")),
                    }
                }
                Err(e) => Err(format!("Network error: {e}")),
            }
        }));
    }

    let (images, errors) = collect_image_results(tasks, &response_format).await;

    if images.is_empty() {
        let error_msg = if errors.is_empty() { "No images generated".to_string() } else { errors.join("; ") };
        tracing::error!("[Images] All {} edit requests failed. Errors: {}", n, error_msg);
        return ProxyError::UpstreamError { status: 502, message: error_msg, request_id: Some(request_id) }.into_response();
    }

    if !errors.is_empty() {
        tracing::warn!("[Images] Partial success: {} out of {} requests succeeded. Errors: {}", images.len(), n, errors.join("; "));
    }

    tracing::info!("[Images] Successfully generated {} out of {} requested edited image(s)", images.len(), n);

    Json(json!({
        "created": time::OffsetDateTime::now_utc().unix_timestamp(),
        "data": images
    })).into_response()
}

// ===== Image Result Collection Helper =====

async fn collect_image_results(
    tasks: Vec<tokio::task::JoinHandle<Result<Value, String>>>,
    response_format: &str,
) -> (Vec<Value>, Vec<String>) {
    let mut images: Vec<Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (idx, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(result) => match result {
                Ok(gemini_resp) => {
                    let raw = gemini_resp.get("response").unwrap_or(&gemini_resp);
                    if let Some(parts) = raw
                        .get("candidates")
                        .and_then(|c| c.get(0))
                        .and_then(|cand| cand.get("content"))
                        .and_then(|content| content.get("parts"))
                        .and_then(|p| p.as_array())
                    {
                        for part in parts {
                            if let Some(img) = part.get("inlineData") {
                                let data = img.get("data").and_then(|v| v.as_str()).unwrap_or("");
                                if !data.is_empty() {
                                    if response_format == "url" {
                                        let mime_type = img.get("mimeType").and_then(|v| v.as_str()).unwrap_or("image/png");
                                        images.push(json!({"url": format!("data:{};base64,{}", mime_type, data)}));
                                    } else {
                                        images.push(json!({"b64_json": data}));
                                    }
                                    tracing::debug!("[Images] Task {} succeeded", idx);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("[Images] Task {} failed: {}", idx, e);
                    errors.push(e);
                }
            },
            Err(e) => {
                let err_msg = format!("Task join error: {e}");
                tracing::error!("[Images] Task {} join error: {}", idx, e);
                errors.push(err_msg);
            }
        }
    }

    (images, errors)
}
