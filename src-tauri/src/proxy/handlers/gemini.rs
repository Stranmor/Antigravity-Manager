// Gemini Handler
use axum::{extract::State, extract::{Json, Path}, response::IntoResponse, Extension, http::HeaderMap};
use serde_json::{json, Value};
use tracing::{debug, error, info};

use crate::proxy::mappers::gemini::{wrap_request, unwrap_response};
use crate::proxy::server::AppState;
use crate::proxy::session_manager::SessionManager;
use crate::proxy::handlers::common::WithResolvedModel;
use crate::proxy::error::ProxyError;
use crate::proxy::middleware::request_id::RequestId;
use crate::proxy::common::retry::MAX_RETRY_ATTEMPTS;
use crate::proxy::common::coalescing::{calculate_gemini_fingerprint, CoalesceResult};
 
/// 处理 generateContent 和 streamGenerateContent
/// 路径参数: model_name, method (e.g. "gemini-pro", "generateContent")
pub async fn handle_generate(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    Path(model_action): Path<String>,
    Json(body): Json<Value>
) -> Result<impl IntoResponse, ProxyError> {
    // 解析 model:method
    let (model_name, method) = if let Some((m, action)) = model_action.rsplit_once(':') {
        (m.to_string(), action.to_string())
    } else {
        (model_action, "generateContent".to_string())
    };

    crate::modules::logger::log_info(&format!("Received Gemini request: {model_name}/{method}"));

    // 1. 验证方法
    if method != "generateContent" && method != "streamGenerateContent" {
        return Err(ProxyError::invalid_request(format!("Unsupported method: {method}"))
            .with_request_id(request_id));
    }
    let is_stream = method == "streamGenerateContent";

    // [COALESCING] Try to deduplicate identical concurrent non-streaming requests
    let coalesce_sender = if !is_stream && state.coalescer.is_enabled() {
        let fingerprint = calculate_gemini_fingerprint(&body);
        match state.coalescer.get_or_create(fingerprint) {
            CoalesceResult::Primary(s) => Some(s),
            CoalesceResult::Coalesced(r) => {
                debug!("[{}] Identical Gemini request in-flight, waiting for coalesced result...", request_id.as_str());
                match r.recv().await {
                    Ok(shared_response) => {
                        info!("[{}] Received coalesced result from primary Gemini request", request_id.as_str());
                        return Ok(Json((*shared_response).clone()).into_response());
                    }
                    Err(e) => {
                        tracing::warn!("[{}] Coalesced Gemini request failed ({e}), falling back to individual execution", request_id.as_str());
                        None
                    }
                }
            }
        }
    } else {
        None
    };

    // 2. 获取 UpstreamClient 和 TokenManager
    let upstream = state.upstream.clone();
    let token_manager = state.token_manager;
    let pool_size = token_manager.len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(pool_size).max(1);
    
    let mut last_error = String::new();

    for attempt in 0..max_attempts {
        // 3. 模型路由与配置解析
        let mapped_model = crate::proxy::common::model_mapping::resolve_model_route(
            &model_name,
            &*state.custom_mapping.read().await,
            &*state.openai_mapping.read().await,
            &*state.anthropic_mapping.read().await,
            false,  // Gemini 请求不应用 Claude 家族映射
        );

        // [SCHEDULING] Classify request priority (observability only when scheduler enabled)
        if state.scheduler.is_enabled() {
            let x_priority = headers.get("x-priority").and_then(|h| h.to_str().ok());
            let priority = state.scheduler.classify_priority(x_priority, None, Some(&mapped_model));
            debug!("[{}] Request classified as priority: {}", request_id.as_str(), priority);
        }

        // 提取 tools 列表以进行联网探测 (Gemini 风格可能是嵌套的)
        let tools_val: Option<Vec<Value>> = body.get("tools").and_then(|t| t.as_array()).map(|arr| {
            let mut flattened = Vec::new();
            for tool_entry in arr {
                if let Some(decls) = tool_entry.get("functionDeclarations").and_then(|v| v.as_array()) {
                    flattened.extend(decls.iter().cloned());
                } else {
                    flattened.push(tool_entry.clone());
                }
            }
            flattened
        });

        let config = crate::proxy::mappers::common_utils::resolve_request_config(&model_name, &mapped_model, &tools_val);

        // 4. 获取 Token (使用准确的 request_type)
        // 提取 SessionId (粘性指纹)
        let session_id = SessionManager::extract_gemini_session_id(&body, &model_name);

        // 关键：在重试尝试 (attempt > 0) 时强制轮换账号
        let (access_token, project_id, email, account_id) = match token_manager.get_token(&config.request_type, attempt > 0, Some(&session_id)).await {
            Ok(t) => t,
            Err(e) => {
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return Err(ProxyError::token_error(format!("Token error: {e}"))
                    .with_request_id(request_id));
            }
        };

        info!("✓ Using account: {} (type: {})", email, config.request_type);

        // 5. 包装请求 (project injection)
        let wrapped_body = wrap_request(&body, &project_id, &mapped_model);

        // 5. 上游调用
        let query_string = if is_stream { Some("alt=sse") } else { None };
        let upstream_method = if is_stream { "streamGenerateContent" } else { "generateContent" };

        let response = match upstream
            .call_v1_internal(upstream_method, &access_token, wrapped_body, query_string, state.request_timeout)
            .await {
                Ok(r) => r,
                Err(e) => {
                    last_error.clone_from(&e);
                    debug!("Gemini Request failed on attempt {}/{}: {}", attempt + 1, max_attempts, e);
                    continue;
                }
            };

        let status = response.status();
        if status.is_success() {
            // Record success in health monitor
            state.health_monitor.record_success(&account_id);

            // 6. 响应处理
            if is_stream {
                use axum::body::Body;
                use axum::response::Response;
                use bytes::{Bytes, BytesMut};
                use futures::StreamExt;
                use memchr::memchr;

                let mut response_stream = response.bytes_stream();
                let mut buffer = BytesMut::with_capacity(8192); // Pre-allocate 8KB buffer

                let stream = async_stream::stream! {
                    while let Some(item) = response_stream.next().await {
                        match item {
                            Ok(bytes) => {
                                debug!("[Gemini-SSE] Received chunk: {} bytes", bytes.len());
                                buffer.extend_from_slice(&bytes);
                                // Use SIMD-accelerated memchr for faster newline detection
                                while let Some(pos) = memchr(b'\n', &buffer) {
                                    let line_raw = buffer.split_to(pos + 1);
                                    if let Ok(line_str) = std::str::from_utf8(&line_raw) {
                                        let line = line_str.trim();
                                        if line.is_empty() { continue; }
                                        
                                        if line.starts_with("data: ") {
                                            let json_part = line.trim_start_matches("data: ").trim();
                                            if json_part == "[DONE]" {
                                                yield Ok::<Bytes, String>(Bytes::from("data: [DONE]\n\n"));
                                                continue;
                                            }
                                            
                                            match serde_json::from_str::<Value>(json_part) {
                                                Ok(mut json) => {
                                                    // Unwrap v1internal response wrapper
                                                    if let Some(inner) = json.get_mut("response").map(serde_json::Value::take) {
                                                        let new_line = format!("data: {}\n\n", serde_json::to_string(&inner).unwrap_or_default());
                                                        yield Ok::<Bytes, String>(Bytes::from(new_line));
                                                    } else {
                                                        yield Ok::<Bytes, String>(Bytes::from(format!("data: {}\n\n", serde_json::to_string(&json).unwrap_or_default())));
                                                    }
                                                }
                                                Err(e) => {
                                                    debug!("[Gemini-SSE] JSON parse error: {}, passing raw line", e);
                                                    yield Ok::<Bytes, String>(Bytes::from(format!("{line}\n\n")));
                                                }
                                            }
                                        } else {
                                            // Non-data lines (comments, etc.)
                                            yield Ok::<Bytes, String>(Bytes::from(format!("{line}\n\n")));
                                        }
                                    } else {
                                        // Non-UTF8 data? Just pass it through or skip
                                        debug!("[Gemini-SSE] Non-UTF8 line encountered");
                                        yield Ok::<Bytes, String>(line_raw.freeze());
                                    }
                                }
                            }
                            Err(e) => {
                                error!("[Gemini-SSE] Connection error: {}", e);
                                yield Err(format!("Stream error: {e}"));
                            }
                        }
                    }
                };
                
                let body = Body::from_stream(stream);
                let sse_response = Response::builder()
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .header("Connection", "keep-alive")
                    .header(crate::proxy::middleware::monitor::X_RESOLVED_MODEL_HEADER, mapped_model.as_str())
                    .body(body)
                    .map_err(|e| {
                        tracing::error!("Failed to build SSE response: {}", e);
                        ProxyError::response_build_error(format!("SSE response build failed: {e}"))
                            .with_request_id(request_id.clone())
                    })?;

                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return Ok(sse_response.into_response());
            }

            let gemini_resp: Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    if let Some(sender) = coalesce_sender {
                        sender.fail();
                    }
                    return Err(ProxyError::parse_error(format!("Parse error: {e}")).with_request_id(request_id.clone()));
                }
            };

            let unwrapped = unwrap_response(&gemini_resp);

            // [COALESCING] Broadcast successful response
            if let Some(sender) = coalesce_sender {
                sender.send(unwrapped.clone());
            }

            return Ok(Json(unwrapped).with_resolved_model(&mapped_model));
        }

        // 处理错误并重试
        let status_code = status.as_u16();
        let retry_after = response.headers().get("Retry-After").and_then(|h| h.to_str().ok()).map(std::string::ToString::to_string);
        let error_text = response.text().await.unwrap_or_else(|_| format!("HTTP {status_code}"));
        last_error = format!("HTTP {status_code}: {error_text}");
 
        // 只有 429 (限流), 529 (过载), 503, 403 (权限) 和 401 (认证失效) 触发账号轮换
        if status_code == 429 || status_code == 529 || status_code == 503 || status_code == 500 || status_code == 403 || status_code == 401 {
            // Record error in health monitor for 403/401 (may auto-disable account)
            if status_code == 403 || status_code == 401 {
                state.health_monitor.record_error(&account_id, status_code, &error_text).await;
            }

            // 记录限流信息并自动解绑会话 (使用 account_id 而非 email)
            token_manager.mark_rate_limited_and_unbind(&account_id, status_code, retry_after.as_deref(), &error_text, Some(&config.request_type));

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

            // 只有明确包含 "QUOTA_EXHAUSTED" 才停止，避免误判上游的频率限制提示 (如 "check quota")
            if status_code == 429 && error_text.contains("QUOTA_EXHAUSTED") {
                error!("Gemini Quota exhausted (429) on account {} attempt {}/{}, stopping to protect pool.", email, attempt + 1, max_attempts);
                if let Some(sender) = coalesce_sender {
                    sender.fail();
                }
                return Err(ProxyError::RateLimited(error_text, Some(request_id)));
            }

            tracing::warn!("Gemini Upstream {} on account {} attempt {}/{}, rotating account", status_code, email, attempt + 1, max_attempts);
            continue;
        }
 
        // 404 等由于模型配置或路径错误的 HTTP 异常，直接报错，不进行无效轮换
        error!("Gemini Upstream non-retryable error {}: {}", status_code, error_text);
        if let Some(sender) = coalesce_sender {
            sender.fail();
        }
        return Err(ProxyError::upstream_error(status_code, error_text).with_request_id(request_id));
    }

    if let Some(sender) = coalesce_sender {
        sender.fail();
    }

    Err(ProxyError::Overloaded(
        format!("All {max_attempts} attempts failed. Last error: {last_error}"),
        Some(request_id),
    ))
}

