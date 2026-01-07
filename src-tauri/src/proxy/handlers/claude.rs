// Claude Protocol Handler
//
// This module handles Claude/Anthropic API requests, transforming them to Gemini API format
// and managing account selection, rate limiting, and error handling.

use axum::{
    body::Body,
    extract::{Json, State},
    response::{IntoResponse, Response},
    Extension,
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};
use tracing::{debug, error, info, info_span};

use crate::proxy::mappers::claude::{
    transform_claude_request_in, transform_response, create_claude_sse_stream, ClaudeRequest,
};
use crate::proxy::server::AppState;
use crate::proxy::handlers::common::WithResolvedModel;
use crate::proxy::handlers::helpers::{
    handle_overload_retry, handle_rate_limit_response, is_overload_error,
    format_final_error, record_failure, record_auth_error,
    record_success_with_probe, RateLimitContext,
};
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::common::perf::{time_request_transform, time_response_transform, time_upstream_call};
use crate::proxy::common::retry::{
    determine_retry_strategy, execute_retry_strategy, should_rotate_account,
    MAX_RETRY_ATTEMPTS,
};
use crate::proxy::common::coalescing::CoalesceSender;
use crate::proxy::common::sse::build_sse_response;
use crate::proxy::common::background_task::{self, BackgroundTaskType};
use axum::http::HeaderMap;
use std::sync::atomic::Ordering;

const MIN_SIGNATURE_LENGTH: usize = 10;

// ===== Thinking Block Processing Helpers =====

use crate::proxy::mappers::claude::models::{ContentBlock, Message, MessageContent};

/// Check if a thinking block has a valid signature
fn has_valid_signature(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Thinking { signature, thinking, .. } => {
            // Empty thinking + any signature = valid (trailing signature case)
            if thinking.is_empty() && signature.is_some() {
                return true;
            }
            // Has content + long enough signature = valid
            signature.as_ref().is_some_and(|s| s.len() >= MIN_SIGNATURE_LENGTH)
        }
        _ => true  // Non-thinking blocks are always valid
    }
}

/// Sanitize thinking block, keeping only necessary fields (removing cache_control etc.)
fn sanitize_thinking_block(block: ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::Thinking { thinking, signature, .. } => {
            ContentBlock::Thinking {
                thinking,
                signature,
                cache_control: None,
            }
        }
        _ => block
    }
}

/// Filter invalid thinking blocks from messages
fn filter_invalid_thinking_blocks(messages: &mut [Message]) {
    let mut total_filtered = 0;

    for msg in messages.iter_mut() {
        // Only process assistant/model messages
        if msg.role != "assistant" && msg.role != "model" {
            continue;
        }
        tracing::error!("[DEBUG-FILTER] Inspecting msg with role: {}", msg.role);

        if let MessageContent::Array(blocks) = &mut msg.content {
            let original_len = blocks.len();

            // Filter and sanitize
            let mut new_blocks = Vec::new();
            for block in blocks.drain(..) {
                if matches!(block, ContentBlock::Thinking { .. }) {
                    if let ContentBlock::Thinking { ref signature, .. } = block {
                         tracing::error!("[DEBUG-FILTER] Found thinking block. Sig len: {:?}", signature.as_ref().map(std::string::String::len));
                    }

                    if has_valid_signature(&block) {
                        new_blocks.push(sanitize_thinking_block(block));
                    } else {
                        // Convert invalid thinking blocks to text instead of dropping
                        if let ContentBlock::Thinking { thinking, .. } = &block {
                            if thinking.is_empty() {
                                tracing::debug!("[Claude-Handler] Dropping empty thinking block with invalid signature");
                            } else {
                                tracing::info!(
                                    "[Claude-Handler] Converting thinking block with invalid signature to text. \
                                     Content length: {} chars",
                                    thinking.len()
                                );
                                new_blocks.push(ContentBlock::Text { text: thinking.clone() });
                            }
                        }
                    }
                } else {
                    new_blocks.push(block);
                }
            }

            *blocks = new_blocks;
            let filtered_count = original_len - blocks.len();
            total_filtered += filtered_count;

            // Add empty text block if all blocks were filtered
            if blocks.is_empty() {
                blocks.push(ContentBlock::Text {
                    text: String::new()
                });
            }
        }
    }

    if total_filtered > 0 {
        debug!("Filtered {} invalid thinking block(s) from history", total_filtered);
    }
}

