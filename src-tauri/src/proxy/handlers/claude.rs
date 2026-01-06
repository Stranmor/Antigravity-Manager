// Claude 协议处理器

use axum::{
    body::Body,
    extract::{Json, State},
    response::{IntoResponse, Response},
    Extension,
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, info_span};

use crate::proxy::mappers::claude::{
    transform_claude_request_in, transform_response, create_claude_sse_stream, ClaudeRequest,
};
use crate::proxy::server::AppState;
use crate::proxy::handlers::common::WithResolvedModel;
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::common::perf::{time_request_transform, time_response_transform, time_upstream_call};
use crate::proxy::common::retry::{
    apply_jitter, determine_retry_strategy, execute_retry_strategy, should_rotate_account,
    MAX_RETRY_ATTEMPTS, MAX_OVERLOAD_RETRIES, OVERLOAD_BASE_DELAY_MS, OVERLOAD_MAX_DELAY_MS,
};
use crate::proxy::common::sse::build_sse_response;
use crate::proxy::common::background_task::{self, BackgroundTaskType};
use axum::http::HeaderMap;
use std::sync::atomic::Ordering;

const MIN_SIGNATURE_LENGTH: usize = 10;

// ===== Thinking 块处理辅助函数 =====

use crate::proxy::mappers::claude::models::{ContentBlock, Message, MessageContent};

/// 检查 thinking 块是否有有效签名
fn has_valid_signature(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Thinking { signature, thinking, .. } => {
            // 空 thinking + 任意 signature = 有效 (trailing signature case)
            if thinking.is_empty() && signature.is_some() {
                return true;
            }
            // 有内容 + 足够长度的 signature = 有效
            signature.as_ref().is_some_and(|s| s.len() >= MIN_SIGNATURE_LENGTH)
        }
        _ => true  // 非 thinking 块默认有效
    }
}

/// 清理 thinking 块,只保留必要字段(移除 cache_control 等)
fn sanitize_thinking_block(block: ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::Thinking { thinking, signature, .. } => {
            // 重建块,移除 cache_control 等额外字段
            ContentBlock::Thinking {
                thinking,
                signature,
                cache_control: None,
            }
        }
        _ => block
    }
}

