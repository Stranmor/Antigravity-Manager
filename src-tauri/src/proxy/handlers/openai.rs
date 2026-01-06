// OpenAI Handler
use axum::{extract::Json, extract::State, response::{IntoResponse, Response}, Extension};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};

use crate::proxy::mappers::openai::{
    transform_openai_request, transform_openai_response, OpenAIRequest,
};
use crate::proxy::server::AppState;
use crate::proxy::handlers::common::WithResolvedModel;
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::common::perf::{time_request_transform, time_response_transform, time_upstream_call};
use crate::proxy::common::retry::{
    apply_jitter, MAX_RETRY_ATTEMPTS, MAX_OVERLOAD_RETRIES, OVERLOAD_BASE_DELAY_MS, OVERLOAD_MAX_DELAY_MS,
};
use crate::proxy::common::sse::build_sse_response;
use crate::proxy::common::background_task;
use crate::proxy::session_manager::SessionManager;

pub async fn handle_chat_completions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    let mut openai_req: OpenAIRequest = match serde_json::from_value(body) {
        Ok(req) => req,
        Err(e) => return ProxyError::invalid_request(format!("Invalid request: {e}"))
            .with_request_id(request_id)
            .into_response(),
    };

    // Safety: Ensure messages is not empty
    if openai_req.messages.is_empty() {
        debug!("Received request with empty messages, injecting fallback...");
        openai_req
            .messages
            .push(crate::proxy::mappers::openai::OpenAIMessage {
                role: "user".to_string(),
                content: Some(crate::proxy::mappers::openai::OpenAIContent::String(
                    " ".to_string(),
                )),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
    }

    debug!("Received OpenAI request for model: {}", openai_req.model);

    // [Background Task Detection] Check if this is a background task and downgrade model if needed
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

    // 1. 获取 UpstreamClient (Clone handle)
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager;
    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();

    // [529 RESILIENCE] Separate counter for 529 overload retries
    // This allows unlimited retries for server overload (same account)
    // while still respecting account rotation limits for other errors
    let mut overload_retry_count: usize = 0;
    #[allow(unused)]  // Reserved for future logging/metrics of overload patterns
    let mut current_overload_account: Option<String> = None;

    // Generate trace ID for logging
    let trace_id: String = rand::Rng::sample_iter(rand::thread_rng(), &rand::distributions::Alphanumeric)
        .take(6)
        .map(char::from)
        .collect::<String>().to_lowercase();

    // Use manual loop instead of `for` to allow 529 retries without consuming attempt quota
    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }
        // 2. 预解析模型路由与配置
        let mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
            &openai_req.model,
            &*state.custom_mapping.read().await,
            &*state.openai_mapping.read().await,
            &*state.anthropic_mapping.read().await,
            false,  // OpenAI 请求不应用 Claude 家族映射
        );
        // 将 OpenAI 工具转为 Value 数组以便探测联网
        let tools_val: Option<Vec<Value>> = openai_req
            .tools.clone();
        let config = crate::proxy::mappers::common_utils::resolve_request_config(
            &openai_req.model,
            &mapped_model,
            &tools_val,
        );

        // 3. 提取 SessionId (粘性指纹)
        let session_id = SessionManager::extract_openai_session_id(&openai_req);

        // 4. 获取 Token (使用准确的 request_type)
        // 关键：在重试尝试 (attempt > 0) 时强制轮换账号
        let (access_token, project_id, email, account_id) = match token_manager
            .get_token(&config.request_type, attempt > 0, Some(&session_id))
            .await
        {
            Ok(t) => t,
            Err(e) => {
                return ProxyError::token_error(format!("Token error: {e}"))
                    .with_request_id(request_id)
                    .into_response();
            }
        };

        // Suppress unused warning for account_id (used for rate limiting in error paths)
        let _ = &account_id;

        info!("✓ Using account: {} (type: {})", email, config.request_type);

        // [PERF] Time request transformation
        let _transform_timer = time_request_transform("openai", &mapped_model);
        let gemini_body = transform_openai_request(&openai_req, &project_id, &mapped_model);
        // transform_timer is dropped here, recording the duration

        // [New] 打印转换后的报文 (Gemini Body) 供调试
        if let Ok(body_json) = serde_json::to_string_pretty(&gemini_body) {
            debug!("[OpenAI-Request] Transformed Gemini Body:\n{}", body_json);
        }

        // 5. 发送请求
        let list_response = openai_req.stream;
        let method = if list_response {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let query_string = if list_response { Some("alt=sse") } else { None };

        // [PERF] Time upstream API call
        let _upstream_timer = time_upstream_call("openai", &mapped_model);
        let response = match upstream
            .call_v1_internal(method, &access_token, gemini_body, query_string)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_error.clone_from(&e);
                debug!(
                    "OpenAI Request failed on attempt {}/{}: {}",
                    attempt + 1,
                    max_attempts,
                    e
                );
                continue;
            }
        };

        let status = response.status();
        if status.is_success() {
            // Record success in health monitor
            state.health_monitor.record_success(&account_id);

            // 5. 处理流式 vs 非流式
            if list_response {
                use crate::proxy::mappers::openai::streaming::create_openai_sse_stream;
                use axum::body::Body;

                let gemini_stream = response.bytes_stream();
                let openai_stream =
                    create_openai_sse_stream(Box::pin(gemini_stream), openai_req.model.clone());
                let body = Body::from_stream(openai_stream);

                return build_sse_response(body, mapped_model.as_str());
            }

            let gemini_resp: Value = match response.json().await {
                Ok(v) => v,
                Err(e) => return ProxyError::parse_error(format!("Parse error: {e}"))
                    .with_request_id(request_id)
                    .into_response(),
            };

            // [PERF] Time response transformation
            let _response_timer = time_response_transform("openai", &mapped_model);
            let openai_response = transform_openai_response(&gemini_resp);
            // response_timer is dropped here, recording the duration
            return Json(openai_response).with_resolved_model(&mapped_model);
        }

        // 处理特定错误并重试
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_else(|_| format!("HTTP {status_code}"));
        last_error = format!("HTTP {status_code}: {error_text}");

        // [New] 打印错误报文日志
        tracing::error!(
            "[OpenAI-Upstream] Error Response {}: {}",
            status_code,
            error_text
        );

        // 429/529/503 智能处理
        if status_code == 429 || status_code == 529 || status_code == 503 || status_code == 500 {
            // 记录限流信息 (全局同步)
            token_manager.mark_rate_limited(&email, status_code, retry_after.as_deref(), &error_text);

            // [529 RESILIENCE] Special handling for 529 Overloaded errors
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

            // 1. 优先尝试解析 RetryInfo (由 Google Cloud 直接下发)
            if let Some(delay_ms) = crate::proxy::upstream::retry::parse_retry_delay(&error_text) {
                let actual_delay = delay_ms.saturating_add(200).min(10_000);
                let jittered_delay = apply_jitter(actual_delay);
                tracing::warn!(
                    "[{}] OpenAI Upstream {} on {} attempt {}/{}, waiting {}ms then retrying",
                    trace_id,
                    status_code,
                    email,
                    attempt + 1,
                    max_attempts,
                    jittered_delay
                );
                sleep(Duration::from_millis(jittered_delay)).await;
                attempt += 1;
                continue;
            }

            // 2. 只有明确包含 "QUOTA_EXHAUSTED" 才停止，避免误判频率提示 (如 "check quota")
            if error_text.contains("QUOTA_EXHAUSTED") {
                error!(
                    "[{}] OpenAI Quota exhausted (429) on account {} attempt {}/{}, stopping to protect pool.",
                    trace_id,
                    email,
                    attempt + 1,
                    max_attempts
                );
                return ProxyError::RateLimited(error_text, Some(request_id)).into_response();
            }

            // 3. 其他限流或服务器过载情况，轮换账号
            tracing::warn!(
                "[{}] OpenAI Upstream {} on {} attempt {}/{}, rotating account",
                trace_id,
                status_code,
                email,
                attempt + 1,
                max_attempts
            );
            attempt += 1;
            continue;
        }

        // 只有 403 (权限/地区限制) 和 401 (认证失效) 触发账号轮换
        if status_code == 403 || status_code == 401 {
            // Record error in health monitor (may auto-disable account)
            state.health_monitor.record_error(&account_id, status_code, &error_text).await;

            tracing::warn!(
                "[{}] OpenAI Upstream {} on account {} attempt {}/{}, rotating account",
                trace_id,
                status_code,
                email,
                attempt + 1,
                max_attempts
            );
            attempt += 1;
            continue;
        }

        // 404 等由于模型配置或路径错误的 HTTP 异常，直接报错，不进行无效轮换
        error!(
            "[{}] OpenAI Upstream non-retryable error {} on account {}: {}",
            trace_id, status_code, email, error_text
        );
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

    // 所有尝试均失败
    ProxyError::Overloaded(
        format!(
            "All {max_attempts} attempts failed{retry_info}. Last error: {last_error}"
        ),
        Some(request_id),
    )
    .into_response()
}