/// Remove trailing unsigned thinking blocks
fn remove_trailing_unsigned_thinking(blocks: &mut Vec<ContentBlock>) {
    if blocks.is_empty() {
        return;
    }

    // Scan from end
    let mut end_index = blocks.len();
    for i in (0..blocks.len()).rev() {
        match &blocks[i] {
            ContentBlock::Thinking { .. } => {
                if has_valid_signature(&blocks[i]) {
                    break;  // Found valid signed thinking block, stop
                }
                end_index = i;
            }
            _ => break  // Non-thinking block, stop
        }
    }

    if end_index < blocks.len() {
        let removed = blocks.len() - end_index;
        blocks.truncate(end_index);
        debug!("Removed {} trailing unsigned thinking block(s)", removed);
    }
}

// ===== Z.ai Dispatch Decision =====

/// Determines whether to use z.ai (Anthropic passthrough) or Google flow
async fn should_use_zai(state: &AppState) -> bool {
    let zai = state.zai.read().await.clone();
    let zai_enabled = zai.enabled && !matches!(zai.dispatch_mode, crate::proxy::ZaiDispatchMode::Off);
    let google_accounts = state.token_manager.len();

    if zai_enabled {
        match zai.dispatch_mode {
            crate::proxy::ZaiDispatchMode::Off => false,
            crate::proxy::ZaiDispatchMode::Exclusive => true,
            crate::proxy::ZaiDispatchMode::Fallback => google_accounts == 0,
            crate::proxy::ZaiDispatchMode::Pooled => {
                let total = google_accounts.saturating_add(1).max(1);
                let slot = state.provider_rr.fetch_add(1, Ordering::Relaxed) % total;
                slot == 0
            }
        }
    } else {
        false
    }
}

// ===== Message Content Extraction =====

/// Extracts the latest meaningful user message for logging and background task detection
fn extract_meaningful_message(request: &ClaudeRequest) -> String {
    let meaningful_msg = request.messages.iter().rev()
        .filter(|m| m.role == "user")
        .find_map(|m| {
            let content = match &m.content {
                MessageContent::String(s) => s.clone(),
                MessageContent::Array(arr) => {
                    arr.iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            };

            // Filter rules:
            // 1. Ignore empty messages
            // 2. Ignore "Warmup" messages
            // 3. Ignore <system-reminder> tagged messages
            if content.trim().is_empty()
                || content.starts_with("Warmup")
                || content.contains("<system-reminder>")
            {
                None
            } else {
                Some(content)
            }
        });

    // Fallback to last message if no meaningful message found
    meaningful_msg.unwrap_or_else(|| {
        request.messages.last().map_or_else(
            || "[No Messages]".to_string(),
            |m| match &m.content {
                MessageContent::String(s) => s.clone(),
                MessageContent::Array(_) => "[Complex/Tool Message]".to_string()
            },
        )
    })
}

// ===== Request Logging =====

/// Log request details at appropriate levels
fn log_request_details(trace_id: &str, request: &ClaudeRequest, latest_msg: &str) {
    // INFO level: Concise one-line summary
    info!(
        "[{}] Claude Request | Model: {} | Stream: {} | Messages: {} | Tools: {}",
        trace_id,
        request.model,
        request.stream,
        request.messages.len(),
        request.tools.is_some()
    );

    // DEBUG level: Detailed debugging info
    debug!("========== [{}] CLAUDE REQUEST DEBUG START ==========", trace_id);
    debug!("[{}] Model: {}", trace_id, request.model);
    debug!("[{}] Stream: {}", trace_id, request.stream);
    debug!("[{}] Max Tokens: {:?}", trace_id, request.max_tokens);
    debug!("[{}] Temperature: {:?}", trace_id, request.temperature);
    debug!("[{}] Message Count: {}", trace_id, request.messages.len());
    debug!("[{}] Has Tools: {}", trace_id, request.tools.is_some());
    debug!("[{}] Has Thinking Config: {}", trace_id, request.thinking.is_some());
    debug!("[{}] Content Preview: {:.100}...", trace_id, latest_msg);

    // Log each message detail
    for (idx, msg) in request.messages.iter().enumerate() {
        let content_preview = match &msg.content {
            MessageContent::String(s) => {
                if s.len() > 200 {
                    format!("{}... (total {} chars)", &s[..200], s.len())
                } else {
                    s.clone()
                }
            },
            MessageContent::Array(arr) => {
                format!("[Array with {} blocks]", arr.len())
            }
        };
        debug!("[{}] Message[{}] - Role: {}, Content: {}",
            trace_id, idx, msg.role, content_preview);
    }

    debug!("[{}] Full Claude Request JSON: {}", trace_id, serde_json::to_string_pretty(request).unwrap_or_default());
    debug!("========== [{}] CLAUDE REQUEST DEBUG END ==========", trace_id);
}

// ===== Model Routing =====

/// Resolves model route and request configuration
async fn resolve_model_config(
    state: &AppState,
    request: &ClaudeRequest,
) -> (String, crate::proxy::mappers::common_utils::RequestConfig) {
    // Initial model resolution without family mapping
    let initial_mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
        &request.model,
        &*state.custom_mapping.read().await,
        &*state.openai_mapping.read().await,
        &*state.anthropic_mapping.read().await,
        false,
    );

    // Convert tools to Value array for web search detection
    let tools_val: Option<Vec<Value>> = request.tools.as_ref().map(|list| {
        list.iter().map(|t| serde_json::to_value(t).unwrap_or(json!({}))).collect()
    });

    let config = crate::proxy::mappers::common_utils::resolve_request_config(
        &request.model,
        &initial_mapped_model,
        &tools_val
    );

    // Apply Claude family mapping for CLI requests (agent type)
    let is_cli_request = config.request_type == "agent";

    let mapped_model = if is_cli_request {
        crate::proxy::common::model_mapping::resolve_model_route(
            &request.model,
            &*state.custom_mapping.read().await,
            &*state.openai_mapping.read().await,
            &*state.anthropic_mapping.read().await,
            true,  // Apply family mapping for CLI requests
        )
    } else {
        initial_mapped_model
    };

    (mapped_model, config)
}