/// 过滤消息中的无效 thinking 块
fn filter_invalid_thinking_blocks(messages: &mut [Message]) {
    let mut total_filtered = 0;
    
    for msg in messages.iter_mut() {
        // 只处理 assistant 消息
        // [CRITICAL FIX] Handle 'model' role too (Google history usage)
        if msg.role != "assistant" && msg.role != "model" {
            continue;
        }
        tracing::error!("[DEBUG-FILTER] Inspecting msg with role: {}", msg.role);
        
        if let MessageContent::Array(blocks) = &mut msg.content {
            let original_len = blocks.len();
            
            // 过滤并清理
            let mut new_blocks = Vec::new();
            for block in blocks.drain(..) {
                if matches!(block, ContentBlock::Thinking { .. }) {
                    // [DEBUG] 强制输出日志
                    if let ContentBlock::Thinking { ref signature, .. } = block {
                         tracing::error!("[DEBUG-FILTER] Found thinking block. Sig len: {:?}", signature.as_ref().map(std::string::String::len));
                    }

                    // [CRITICAL FIX] Vertex AI 不认可 skip_thought_signature_validator
                    // 必须直接删除无效的 thinking 块
                    if has_valid_signature(&block) {
                        new_blocks.push(sanitize_thinking_block(block));
                    } else {
                        // [IMPROVED] 保留内容转换为 text，而不是直接丢弃
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
            
            // 如果过滤后为空,添加一个空文本块以保持消息有效
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

/// 移除尾部的无签名 thinking 块
fn remove_trailing_unsigned_thinking(blocks: &mut Vec<ContentBlock>) {
    if blocks.is_empty() {
        return;
    }
    
    // 从后向前扫描
    let mut end_index = blocks.len();
    for i in (0..blocks.len()).rev() {
        match &blocks[i] {
            ContentBlock::Thinking { .. } => {
                if has_valid_signature(&blocks[i]) {
                    break;  // 遇到有效签名的 thinking 块,停止
                }
                end_index = i;
            }
            _ => break  // 遇到非 thinking 块,停止
        }
    }
    
    if end_index < blocks.len() {
        let removed = blocks.len() - end_index;
        blocks.truncate(end_index);
        debug!("Removed {} trailing unsigned thinking block(s)", removed);
    }
}

/// 处理 Claude messages 请求
/// 处理 Chat 消息请求流程
pub async fn handle_messages(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    tracing::error!(">>> [RED ALERT] handle_messages called! Body JSON len: {}", body.to_string().len());

    // Use middleware-provided request_id for consistent tracing
    let trace_id = request_id.as_str();
        
    // Decide whether this request should be handled by z.ai (Anthropic passthrough) or the existing Google flow.
    let zai = state.zai.read().await.clone();
    let zai_enabled = zai.enabled && !matches!(zai.dispatch_mode, crate::proxy::ZaiDispatchMode::Off);
    let google_accounts = state.token_manager.len();

    let use_zai = if zai_enabled {
        match zai.dispatch_mode {
            crate::proxy::ZaiDispatchMode::Off => false,
            crate::proxy::ZaiDispatchMode::Exclusive => true,
            crate::proxy::ZaiDispatchMode::Fallback => google_accounts == 0,
            crate::proxy::ZaiDispatchMode::Pooled => {
                // Treat z.ai as exactly one extra slot in the pool.
                // No strict guarantees: it may get 0 requests if selection never hits.
                let total = google_accounts.saturating_add(1).max(1);
                let slot = state.provider_rr.fetch_add(1, Ordering::Relaxed) % total;
                slot == 0
            }
        }
    } else {
        false
    };

    // [CRITICAL REFACTOR] 优先解析并过滤 Thinking 块，确保 z.ai 也是用修复后的 Body
    let mut request: crate::proxy::mappers::claude::models::ClaudeRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return ProxyError::invalid_request(format!("Invalid request body: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    // [CRITICAL FIX] 过滤并修复 Thinking 块签名
    filter_invalid_thinking_blocks(&mut request.messages);

    if use_zai {
        // 重新序列化修复后的请求体
        let new_body = match serde_json::to_value(&request) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to serialize fixed request for z.ai: {e}");
                return ProxyError::internal_error(format!("Failed to serialize request: {e}"))
                    .with_request_id(request_id)
                    .into_response();
            }
        };

        return crate::proxy::providers::zai_anthropic::forward_anthropic_json(
            &state,
            axum::http::Method::POST,
            "/v1/messages",
            &headers,
            new_body,
        )
        .await;
    }
    
    // Google Flow 继续使用 request 对象
    // (后续代码不需要再次 filter_invalid_thinking_blocks)

    // 获取最新一条“有意义”的消息内容（用于日志记录和后台任务检测）
    // 策略：反向遍历，首先筛选出所有角色为 "user" 的消息，然后从中找到第一条非 "Warmup" 且非空的文本消息
    // 获取最新一条“有意义”的消息内容（用于日志记录和后台任务检测）
    // 策略：反向遍历，首先筛选出所有和用户相关的消息 (role="user")
    // 然后提取其文本内容，跳过 "Warmup" 或系统预设的 reminder
    let meaningful_msg = request.messages.iter().rev()
        .filter(|m| m.role == "user")
        .find_map(|m| {
            let content = match &m.content {
                crate::proxy::mappers::claude::models::MessageContent::String(s) => s.clone(),
                crate::proxy::mappers::claude::models::MessageContent::Array(arr) => {
                    // 对于数组，提取所有 Text 块并拼接，忽略 ToolResult
                    arr.iter()
                        .filter_map(|block| match block {
                            crate::proxy::mappers::claude::models::ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            };
            
            // 过滤规则：
            // 1. 忽略空消息
            // 2. 忽略 "Warmup" 消息
            // 3. 忽略 <system-reminder> 标签的消息
            if content.trim().is_empty() 
                || content.starts_with("Warmup") 
                || content.contains("<system-reminder>") 
            {
                None 
            } else {
                Some(content)
            }
        });

    // 如果经过过滤还是找不到（例如纯工具调用），则回退到最后一条消息的原始展示
    let latest_msg = meaningful_msg.unwrap_or_else(|| {
        request.messages.last().map_or_else(
            || "[No Messages]".to_string(),
            |m| match &m.content {
                crate::proxy::mappers::claude::models::MessageContent::String(s) => s.clone(),
                crate::proxy::mappers::claude::models::MessageContent::Array(_) => "[Complex/Tool Message]".to_string()
            },
        )
    });
    
    
    // INFO 级别: 简洁的一行摘要
    info!(
        "[{}] Claude Request | Model: {} | Stream: {} | Messages: {} | Tools: {}",
        trace_id,
        request.model,
        request.stream,
        request.messages.len(),
        request.tools.is_some()
    );
    
    // DEBUG 级别: 详细的调试信息
    debug!("========== [{}] CLAUDE REQUEST DEBUG START ==========", trace_id);
    debug!("[{}] Model: {}", trace_id, request.model);
    debug!("[{}] Stream: {}", trace_id, request.stream);
    debug!("[{}] Max Tokens: {:?}", trace_id, request.max_tokens);
    debug!("[{}] Temperature: {:?}", trace_id, request.temperature);
    debug!("[{}] Message Count: {}", trace_id, request.messages.len());
    debug!("[{}] Has Tools: {}", trace_id, request.tools.is_some());
    debug!("[{}] Has Thinking Config: {}", trace_id, request.thinking.is_some());
    debug!("[{}] Content Preview: {:.100}...", trace_id, latest_msg);
    
    // 输出每一条消息的详细信息
    for (idx, msg) in request.messages.iter().enumerate() {
        let content_preview = match &msg.content {
            crate::proxy::mappers::claude::models::MessageContent::String(s) => {
                if s.len() > 200 {
                    format!("{}... (total {} chars)", &s[..200], s.len())
                } else {
                    s.clone()
                }
            },
            crate::proxy::mappers::claude::models::MessageContent::Array(arr) => {
                format!("[Array with {} blocks]", arr.len())
            }
        };
        debug!("[{}] Message[{}] - Role: {}, Content: {}", 
            trace_id, idx, msg.role, content_preview);
    }
    
    debug!("[{}] Full Claude Request JSON: {}", trace_id, serde_json::to_string_pretty(&request).unwrap_or_default());
    debug!("========== [{}] CLAUDE REQUEST DEBUG END ==========", trace_id);

    // 1. 获取 会话 ID (已废弃基于内容的哈希，改用 TokenManager 内部的时间窗口锁定)
    // session_id is reserved for potential future sticky session features

    // 2. 获取 UpstreamClient
    let upstream = state.upstream.clone();
    
    // 3. 准备闭包
    let mut request_for_body = request.clone();
    let token_manager = state.token_manager;

    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();
    let mut retried_without_thinking = false;

    // [529 RESILIENCE] Separate counter for 529 overload retries
    // This allows unlimited retries for server overload (same account)
    // while still respecting account rotation limits for other errors
    let mut overload_retry_count: usize = 0;
    #[allow(unused)]  // Reserved for future logging/metrics of overload patterns
    let mut current_overload_account: Option<String> = None;

    // Use manual loop instead of `for` to allow 529 retries without consuming attempt quota
    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }
        // 2. 模型路由与配置解析 (提前解析以确定请求类型)
        // 先不应用家族映射，获取初步的 mapped_model
        let initial_mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
            &request_for_body.model,
            &*state.custom_mapping.read().await,
            &*state.openai_mapping.read().await,
            &*state.anthropic_mapping.read().await,
            false,  // 先不应用家族映射
        );
        
        // 将 Claude 工具转为 Value 数组以便探测联网
        let tools_val: Option<Vec<Value>> = request_for_body.tools.as_ref().map(|list| {
            list.iter().map(|t| serde_json::to_value(t).unwrap_or(json!({}))).collect()
        });

        let config = crate::proxy::mappers::common_utils::resolve_request_config(&request_for_body.model, &initial_mapped_model, &tools_val);

        // 3. 根据 request_type 决定是否应用 Claude 家族映射
        // request_type == "agent" 表示 CLI 请求，应该应用家族映射
        // 其他类型（web_search, image_gen）不应用家族映射
        let is_cli_request = config.request_type == "agent";
        
        let mut mapped_model = if is_cli_request {
            // CLI 请求：重新调用 resolve_model_route，应用家族映射
            crate::proxy::common::model_mapping::resolve_model_route(
                &request_for_body.model,
                &*state.custom_mapping.read().await,
                &*state.openai_mapping.read().await,
                &*state.anthropic_mapping.read().await,
                true,  // CLI 请求应用家族映射
            )
        } else {
            // 非 CLI 请求：使用初步的 mapped_model（已跳过家族映射）
            initial_mapped_model
        };

        // 0. 尝试提取 session_id 用于粘性调度 (Phase 2/3)
        // 使用 SessionManager 生成稳定的会话指纹
        let session_id_str = crate::proxy::session_manager::SessionManager::extract_session_id(&request_for_body);
        let session_id = Some(session_id_str.as_str());

        let force_rotate_token = attempt > 0;

        // === SPAN: Account Selection Phase ===
        // Note: otel.kind = "internal" marks this as an internal operation
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
                    return ProxyError::token_error(format!("No available accounts: {safe_message}"))
                        .with_request_id(request_id.clone())
                        .into_response();
                }
            }
        };

        info!("Using account: {} (type: {})", email, config.request_type);

        // === Circuit Breaker Check ===
        // Fast-fail if account has too many recent failures
        if let Err(retry_after) = state.circuit_breaker.should_allow(&account_id) {
            tracing::warn!(
                "[{}] Circuit breaker OPEN for account {} - skipping (retry in {:?})",
                trace_id,
                email,
                retry_after
            );
            // Force rotate to next account on retry
            attempt += 1;
            last_error = format!("Circuit breaker open for account {email}");
            continue;
        }

        // ===== 【优化】后台任务智能检测与降级 =====
        // 使用新的检测系统，支持 5 大类关键词和多 Flash 模型策略
        let background_task_type = detect_background_task_type(&request_for_body);
        
        // 传递映射后的模型名
        let mut request_with_mapped = request_for_body.clone();

        if let Some(task_type) = background_task_type {
            // 检测到后台任务,强制降级到 Flash 模型
            let downgrade_model = background_task::select_model(task_type);
            
            info!(
                "[{}][AUTO] 检测到后台任务 (类型: {:?}),强制降级: {} -> {}",
                trace_id,
                task_type,
                mapped_model,
                downgrade_model
            );
            
            // 覆盖用户自定义映射
            mapped_model = downgrade_model.to_string();
            
            // 后台任务净化：
            // 1. 移除工具定义（后台任务不需要工具）
            request_with_mapped.tools = None;
            
            // 2. 移除 Thinking 配置（Flash 模型不支持）
            request_with_mapped.thinking = None;
            
            // 3. 清理历史消息中的 Thinking Block，防止 Invalid Argument
            for msg in &mut request_with_mapped.messages {
                if let crate::proxy::mappers::claude::models::MessageContent::Array(blocks) = &mut msg.content {
                    blocks.retain(|b| !matches!(b, 
                        crate::proxy::mappers::claude::models::ContentBlock::Thinking { .. } |
                        crate::proxy::mappers::claude::models::ContentBlock::RedactedThinking { .. }
                    ));
                }
            }
        } else {
            // 真实用户请求,保持原映射
            debug!(
                "[{}][USER] 用户交互请求,保持映射: {}",
                trace_id,
                mapped_model
            );
            
            // 对真实请求应用额外的清理:移除尾部无签名的 thinking 块
            // 对真实请求应用额外的清理:移除尾部无签名的 thinking 块
            for msg in &mut request_with_mapped.messages {
                if msg.role == "assistant" || msg.role == "model" {
                    if let crate::proxy::mappers::claude::models::MessageContent::Array(blocks) = &mut msg.content {
                        remove_trailing_unsigned_thinking(blocks);
                    }
                }
            }
        }

        // Save a copy of mapped_model for logging before moving it
        let resolved_model_for_log = mapped_model.clone();
        request_with_mapped.model = mapped_model;

        // 生成 Trace ID (简单用时间戳后缀)
        // let _trace_id = format!("req_{}", chrono::Utc::now().timestamp_subsec_millis());

        // [PERF] Time request transformation
        let _transform_timer = time_request_transform("claude", &resolved_model_for_log);
        let gemini_body = match transform_claude_request_in(&request_with_mapped, &project_id) {
            Ok(b) => {
                debug!("[{}] Transformed Gemini Body: {}", trace_id, serde_json::to_string_pretty(&b).unwrap_or_default());
                b
            },
            Err(e) => {
                return ProxyError::TransformError(format!("Transform error: {e}"), Some(request_id.clone()))
                    .into_response();
            }
        };
        // transform_timer is dropped here, recording the duration

    // 4. 上游调用
    let is_stream = request.stream;
    let method = if is_stream { "streamGenerateContent" } else { "generateContent" };
    let query = if is_stream { Some("alt=sse") } else { None };

    // === SPAN: Upstream API Call Phase ===
    // Note: otel.kind = "client" marks this as an outgoing client call
    let upstream_span = info_span!(
        "upstream_call",
        provider = "gemini",
        model = %resolved_model_for_log,
        account_id = %account_id,
        method = %method,
        stream = %is_stream,
        otel.kind = "client",
    );

    // [PERF] Time upstream API call
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

        // 成功
        if status.is_success() {
            // Record success in health monitor and circuit breaker
            state.health_monitor.record_success(&account_id);
            state.circuit_breaker.record_success(&account_id);

            // 处理流式响应
            if request.stream {
                let stream = response.bytes_stream();
                let gemini_stream = Box::pin(stream);
                let claude_stream = create_claude_sse_stream(gemini_stream, trace_id.to_string(), email);

                // 转换为 Bytes stream
                let sse_stream = claude_stream.map(|result| -> Result<Bytes, std::io::Error> {
                    match result {
                        Ok(bytes) => Ok(bytes),
                        Err(e) => Ok(Bytes::from(format!("data: {{\"error\":\"{e}\"}}\n\n"))),
                    }
                });

                return build_sse_response(Body::from_stream(sse_stream), resolved_model_for_log.as_str());
            }
            // 处理非流式响应
            let bytes = match response.bytes().await {
                Ok(b) => b,
                Err(e) => return ProxyError::NetworkError(format!("Failed to read body: {e}"), Some(request_id.clone())).into_response(),
            };

            // Debug print
            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                debug!("Upstream Response for Claude request: {}", text);
            }

            let gemini_resp: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => return ProxyError::parse_error(format!("Parse error: {e}")).with_request_id(request_id.clone()).into_response(),
            };

            // 解包 response 字段（v1internal 格式）
            let raw = gemini_resp.get("response").unwrap_or(&gemini_resp);

            // 转换为 Gemini Response 结构
            let gemini_response: crate::proxy::mappers::claude::models::GeminiResponse = match serde_json::from_value(raw.clone()) {
                Ok(r) => r,
                Err(e) => return ProxyError::parse_error(format!("Convert error: {e}")).with_request_id(request_id.clone()).into_response(),
            };

            // === SPAN: Response Transformation Phase ===
            // Note: otel.kind = "internal" marks this as an internal operation
            let response_transform_span = info_span!(
                "response_transform",
                provider = "claude",
                model = %resolved_model_for_log,
                account_id = %account_id,
                otel.kind = "internal",
            );

            // [PERF] Time response transformation
            let _response_timer = time_response_transform("claude", &resolved_model_for_log);
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
                        return ProxyError::TransformError(format!("Transform error: {e}"), Some(request_id.clone())).into_response();
                    }
                }
            };
            // response_timer is dropped here, recording the duration

            // [Optimization] 记录闭环日志：消耗情况
            let cache_info = if let Some(cached) = claude_response.usage.cache_read_input_tokens {
                format!(", Cached: {cached}")
            } else {
                String::new()
            };

            tracing::info!(
                "[{}] Request finished. Model: {}, Tokens: In {}, Out {}{}",
                trace_id,
                request_with_mapped.model,
                claude_response.usage.input_tokens,
                claude_response.usage.output_tokens,
                cache_info
            );

            return Json(claude_response).with_resolved_model(&resolved_model_for_log);
        }
        
        // 1. 立即提取状态码和 headers（防止 response 被 move）
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        
        // 2. 获取错误文本并转移 Response 所有权
        let error_text = response.text().await.unwrap_or_else(|_| format!("HTTP {status}"));
        last_error = format!("HTTP {status_code}: {error_text}");
        debug!("[{trace_id}] Upstream Error Response: {error_text}");

        // 2.5 Record failure in circuit breaker for account-specific errors
        // Skip 529 (global overload) as it's not account-specific
        if status_code != 529 && (status_code >= 400) {
            state.circuit_breaker.record_failure(&account_id, &error_text);
        }

        // 3. 标记限流状态（用于 UI 显示）并自动解绑所有关联会话
        // [IMPROVEMENT] 使用 mark_rate_limited_and_unbind 确保 429 时立即切换账号
        // [FIX] 使用 account_id 而非 email 作为 rate limit key
        if status_code == 429 || status_code == 529 || status_code == 503 || status_code == 500 {
            token_manager.mark_rate_limited_and_unbind(
                &account_id,
                status_code,
                retry_after.as_deref(),
                &error_text,
                Some(&config.request_type),  // Per-model rate limiting
            );

            // [PERSISTENCE] Record rate limit event to database (fire-and-forget)
            if status_code == 429 {
                let account_id_owned = account_id.clone();
                let quota_group = config.request_type.clone();
                let retry_after_secs = retry_after.as_ref().and_then(|r| r.parse::<i32>().ok());
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

        // 4. 处理 400 错误 (Thinking 签名失效)
        // 由于已经主动过滤,这个错误应该很少发生
        if status_code == 400
            && !retried_without_thinking
            && (error_text.contains("Invalid `signature`")
                || error_text.contains("thinking.signature: Field required")
                || error_text.contains("thinking.thinking: Field required")
                || error_text.contains("thinking.signature")
                || error_text.contains("thinking.thinking"))
        {
            retried_without_thinking = true;
            
            // 使用 WARN 级别,因为这不应该经常发生(已经主动过滤过)
            tracing::warn!(
                "[{}] Unexpected thinking signature error (should have been filtered). \
                 Retrying with all thinking blocks removed.",
                trace_id
            );

            // 完全移除所有 thinking 相关内容
            request_for_body.thinking = None;
            
            // 清理历史消息中的所有 Thinking Block
            for msg in &mut request_for_body.messages {
                if let crate::proxy::mappers::claude::models::MessageContent::Array(blocks) = &mut msg.content {
                    blocks.retain(|b| !matches!(b, 
                        crate::proxy::mappers::claude::models::ContentBlock::Thinking { .. } |
                        crate::proxy::mappers::claude::models::ContentBlock::RedactedThinking { .. }
                    ));
                }
            }
            
            // 清理模型名中的 -thinking 后缀
            if request_for_body.model.contains("claude-") {
                let mut m = request_for_body.model.clone();
                m = m.replace("-thinking", "");
                if m.contains("claude-sonnet-4-5-") {
                    m = "claude-sonnet-4-5".to_string();
                } else if m.contains("claude-opus-4-5-") || m.contains("claude-opus-4-") {
                    m = "claude-opus-4-5".to_string();
                }
                request_for_body.model = m;
            }
            
            // 使用统一退避策略
            let strategy = determine_retry_strategy(status_code, &error_text, retried_without_thinking);
            if execute_retry_strategy(strategy, attempt, status_code, trace_id).await {
                continue;
            }
        }

        // 5. [529 RESILIENCE] Special handling for 529 Overloaded errors
        // 529 means the upstream server is overloaded - this is NOT account-specific
        // We retry aggressively with exponential backoff until success or max retries
        if status_code == 529 || (status_code == 503 && error_text.contains("overloaded")) {
            overload_retry_count += 1;
            let _ = current_overload_account.insert(email.clone());  // Reserved for future metrics

            if overload_retry_count <= MAX_OVERLOAD_RETRIES {
                // Exponential backoff with jitter: 2s, 4s, 8s, 16s, ... capped at 60s
                let base_delay = OVERLOAD_BASE_DELAY_MS * 2_u64.pow((overload_retry_count - 1).min(5) as u32);
                let capped_delay = base_delay.min(OVERLOAD_MAX_DELAY_MS);
                let jittered_delay = apply_jitter(capped_delay);

                tracing::warn!(
                    "[{}] 🔄 529 Overloaded - retry {}/{} in {}ms (account: {}, NOT rotating)",
                    trace_id,
                    overload_retry_count,
                    MAX_OVERLOAD_RETRIES,
                    jittered_delay,
                    email
                );

                sleep(Duration::from_millis(jittered_delay)).await;

                // CRITICAL: Do NOT increment `attempt` - 529 retries are "free"
                // This allows us to keep retrying without exhausting the account pool
                continue;
            }
            tracing::error!(
                "[{}] ❌ 529 Overloaded - exhausted {} retries, giving up",
                trace_id,
                MAX_OVERLOAD_RETRIES
            );
            // Fall through to normal error handling after max retries
        }

        // 6. 统一处理所有可重试错误
        // [REMOVED] 不再特殊处理 QUOTA_EXHAUSTED,允许账号轮换
        // 原逻辑会在第一个账号配额耗尽时直接返回,导致"平衡"模式无法切换账号
        
        
        // 确定重试策略
        let strategy = determine_retry_strategy(status_code, &error_text, retried_without_thinking);

        // 执行退避
        if execute_retry_strategy(strategy, attempt, status_code, trace_id).await {
            // Record error in health monitor for 403/401 (may auto-disable account)
            if status_code == 403 || status_code == 401 {
                state.health_monitor.record_error(&account_id, status_code, &error_text).await;
            }

            // 判断是否需要轮换账号
            if !should_rotate_account(status_code) {
                debug!("[{}] Keeping same account for status {} (server-side issue)", trace_id, status_code);
            }
            // Increment attempt counter for non-529 errors (account rotation)
            attempt += 1;
            continue;
        }
        // 不可重试的错误，直接返回
        error!("[{}] Non-retryable error {}: {}", trace_id, status_code, error_text);
        return ProxyError::upstream_error(status_code, error_text)
            .with_request_id(request_id)
            .into_response();
    }

    // Include 529 retry info in final error message if applicable
    let retry_info = if overload_retry_count > 0 {
        format!(" (including {overload_retry_count} overload retries)")
    } else {
        String::new()
    };

    ProxyError::Overloaded(
        format!(
            "All {max_attempts} attempts failed{retry_info}. Last error: {last_error}"
        ),
        Some(request_id),
    )
    .into_response()
}

/// 列出可用模型
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

/// 计算 tokens (占位符)
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

// 移除已失效的简单单元测试，后续将补全完整的集成测试
/*
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_list_models() {
        // handle_list_models 现在需要 AppState，此处跳过旧的单元测试
    }
}
*/

// ===== Background Task Detection (Claude-specific wrapper) =====

fn detect_background_task_type(request: &ClaudeRequest) -> Option<BackgroundTaskType> {
    let last_user_msg = extract_last_user_message(request)?;
    background_task::detect_from_text(&last_user_msg)
}

fn extract_last_user_message(request: &ClaudeRequest) -> Option<String> {
    request.messages.iter().rev()
        .filter(|m| m.role == "user")
        .find_map(|m| {
            let content = match &m.content {
                crate::proxy::mappers::claude::models::MessageContent::String(s) => s.clone(),
                crate::proxy::mappers::claude::models::MessageContent::Array(arr) => {
                    arr.iter()
                        .filter_map(|block| match block {
                            crate::proxy::mappers::claude::models::ContentBlock::Text { text } => Some(text.as_str()),
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