pub async fn handle_list_models(State(state): State<AppState>) -> Result<impl IntoResponse, ProxyError> {
    use crate::proxy::common::model_mapping::get_all_dynamic_models;

    // 获取所有动态模型列表（与 /v1/models 一致）
    let model_ids = get_all_dynamic_models(
        &state.openai_mapping,
        &state.custom_mapping,
        &state.anthropic_mapping,
    ).await;

    // 转换为 Gemini API 格式
    let models: Vec<_> = model_ids.into_iter().map(|id| {
        json!({
            "name": format!("models/{}", id),
            "version": "001",
            "displayName": id.clone(),
            "description": "",
            "inputTokenLimit": 128000,
            "outputTokenLimit": 8192,
            "supportedGenerationMethods": ["generateContent", "countTokens"],
            "temperature": 1.0,
            "topP": 0.95,
            "topK": 64
        })
    }).collect();

    Ok(Json(json!({ "models": models })))
}

pub async fn handle_get_model(Path(model_name): Path<String>) -> impl IntoResponse {
    Json(json!({
        "name": format!("models/{}", model_name),
        "displayName": model_name
    }))
}

pub async fn handle_count_tokens(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(_model_name): Path<String>,
    Json(_body): Json<Value>,
) -> Result<impl IntoResponse, ProxyError> {
    let model_group = "gemini";
    let (_access_token, _project_id, _, _account_id) = state
        .token_manager
        .get_token(model_group, false, None)
        .await
        .map_err(|e| ProxyError::token_error(format!("Token error: {e}")).with_request_id(request_id))?;

    Ok(Json(json!({"totalTokens": 0})))
}