// ===== Background Task Handling =====

/// Prepares request for background task (model downgrade, tool removal, etc.)
fn prepare_background_task_request(
    request: &mut ClaudeRequest,
    mapped_model: &mut String,
    task_type: BackgroundTaskType,
    trace_id: &str,
) {
    let downgrade_model = background_task::select_model(task_type);

    info!(
        "[{}][AUTO] Background task detected (type: {:?}), downgrading: {} -> {}",
        trace_id,
        task_type,
        mapped_model,
        downgrade_model
    );

    // Override model
    *mapped_model = downgrade_model.to_string();

    // Background task cleanup:
    // 1. Remove tools (not needed for background tasks)
    request.tools = None;

    // 2. Remove Thinking config (Flash models don't support it)
    request.thinking = None;

    // 3. Clear Thinking blocks from history
    for msg in &mut request.messages {
        if let MessageContent::Array(blocks) = &mut msg.content {
            blocks.retain(|b| !matches!(b,
                ContentBlock::Thinking { .. } |
                ContentBlock::RedactedThinking { .. }
            ));
        }
    }
}

/// Prepares request for real user interaction
fn prepare_user_request(request: &mut ClaudeRequest, trace_id: &str, mapped_model: &str) {
    debug!(
        "[{}][USER] User interaction request, keeping mapping: {}",
        trace_id,
        mapped_model
    );

    // Clean trailing unsigned thinking blocks
    for msg in &mut request.messages {
        if msg.role == "assistant" || msg.role == "model" {
            if let MessageContent::Array(blocks) = &mut msg.content {
                remove_trailing_unsigned_thinking(blocks);
            }
        }
    }
}

// ===== Thinking Signature Error Handling =====