/// 处理 Legacy Completions API (/v1/completions)
/// 将 Prompt 转换为 Chat Message 格式，复用 handle_chat_completions
pub async fn handle_completions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(mut body): Json<Value>,
) -> Response {
    info!(
        "Received /v1/completions or /v1/responses payload: {:?}",
        body
    );

    let is_codex_style = body.get("input").is_some() && body.get("instructions").is_some();

    // 1. Convert Payload to Messages (Shared Chat Format)
    if is_codex_style {
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
                        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                        let content = item.get("content").and_then(|v| v.as_array());
                        let mut text_parts = Vec::new();
                        let mut image_parts: Vec<Value> = Vec::new();

                        if let Some(parts) = content {
                            for part in parts {
                                // 处理文本块
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    text_parts.push(text.to_string());
                                }
                                // [NEW] 处理图像块 (Codex input_image 格式)
                                else if part.get("type").and_then(|v| v.as_str())
                                    == Some("input_image")
                                {
                                    if let Some(image_url) =
                                        part.get("image_url").and_then(|v| v.as_str())
                                    {
                                        image_parts.push(json!({
                                            "type": "image_url",
                                            "image_url": { "url": image_url }
                                        }));
                                        debug!("[Codex] Found input_image: {}", image_url);
                                    }
                                }
                                // [NEW] 兼容标准 OpenAI image_url 格式
                                else if part.get("type").and_then(|v| v.as_str())
                                    == Some("image_url")
                                {
                                    if let Some(url_obj) = part.get("image_url") {
                                        image_parts.push(json!({
                                            "type": "image_url",
                                            "image_url": url_obj.clone()
                                        }));
                                    }
                                }
                            }
                        }

                        // 构造消息内容：如果有图像则使用数组格式
                        if image_parts.is_empty() {
                            messages.push(json!({
                                "role": role,
                                "content": text_parts.join("\n")
                            }));
                        } else {
                            let mut content_blocks: Vec<Value> = Vec::new();
                            if !text_parts.is_empty() {
                                content_blocks.push(json!({
                                    "type": "text",
                                    "text": text_parts.join("\n")
                                }));
                            }
                            content_blocks.extend(image_parts);
                            messages.push(json!({
                                "role": role,
                                "content": content_blocks
                            }));
                        }
                    }
                    "function_call" | "local_shell_call" | "web_search_call" => {
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

                        // Handle native shell calls
                        if item_type == "local_shell_call" {
                            name = "shell";
                            if let Some(action) = item.get("action") {
                                if let Some(exec) = action.get("exec") {
                                    // Map to ShellCommandToolCallParams (string command) or ShellToolCallParams (array command)
                                    // Most LLMs prefer a single string for shell
                                    let mut args_obj = serde_json::Map::new();
                                    if let Some(cmd) = exec.get("command") {
                                        // CRITICAL FIX: The 'shell' tool schema defines 'command' as an ARRAY of strings.
                                        // We MUST pass it as an array, not a joined string, otherwise Gemini rejects with 400 INVALID_ARGUMENT.
                                        let cmd_val = if cmd.is_string() {
                                            json!([cmd]) // Wrap in array
                                        } else {
                                            cmd.clone() // Assume already array
                                        };
                                        args_obj.insert("command".to_string(), cmd_val);
                                    }
                                    if let Some(wd) =
                                        exec.get("working_directory").or(exec.get("workdir"))
                                    {
                                        args_obj.insert("workdir".to_string(), wd.clone());
                                    }
                                    args_str = serde_json::to_string(&args_obj)
                                        .unwrap_or("{}".to_string());
                                }
                            }
                        } else if item_type == "web_search_call" {
                            name = "google_search";
                            if let Some(action) = item.get("action") {
                                let mut args_obj = serde_json::Map::new();
                                if let Some(q) = action.get("query") {
                                    args_obj.insert("query".to_string(), q.clone());
                                }
                                args_str =
                                    serde_json::to_string(&args_obj).unwrap_or("{}".to_string());
                            }
                        }

                        messages.push(json!({
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
                        }));
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let call_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let output = item.get("output");
                        let output_str = if let Some(o) = output {
                            if o.is_string() {
                                o.as_str().unwrap_or_default().to_string()
                            } else if let Some(content) = o.get("content").and_then(|v| v.as_str())
                            {
                                content.to_string()
                            } else {
                                o.to_string()
                            }
                        } else {
                            String::new()
                        };

                        let name = call_id_to_name.get(call_id).cloned().unwrap_or_else(|| {
                            // Fallback: if unknown and we see function_call_output, it's likely "shell" in this context
                            tracing::warn!(
                                "Unknown tool name for call_id {}, defaulting to 'shell'",
                                call_id
                            );
                            "shell".to_string()
                        });

                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "name": name,
                            "content": output_str
                        }));
                    }
                    _ => {}
                }
            }
        }

        if let Some(obj) = body.as_object_mut() {
            obj.insert("messages".to_string(), json!(messages));
        }
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

    // 2. Reuse handle_chat_completions logic (wrapping with custom handler or direct call)
    // Actually, due to SSE handling differences (Codex uses different event format), we replicate the loop here or abstract it.
    // For now, let's replicate the core loop but with Codex specific SSE mapping.

    let mut openai_req: OpenAIRequest = match serde_json::from_value(body.clone()) {
        Ok(req) => req,
        Err(e) => return ProxyError::invalid_request(format!("Invalid request: {e}"))
            .with_request_id(request_id)
            .into_response(),
    };

    // Safety: Inject empty message if needed
    if openai_req.messages.is_empty() {
        openai_req
            .messages
            .push(crate::proxy::mappers::openai::OpenAIMessage {
                role: "user".to_string(),
                content: Some(crate::proxy::mappers::openai::OpenAIContent::String(
                    " ".to_string(),
                )),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
    }

    let upstream = state.upstream.clone();
    let token_manager = state.token_manager;
    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);

    let mut last_error = String::new();

    // [529 RESILIENCE] Separate counter for 529 overload retries
    let mut overload_retry_count: usize = 0;
    #[allow(unused)]  // Reserved for future logging/metrics of overload patterns
    let mut current_overload_account: Option<String> = None;

    // Generate trace ID for logging
    let trace_id: String = rand::Rng::sample_iter(rand::thread_rng(), &rand::distributions::Alphanumeric)
        .take(6)
        .map(char::from)
        .collect::<String>().to_lowercase();

    // Use manual loop for 529 resilience
    let mut attempt: usize = 0;
    loop {
        if attempt >= max_attempts {
            break;
        }
        let mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
            &openai_req.model,
            &*state.custom_mapping.read().await,
            &*state.openai_mapping.read().await,
            &*state.anthropic_mapping.read().await,
            false,  // OpenAI 请求不应用 Claude 家族映射
        );
        // 将 OpenAI 工具转为 Value 数组以便探测联网
        let tools_val: Option<Vec<Value>> = openai_req
            .tools.clone();
        let config = crate::proxy::mappers::common_utils::resolve_request_config(
            &openai_req.model,
            &mapped_model,
            &tools_val,
        );

        let (access_token, project_id, email, _account_id) =
            match token_manager.get_token(&config.request_type, false, None).await {
                Ok(t) => t,
                Err(e) => {
                    return ProxyError::token_error(format!("Token error: {e}"))
                        .with_request_id(request_id)
                        .into_response();
                }
            };

        info!("✓ Using account: {} (type: {})", email, config.request_type);

        let gemini_body = transform_openai_request(&openai_req, &project_id, &mapped_model);

        // [New] 打印转换后的报文 (Gemini Body) 供调试 (Codex 路径)
        if let Ok(body_json) = serde_json::to_string_pretty(&gemini_body) {
            debug!("[Codex-Request] Transformed Gemini Body:\n{}", body_json);
        }

        let list_response = openai_req.stream;
        let method = if list_response {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let query_string = if list_response { Some("alt=sse") } else { None };

        let response = match upstream
            .call_v1_internal(method, &access_token, gemini_body, query_string)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_error.clone_from(&e);
                continue;
            }
        };

        let status = response.status();
        if status.is_success() {
            if list_response {
                use axum::body::Body;

                let gemini_stream = response.bytes_stream();
                let body = if is_codex_style {
                    use crate::proxy::mappers::openai::streaming::create_codex_sse_stream;
                    let s =
                        create_codex_sse_stream(Box::pin(gemini_stream), openai_req.model.clone());
                    Body::from_stream(s)
                } else {
                    use crate::proxy::mappers::openai::streaming::create_legacy_sse_stream;
                    let s =
                        create_legacy_sse_stream(Box::pin(gemini_stream), openai_req.model.clone());
                    Body::from_stream(s)
                };

                return build_sse_response(body, mapped_model.as_str());
            }

            let gemini_resp: Value = match response.json().await {
                Ok(v) => v,
                Err(e) => return ProxyError::parse_error(format!("Parse error: {e}"))
                    .with_request_id(request_id)
                    .into_response(),
            };

            let chat_resp = transform_openai_response(&gemini_resp);

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

            return axum::Json(legacy_resp).with_resolved_model(&mapped_model);
        }

        // Handle errors and retry
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_default();
        last_error = format!("HTTP {status_code}: {error_text}");

        // [529 RESILIENCE] Special handling for 529/503 overloaded errors
        if status_code == 529 || status_code == 503 || status_code == 500 {
            token_manager.mark_rate_limited(&email, status_code, retry_after.as_deref(), &error_text);

            if status_code == 529 || (status_code == 503 && error_text.contains("overloaded")) {
                overload_retry_count += 1;
                let _ = current_overload_account.insert(email.clone());  // Reserved for future metrics

                if overload_retry_count <= MAX_OVERLOAD_RETRIES {
                    let base_delay = OVERLOAD_BASE_DELAY_MS * 2_u64.pow((overload_retry_count - 1).min(5) as u32);
                    let capped_delay = base_delay.min(OVERLOAD_MAX_DELAY_MS);
                    let jittered_delay = apply_jitter(capped_delay);

                    tracing::warn!(
                        "[{}] 🔄 529 Overloaded (Codex) - retry {}/{} in {}ms (account: {}, NOT rotating)",
                        trace_id,
                        overload_retry_count,
                        MAX_OVERLOAD_RETRIES,
                        jittered_delay,
                        email
                    );

                    sleep(Duration::from_millis(jittered_delay)).await;
                    continue;
                }
                tracing::error!(
                    "[{}] ❌ 529 Overloaded (Codex) - exhausted {} retries, giving up",
                    trace_id,
                    MAX_OVERLOAD_RETRIES
                );
            }
        }

        if status_code == 429 || status_code == 403 || status_code == 401 {
            attempt += 1;
            continue;
        }
        return ProxyError::upstream_error(status_code, error_text)
            .with_request_id(request_id)
            .into_response();
    }

    // Include 529 retry info in final error message
    let retry_info = if overload_retry_count > 0 {
        format!(" (including {overload_retry_count} overload retries)")
    } else {
        String::new()
    };

    ProxyError::Overloaded(
        format!("All {max_attempts} attempts failed{retry_info}. Last error: {last_error}"),
        Some(request_id),
    )
    .into_response()
}

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