/// Handles 400 errors related to thinking signature failures
fn handle_thinking_signature_error(
    request: &mut ClaudeRequest,
    trace_id: &str,
) {
    tracing::warn!(
        "[{}] Unexpected thinking signature error (should have been filtered). \
         Retrying with all thinking blocks removed.",
        trace_id
    );

    // Remove all thinking-related content
    request.thinking = None;

    // Clear Thinking blocks from history
    for msg in &mut request.messages {
        if let MessageContent::Array(blocks) = &mut msg.content {
            blocks.retain(|b| !matches!(b,
                ContentBlock::Thinking { .. } |
                ContentBlock::RedactedThinking { .. }
            ));
        }
    }

    // Clean model name -thinking suffix
    if request.model.contains("claude-") {
        let mut m = request.model.clone();
        m = m.replace("-thinking", "");
        if m.contains("claude-sonnet-4-5-") {
            m = "claude-sonnet-4-5".to_string();
        } else if m.contains("claude-opus-4-5-") || m.contains("claude-opus-4-") {
            m = "claude-opus-4-5".to_string();
        }
        request.model = m;
    }
}

/// Checks if error is a thinking signature error
fn is_thinking_signature_error(error_text: &str) -> bool {
    error_text.contains("Invalid `signature`")
        || error_text.contains("thinking.signature: Field required")
        || error_text.contains("thinking.thinking: Field required")
        || error_text.contains("thinking.signature")
        || error_text.contains("thinking.thinking")
}

// ===== Main Handler =====

/// Handle Claude messages request
pub async fn handle_messages(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    tracing::error!(">>> [RED ALERT] handle_messages called! Body JSON len: {}", body.to_string().len());

    let trace_id = request_id.as_str();

    // Parse request body
    let mut request: ClaudeRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return ProxyError::invalid_request(format!("Invalid request body: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    // Filter and fix Thinking block signatures
    filter_invalid_thinking_blocks(&mut request.messages);

    // [COALESCING] Try to deduplicate identical concurrent requests
    let coalesce_sender = if !request.stream && state.coalescer.is_enabled() {
        let fingerprint = crate::proxy::common::coalescing::calculate_claude_fingerprint(&request);
        match state.coalescer.get_or_create(fingerprint) {
            crate::proxy::common::coalescing::CoalesceResult::Primary(s) => Some(s),
            crate::proxy::common::coalescing::CoalesceResult::Coalesced(r) => {
                debug!("[{trace_id}] Identical Claude request in-flight, waiting for coalesced result...");
                match r.recv().await {
                    Ok(shared_response) => {
                        info!("[{trace_id}] Received coalesced result from primary Claude request");
                        return Json((*shared_response).clone()).into_response();
                    }
                    Err(e) => {
                        tracing::warn!("[{trace_id}] Coalesced Claude request failed ({e}), falling back to individual execution");
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    // Check z.ai dispatch
    if should_use_zai(&state).await {
        let new_body = match serde_json::to_value(&request) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to serialize fixed request for z.ai: {e}");
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return ProxyError::internal_error(format!("Failed to serialize request: {e}"))
                    .with_request_id(request_id)
                    .into_response();
            }
        };

        let res = crate::proxy::providers::zai_anthropic::forward_anthropic_json(
            &state,
            axum::http::Method::POST,
            "/v1/messages",
            &headers,
            new_body,
        )
        .await;

        // [COALESCING] For z.ai, we don't easily get the response back here to broadcast
        // so we fail the coalescing and let other requests go through normally
        if let Some(sender) = coalesce_sender {
            sender.fail();
        }

        return res;
    }

    // Extract meaningful message and log request
    let latest_msg = extract_meaningful_message(&request);
    log_request_details(trace_id, &request, &latest_msg);

    // Setup retry loop
    let upstream = state.upstream.clone();
    let mut request_for_body = request.clone();
    let token_manager = state.token_manager.clone();

    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();
    let mut retried_without_thinking = false;
    let mut overload_retry_count: usize = 0;

    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }

        // Resolve model route and config
        let (mut mapped_model, config) = resolve_model_config(&state, &request_for_body).await;

        // [SCHEDULING] Classify request priority (observability only when scheduler enabled)
        if state.scheduler.is_enabled() {
            let x_priority = headers.get("x-priority").and_then(|h| h.to_str().ok());
            let priority = state.scheduler.classify_priority(x_priority, None, Some(&mapped_model));
            debug!("[{}] Request classified as priority: {}", trace_id, priority);
        }

        // Extract session ID for sticky scheduling
        let session_id_str = crate::proxy::session_manager::SessionManager::extract_session_id(&request_for_body);
        let session_id = Some(session_id_str.as_str());

        let force_rotate_token = attempt > 0;

        // Account selection span
        let account_selection_span = info_span!(
            "account_selection",
            request_type = %config.request_type,
            force_rotate = %force_rotate_token,
            attempt = %attempt,
            otel.kind = "internal",
        );

        let (access_token, project_id, email, account_id) = {
            let _guard = account_selection_span.enter();
            match token_manager.get_token(&config.request_type, force_rotate_token, session_id).await {
                Ok(t) => {
                    info!(account_id = %t.3, email = %t.2, "Account selected successfully");
                    t
                },
                Err(e) => {
                    let safe_message = if e.contains("invalid_grant") {
                        "OAuth refresh failed (invalid_grant): refresh_token likely revoked/expired; reauthorize account(s) to restore service.".to_string()
                    } else {
                        e
                    };
                    error!(error = %safe_message, "Account selection failed");
                    if let Some(sender) = coalesce_sender {
                        sender.fail();
                    }
                    return ProxyError::token_error(format!("No available accounts: {safe_message}"))
                        .with_request_id(request_id.clone())
                        .into_response();
                }
            }
        };

        info!("Using account: {} (type: {})", email, config.request_type);

        // Circuit breaker check
        if let Err(retry_after) = state.circuit_breaker.should_allow(&account_id) {
            tracing::warn!(
                "[{}] Circuit breaker OPEN for account {} - skipping (retry in {:?})",
                trace_id,
                email,
                retry_after
            );
            attempt += 1;
            last_error = format!("Circuit breaker open for account {email}");
            continue;
        }

        // Adaptive rate limit check - skip account if at/near limit
        if let Some(skip_reason) = crate::proxy::handlers::helpers::should_skip_account_adaptive(&state, &account_id) {
            tracing::debug!(
                "[{}] Skipping account {} due to adaptive limit: {}",
                trace_id, email, skip_reason
            );
            attempt += 1;
            last_error = format!("Account {email} {skip_reason}");
            continue;
        }
        crate::proxy::handlers::helpers::log_adaptive_status(&state, &account_id, trace_id);

        // Background task detection and request preparation
        let background_task_type = detect_background_task_type(&request_for_body);
        let mut request_with_mapped = request_for_body.clone();

        if let Some(task_type) = background_task_type {
            prepare_background_task_request(&mut request_with_mapped, &mut mapped_model, task_type, trace_id);
        } else {
            prepare_user_request(&mut request_with_mapped, trace_id, &mapped_model);
        }

        let resolved_model_for_log = mapped_model.clone();
        request_with_mapped.model = mapped_model;

        // Transform request
        let _transform_timer = time_request_transform("claude", &resolved_model_for_log);
        let gemini_body = match transform_claude_request_in(&request_with_mapped, &project_id) {
            Ok(b) => {
                debug!("[{}] Transformed Gemini Body: {}", trace_id, serde_json::to_string_pretty(&b).unwrap_or_default());
                b
            },
            Err(e) => {
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return ProxyError::TransformError(format!("Transform error: {e}"), Some(request_id.clone()))
                    .into_response();
            }
        };

        // Upstream call
        let is_stream = request.stream;
        let method = if is_stream { "streamGenerateContent" } else { "generateContent" };
        let query = if is_stream { Some("alt=sse") } else { None };

        let upstream_span = info_span!(
            "upstream_call",
            provider = "gemini",
            model = %resolved_model_for_log,
            account_id = %account_id,
            method = %method,
            stream = %is_stream,
            otel.kind = "client",
        );

        let _upstream_timer = time_upstream_call("claude", &resolved_model_for_log);
        let response = {
            let _guard = upstream_span.enter();
            let call_start = std::time::Instant::now();

            match upstream.call_v1_internal(
                method,
                &access_token,
                gemini_body,
                query,
                state.request_timeout,
            ).await {
                Ok(r) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    info!(latency_ms = %latency_ms, status = %r.status(), "Upstream call completed");
                    r
                },
                Err(e) => {
                    let latency_ms = call_start.elapsed().as_millis();
                    error!(latency_ms = %latency_ms, error = %e, "Upstream call failed");
                    last_error.clone_from(&e);
                    debug!("Request failed on attempt {}/{}: {}", attempt + 1, max_attempts, e);
                    continue;
                }
            }
        };

        let status = response.status();

        // Handle success
        if status.is_success() {
            record_success_with_probe(&state, &account_id, &access_token, trace_id);

            if request.stream {
                return handle_streaming_response(response, trace_id, email, &resolved_model_for_log);
            }

            return handle_non_streaming_response(
                response,
                &state,
                &request_id,
                trace_id,
                &account_id,
                &resolved_model_for_log,
                &request_for_body,
                &request_with_mapped,
                coalesce_sender,
            ).await;
        }

        // Handle errors
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_else(|_| format!("HTTP {status}"));
        last_error = format!("HTTP {status_code}: {error_text}");
        debug!("[{trace_id}] Upstream Error Response: {error_text}");

        // Record failure in circuit breaker
        record_failure(&state, &account_id, status_code, &error_text);

        // Handle rate limiting
        if status_code == 429 || status_code == 529 || status_code == 503 || status_code == 500 {
            handle_rate_limit_response(RateLimitContext {
                token_manager: &token_manager,
                account_id: &account_id,
                email: &email,
                status_code,
                retry_after: retry_after.as_deref(),
                error_text: &error_text,
                request_type: &config.request_type,
                trace_id,
            });
        }

        // Handle thinking signature error (400)
        if status_code == 400 && !retried_without_thinking && is_thinking_signature_error(&error_text) {
            retried_without_thinking = true;
            handle_thinking_signature_error(&mut request_for_body, trace_id);

            let strategy = determine_retry_strategy(status_code, &error_text, retried_without_thinking);
            if execute_retry_strategy(strategy, attempt, status_code, trace_id).await {
                continue;
            }
        }

        // Handle 529 overload
        if is_overload_error(status_code, &error_text)
            && handle_overload_retry(&mut overload_retry_count, trace_id, &email, "Claude").await
        {
            continue;
        }

        // Standard retry with backoff
        let strategy = determine_retry_strategy(status_code, &error_text, retried_without_thinking);

        if execute_retry_strategy(strategy, attempt, status_code, trace_id).await {
            record_auth_error(&state, &account_id, status_code, &error_text).await;

            if !should_rotate_account(status_code) {
                debug!("[{}] Keeping same account for status {} (server-side issue)", trace_id, status_code);
            }
            attempt += 1;
            continue;
        }

        // Non-retryable error
        error!("[{}] Non-retryable error {}: {}", trace_id, status_code, error_text);
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

// ===== Response Handlers =====

/// Handle streaming response
fn handle_streaming_response(
    response: reqwest::Response,
    trace_id: &str,
    email: String,
    resolved_model: &str,
) -> Response {
    let stream = response.bytes_stream();
    let gemini_stream = Box::pin(stream);
    let claude_stream = create_claude_sse_stream(gemini_stream, trace_id.to_string(), email);

    let sse_stream = claude_stream.map(|result| -> Result<Bytes, std::io::Error> {
        match result {
            Ok(bytes) => Ok(bytes),
            Err(e) => Ok(Bytes::from(format!("data: {{\"error\":\"{e}\"}}\n\n"))),
        }
    });

    build_sse_response(Body::from_stream(sse_stream), resolved_model)
}

/// Handle non-streaming response
async fn handle_non_streaming_response(
    response: reqwest::Response,
    state: &AppState,
    request_id: &RequestId,
    trace_id: &str,
    account_id: &str,
    resolved_model: &str,
    original_request: &ClaudeRequest,
    mapped_request: &ClaudeRequest,
    coalesce_sender: Option<CoalesceSender<Value>>,
) -> Response {
    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            if let Some(sender) = coalesce_sender {
                sender.fail();
            }
            return ProxyError::NetworkError(format!("Failed to read body: {e}"), Some(request_id.clone())).into_response();
        }
    };

    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        debug!("Upstream Response for Claude request: {}", text);
    }

    let gemini_resp: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            if let Some(sender) = coalesce_sender {
                sender.fail();
            }
            return ProxyError::parse_error(format!("Parse error: {e}")).with_request_id(request_id.clone()).into_response();
        }
    };

    let raw = gemini_resp.get("response").unwrap_or(&gemini_resp);

    let gemini_response: crate::proxy::mappers::claude::models::GeminiResponse = match serde_json::from_value(raw.clone()) {
        Ok(r) => r,
        Err(e) => {
            if let Some(sender) = coalesce_sender {
                sender.fail();
            }
            return ProxyError::parse_error(format!("Convert error: {e}")).with_request_id(request_id.clone()).into_response();
        }
    };

    let response_transform_span = info_span!(
        "response_transform",
        provider = "claude",
        model = %resolved_model,
        account_id = %account_id,
        otel.kind = "internal",
    );

    let _response_timer = time_response_transform("claude", resolved_model);
    let claude_response = {
        let _guard = response_transform_span.enter();
        let transform_start = std::time::Instant::now();

        match transform_response(&gemini_response) {
            Ok(r) => {
                let latency_ms = transform_start.elapsed().as_millis();
                info!(
                    latency_ms = %latency_ms,
                    input_tokens = %r.usage.input_tokens,
                    output_tokens = %r.usage.output_tokens,
                    "Response transformation completed"
                );
                r
            },
            Err(e) => {
                error!(error = %e, "Response transformation failed");
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return ProxyError::TransformError(format!("Transform error: {e}"), Some(request_id.clone())).into_response();
            }
        }
    };

    let cache_info = if let Some(cached) = claude_response.usage.cache_read_input_tokens {
        format!(", Cached: {cached}")
    } else {
        String::new()
    };

    tracing::info!(
        "[{}] Request finished. Model: {}, Tokens: In {}, Out {}{}",
        trace_id,
        mapped_request.model,
        claude_response.usage.input_tokens,
        claude_response.usage.output_tokens,
        cache_info
    );

    // [COALESCING] Broadcast successful response to other waiting requests
    if let Some(sender) = coalesce_sender {
        if let Ok(response_value) = serde_json::to_value(&claude_response) {
            sender.send(response_value);
        } else {
            sender.fail();
        }
    }

    // Sampled logging
    if state.sampler.should_sample() {
        use crate::proxy::common::sampling::SampledRequestBuilder;

        let request_json = serde_json::to_string(original_request).unwrap_or_default();
        let (req_excerpt, req_truncated) = state.sampler.truncate_body(&request_json);

        let response_json = serde_json::to_string(&claude_response).unwrap_or_default();
        let (resp_excerpt, resp_truncated) = state.sampler.truncate_body(&response_json);

        let sampled = SampledRequestBuilder::new(trace_id, "POST", "/v1/messages")
            .model(resolved_model)
            .account_id(account_id)
            .request_body(req_excerpt, req_truncated)
            .response_body(resp_excerpt, resp_truncated)
            .status_code(200)
            .tokens(
                Some(u64::from(claude_response.usage.input_tokens)),
                Some(u64::from(claude_response.usage.output_tokens)),
            )
            .build();

        state.sampler.log_sampled_request(&sampled);
    }

    Json(claude_response).with_resolved_model(resolved_model)
}