/// OpenAI Images API: POST /v1/images/generations
/// 处理图像生成请求，转换为 Gemini API 格式
pub async fn handle_images_generations(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Json(body): Json<Value>,
) -> Response {
    // 1. 解析请求参数
    let Some(prompt) = body.get("prompt").and_then(|v| v.as_str()) else {
        return ProxyError::invalid_request("Missing 'prompt' field")
            .with_request_id(request_id)
            .into_response();
    };

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("gemini-3-pro-image");

    let n = body.get("n").and_then(serde_json::Value::as_u64).unwrap_or(1) as usize;

    let size = body
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");

    let response_format = body
        .get("response_format")
        .and_then(|v| v.as_str())
        .unwrap_or("b64_json");

    let quality = body
        .get("quality")
        .and_then(|v| v.as_str())
        .unwrap_or("standard");
    let style = body
        .get("style")
        .and_then(|v| v.as_str())
        .unwrap_or("vivid");

    info!(
        "[Images] Received request: model={}, prompt={:.50}..., n={}, size={}, quality={}, style={}",
        model,
        prompt,
        n,
        size,
        quality,
        style
    );

    // 2. 解析尺寸为宽高比
    let aspect_ratio = match size {
        "1792x768" | "2560x1080" => "21:9", // Ultra-wide
        "1792x1024" | "1920x1080" => "16:9",
        "1024x1792" | "1080x1920" => "9:16",
        "1024x768" | "1280x960" => "4:3",
        "768x1024" | "960x1280" => "3:4",
        _ => "1:1", // 默认 1024x1024
    };

    // Prompt Enhancement
    let mut final_prompt = prompt.to_string();
    if quality == "hd" {
        final_prompt.push_str(", (high quality, highly detailed, 4k resolution, hdr)");
    }
    match style {
        "vivid" => final_prompt.push_str(", (vivid colors, dramatic lighting, rich details)"),
        "natural" => final_prompt.push_str(", (natural lighting, realistic, photorealistic)"),
        _ => {}
    }

    // 3. 获取 Token
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager;

    let (access_token, project_id, email, _account_id) = match token_manager.get_token("image_gen", false, None).await
    {
        Ok(t) => t,
        Err(e) => {
            return ProxyError::token_error(format!("Token error: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    info!("✓ Using account: {} for image generation", email);

    // 4. 并发发送请求 (解决 candidateCount > 1 不支持的问题)
    let mut tasks = Vec::new();

    for _ in 0..n {
        let upstream = upstream.clone();
        let access_token = access_token.clone();
        let project_id = project_id.clone();
        let final_prompt = final_prompt.clone();
        let aspect_ratio = aspect_ratio.to_string();
        let _response_format = response_format.to_string();

        tasks.push(tokio::spawn(async move {
            let gemini_body = json!({
                "project": project_id,
                "requestId": format!("img-{}", uuid::Uuid::new_v4()),
                "model": "gemini-3-pro-image",
                "userAgent": "antigravity",
                "requestType": "image_gen",
                "request": {
                    "contents": [{
                        "role": "user",
                        "parts": [{"text": final_prompt}]
                    }],
                    "generationConfig": {
                        "candidateCount": 1, // 强制单张
                        "imageConfig": {
                            "aspectRatio": aspect_ratio
                        }
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

            match upstream
                .call_v1_internal("generateContent", &access_token, gemini_body, None)
                .await
            {
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

    // 5. 收集结果
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
                                        let mime_type = img
                                            .get("mimeType")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("image/png");
                                        images.push(json!({
                                            "url": format!("data:{};base64,{}", mime_type, data)
                                        }));
                                    } else {
                                        images.push(json!({
                                            "b64_json": data
                                        }));
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

    if images.is_empty() {
        let error_msg = if errors.is_empty() {
            "No images generated".to_string()
        } else {
            errors.join("; ")
        };
        tracing::error!("[Images] All {} requests failed. Errors: {}", n, error_msg);
        return ProxyError::UpstreamError {
            status: 502,
            message: error_msg,
            request_id: Some(request_id),
        }
        .into_response();
    }

    // 部分成功时记录警告
    if !errors.is_empty() {
        tracing::warn!(
            "[Images] Partial success: {} out of {} requests succeeded. Errors: {}",
            images.len(),
            n,
            errors.join("; ")
        );
    }

    tracing::info!(
        "[Images] Successfully generated {} out of {} requested image(s)",
        images.len(),
        n
    );

    // 6. 构建 OpenAI 格式响应
    let openai_response = json!({
        "created": chrono::Utc::now().timestamp(),
        "data": images
    });

    Json(openai_response).into_response()
}

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
    let mut response_format = "b64_json".to_string(); // Default to b64_json for better compatibility with tools handling edits
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

        if name == "image" {
            let data = match field.bytes().await {
                Ok(d) => d,
                Err(e) => return ProxyError::invalid_request(format!("Image read error: {e}"))
                    .with_request_id(request_id)
                    .into_response(),
            };
            image_data = Some(base64::engine::general_purpose::STANDARD.encode(data));
        } else if name == "mask" {
            let data = match field.bytes().await {
                Ok(d) => d,
                Err(e) => return ProxyError::invalid_request(format!("Mask read error: {e}"))
                    .with_request_id(request_id)
                    .into_response(),
            };
            mask_data = Some(base64::engine::general_purpose::STANDARD.encode(data));
        } else if name == "prompt" {
            prompt = match field.text().await {
                Ok(t) => t,
                Err(e) => return ProxyError::invalid_request(format!("Prompt read error: {e}"))
                    .with_request_id(request_id)
                    .into_response(),
            };
        } else if name == "n" {
            if let Ok(val) = field.text().await {
                n = val.parse().unwrap_or(1);
            }
        } else if name == "size" {
            if let Ok(val) = field.text().await {
                size = val;
            }
        } else if name == "response_format" {
            if let Ok(val) = field.text().await {
                response_format = val;
            }
        } else if name == "model" {
            if let Ok(val) = field.text().await {
                if !val.is_empty() {
                    model = val;
                }
            }
        }
    }

    if image_data.is_none() {
        return ProxyError::invalid_request("Missing image")
            .with_request_id(request_id)
            .into_response();
    }
    if prompt.is_empty() {
        return ProxyError::invalid_request("Missing prompt")
            .with_request_id(request_id)
            .into_response();
    }

    tracing::info!(
        "[Images] Edit Request: model={}, prompt={}, n={}, size={}, mask={}, response_format={}",
        model,
        prompt,
        n,
        size,
        mask_data.is_some(),
        response_format
    );

    // FIX: Client Display Issue
    // Cherry Studio (and potentially others) might accept Data URI for generations but display raw text for edits
    // if 'url' format is used with a data-uri.
    // If request asks for 'url' but we are a local proxy, returning b64_json is often safer for correct rendering if the client supports it.
    // However, strictly following spec means 'url' should be 'url'.
    // Let's rely on client requesting the right thing, BUT allow a server-side heuristic:
    // If we simply return b64_json structure even if url was requested? No, that breaks spec.
    // Instead, let's assume successful clients request b64_json.
    // But if users see raw text, it means client defaulted to 'url' or we defaulted to 'url'.
    // Let's keep the log to confirm.

    // 1. 获取 Upstream
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager;
    // Fix: Proper get_token call with correct signature and unwrap (using image_gen quota)
    let (access_token, project_id, _email, _account_id) = match token_manager.get_token("image_gen", false, None).await
    {
        Ok(t) => t,
        Err(e) => {
            return ProxyError::token_error(format!("Token error: {e}"))
                .with_request_id(request_id)
                .into_response();
        }
    };

    // 2. 映射配置
    let mut contents_parts = Vec::new();

    contents_parts.push(json!({
        "text": format!("Edit this image: {}", prompt)
    }));

    if let Some(data) = image_data {
        contents_parts.push(json!({
            "inlineData": {
                "mimeType": "image/png",
                "data": data
            }
        }));
    }

    if let Some(data) = mask_data {
        contents_parts.push(json!({
            "inlineData": {
                "mimeType": "image/png",
                "data": data
            }
        }));
    }

    // 构造 Gemini 内网 API Body (Envelope Structure)
    let gemini_body = json!({
        "project": project_id,
        "requestId": format!("img-edit-{}", uuid::Uuid::new_v4()),
        "model": model,
        "userAgent": "antigravity",
        "requestType": "image_gen",
        "request": {
            "contents": [{
                "role": "user",
                "parts": contents_parts
            }],
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

        tasks.push(tokio::spawn(async move {
            match upstream
                .call_v1_internal("generateContent", &access_token, body, None)
                .await
            {
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
                                        let mime_type = img
                                            .get("mimeType")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("image/png");
                                        images.push(json!({
                                            "url": format!("data:{};base64,{}", mime_type, data)
                                        }));
                                    } else {
                                        images.push(json!({
                                            "b64_json": data
                                        }));
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

    if images.is_empty() {
        let error_msg = if errors.is_empty() {
            "No images generated".to_string()
        } else {
            errors.join("; ")
        };
        tracing::error!(
            "[Images] All {} edit requests failed. Errors: {}",
            n,
            error_msg
        );
        return ProxyError::UpstreamError {
            status: 502,
            message: error_msg,
            request_id: Some(request_id),
        }
        .into_response();
    }

    if !errors.is_empty() {
        tracing::warn!(
            "[Images] Partial success: {} out of {} requests succeeded. Errors: {}",
            images.len(),
            n,
            errors.join("; ")
        );
    }

    tracing::info!(
        "[Images] Successfully generated {} out of {} requested edited image(s)",
        images.len(),
        n
    );

    let openai_response = json!({
        "created": chrono::Utc::now().timestamp(),
        "data": images
    });

    Json(openai_response).into_response()
}

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