// ===== Other Endpoints =====

/// List available models
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

/// Count tokens (placeholder)
pub async fn handle_count_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let zai = state.zai.read().await.clone();
    let zai_enabled = zai.enabled && !matches!(zai.dispatch_mode, crate::proxy::ZaiDispatchMode::Off);

    if zai_enabled {
        return crate::proxy::providers::zai_anthropic::forward_anthropic_json(
            &state,
            axum::http::Method::POST,
            "/v1/messages/count_tokens",
            &headers,
            body,
        )
        .await;
    }

    Json(json!({
        "input_tokens": 0,
        "output_tokens": 0
    }))
    .into_response()
}

// ===== Background Task Detection =====

fn detect_background_task_type(request: &ClaudeRequest) -> Option<BackgroundTaskType> {
    let last_user_msg = extract_last_user_message(request)?;
    background_task::detect_from_text(&last_user_msg)
}

fn extract_last_user_message(request: &ClaudeRequest) -> Option<String> {
    request.messages.iter().rev()
        .filter(|m| m.role == "user")
        .find_map(|m| {
            let content = match &m.content {
                MessageContent::String(s) => s.clone(),
                MessageContent::Array(arr) => {
                    arr.iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            };

            if content.trim().is_empty()
                || content.starts_with("Warmup")
                || content.contains("<system-reminder>")
            {
                None
            } else {
                Some(content)
            }
        })
}
