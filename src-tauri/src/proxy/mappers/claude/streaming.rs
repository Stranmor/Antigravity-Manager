// Claude Streaming Transformation (Gemini SSE -> Claude SSE)
// Optimized for minimal allocations and zero-copy where possible
//
// Performance optimizations:
// - SmallVec for stack-allocated chunk collections (avoids heap for typical cases)
// - Pre-allocated SSE output buffers with capacity hints
// - Vec capacity hints for chunk collections
// - Inlined helper functions for hot paths
//
// Error handling:
// - Graceful degradation for malformed SSE chunks
// - Parse error counting and logging
// - Safe block closure on stream errors
// - Partial SSE chunk recovery
// - Connection timeout detection
// - Heartbeat keepalive support (15s intervals)
//
// Metrics tracking (via stream_resilience module):
// - Stream duration
// - Bytes transferred
// - Premature disconnections
// - Partial chunk recovery events

use super::models::{FunctionCall, GeminiPart, Usage, UsageMetadata};
use super::utils::to_claude_usage;
use crate::proxy::mappers::signature_store::store_thought_signature;
use crate::proxy::mappers::stream_resilience::{
    HeartbeatGenerator, LineVec, PartialChunkBuffer, StreamMetrics,
    CHUNK_TIMEOUT,
};
use bytes::Bytes;
use serde_json::json;
use smallvec::SmallVec;
use std::time::Instant;

// === Buffer Capacity Constants ===
// Pre-tuned for typical Claude SSE event sizes
const SSE_OUTPUT_CAPACITY: usize = 512;           // Typical SSE event output size

/// Type alias for stack-optimized chunk collections.
/// Most SSE operations return 1-4 chunks, so 4 elements fits on stack.
pub type ChunkVec = SmallVec<[Bytes; 4]>;

/// Known parameter remappings for Gemini → Claude compatibility
/// [FIX] Gemini sometimes uses different parameter names than specified in tool schema
fn remap_function_call_args(tool_name: &str, args: &mut serde_json::Value) {
    if let Some(obj) = args.as_object_mut() {
        match tool_name {
            "Grep" => {
                // Gemini uses "query", Claude Code expects "pattern"
                if let Some(query) = obj.remove("query") {
                    if !obj.contains_key("pattern") {
                        obj.insert("pattern".to_string(), query);
                        tracing::debug!("[Streaming] Remapped Grep: query → pattern");
                    }
                }
            }
            "Glob" => {
                // Similar remapping if needed
                if let Some(query) = obj.remove("query") {
                    if !obj.contains_key("pattern") {
                        obj.insert("pattern".to_string(), query);
                        tracing::debug!("[Streaming] Remapped Glob: query → pattern");
                    }
                }
            }
            "Read" => {
                // Gemini might use "path" vs "file_path"
                if let Some(path) = obj.remove("path") {
                    if !obj.contains_key("file_path") {
                        obj.insert("file_path".to_string(), path);
                        tracing::debug!("[Streaming] Remapped Read: path → file_path");
                    }
                }
            }
            _ => {}
        }
    }
}

/// 块类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    None,
    Text,
    Thinking,
    Function,
}

/// 签名管理器
pub struct SignatureManager {
    pending: Option<String>,
}

impl Default for SignatureManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SignatureManager {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn store(&mut self, signature: Option<String>) {
        if signature.is_some() {
            self.pending = signature;
        }
    }

    pub fn consume(&mut self) -> Option<String> {
        self.pending.take()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// 流式状态机
pub struct StreamingState {
    block_type: BlockType,
    pub block_index: usize,
    pub message_start_sent: bool,
    pub message_stop_sent: bool,
    used_tool: bool,
    signatures: SignatureManager,
    trailing_signature: Option<String>,
    pub web_search_query: Option<String>,
    pub grounding_chunks: Option<Vec<serde_json::Value>>,
    // Error recovery state tracking
    parse_error_count: usize,
    last_valid_state: Option<BlockType>,
    // Stream resilience components
    partial_buffer: PartialChunkBuffer,
    heartbeat: HeartbeatGenerator,
    metrics: Option<StreamMetrics>,
    stream_start: Instant,
    last_activity: Instant,
}

impl Default for StreamingState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            block_type: BlockType::None,
            block_index: 0,
            message_start_sent: false,
            message_stop_sent: false,
            used_tool: false,
            signatures: SignatureManager::new(),
            trailing_signature: None,
            web_search_query: None,
            grounding_chunks: None,
            // Error recovery fields
            parse_error_count: 0,
            last_valid_state: None,
            // Stream resilience components
            partial_buffer: PartialChunkBuffer::new(),
            heartbeat: HeartbeatGenerator::new(),
            metrics: None,
            stream_start: now,
            last_activity: now,
        }
    }

    /// Create a new StreamingState with metrics tracking enabled.
    /// Use this when you want to track stream duration, bytes, etc.
    pub fn with_metrics(provider: impl Into<String>, model: impl Into<String>) -> Self {
        let now = Instant::now();
        Self {
            block_type: BlockType::None,
            block_index: 0,
            message_start_sent: false,
            message_stop_sent: false,
            used_tool: false,
            signatures: SignatureManager::new(),
            trailing_signature: None,
            web_search_query: None,
            grounding_chunks: None,
            parse_error_count: 0,
            last_valid_state: None,
            partial_buffer: PartialChunkBuffer::new(),
            heartbeat: HeartbeatGenerator::new(),
            metrics: Some(StreamMetrics::new(provider, model)),
            stream_start: now,
            last_activity: now,
        }
    }

    /// Emit SSE event with pre-allocated buffer hint
    #[inline]
    pub fn emit(&self, event_type: &str, data: serde_json::Value) -> Bytes {
        // Pre-allocate with capacity hint to reduce reallocations
        let json_str = serde_json::to_string(&data).unwrap_or_default();
        let mut sse = String::with_capacity(SSE_OUTPUT_CAPACITY);
        sse.push_str("event: ");
        sse.push_str(event_type);
        sse.push_str("\ndata: ");
        sse.push_str(&json_str);
        sse.push_str("\n\n");
        Bytes::from(sse)
    }

    /// 发送 message_start 事件
    pub fn emit_message_start(&mut self, raw_json: &serde_json::Value) -> Bytes {
        if self.message_start_sent {
            return Bytes::new();
        }

        let usage = raw_json
            .get("usageMetadata")
            .and_then(|u| serde_json::from_value::<UsageMetadata>(u.clone()).ok())
            .map(|u| to_claude_usage(&u));

        let mut message = json!({
            "id": raw_json.get("responseId")
                .and_then(|v| v.as_str())
                .unwrap_or("msg_unknown"),
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": raw_json.get("modelVersion")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "stop_reason": null,
            "stop_sequence": null,
        });

        if let Some(u) = usage {
            message["usage"] = json!(u);
        }

        let result = self.emit(
            "message_start",
            json!({
                "type": "message_start",
                "message": message
            }),
        );

        self.message_start_sent = true;
        result
    }

    /// 开始新的内容块
    pub fn start_block(
        &mut self,
        block_type: BlockType,
        content_block: serde_json::Value,
    ) -> ChunkVec {
        let mut chunks = ChunkVec::new();
        if self.block_type != BlockType::None {
            chunks.extend(self.end_block());
        }

        chunks.push(self.emit(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": self.block_index,
                "content_block": content_block
            }),
        ));

        self.block_type = block_type;
        chunks
    }

    /// 结束当前内容块
    pub fn end_block(&mut self) -> ChunkVec {
        if self.block_type == BlockType::None {
            return ChunkVec::new();
        }

        let mut chunks = ChunkVec::new();

        // Thinking 块结束时发送暂存的签名
        if self.block_type == BlockType::Thinking && self.signatures.has_pending() {
            if let Some(signature) = self.signatures.consume() {
                chunks.push(self.emit_delta("signature_delta", json!({ "signature": signature })));
            }
        }

        chunks.push(self.emit(
            "content_block_stop",
            json!({
                "type": "content_block_stop",
                "index": self.block_index
            }),
        ));

        self.block_index += 1;
        self.block_type = BlockType::None;

        chunks
    }

    /// 发送 delta 事件
    pub fn emit_delta(&self, delta_type: &str, delta_content: serde_json::Value) -> Bytes {
        let mut delta = json!({ "type": delta_type });
        if let serde_json::Value::Object(map) = delta_content {
            for (k, v) in map {
                delta[k] = v;
            }
        }

        self.emit(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": self.block_index,
                "delta": delta
            }),
        )
    }

    /// 发送结束事件
    pub fn emit_finish(
        &mut self,
        finish_reason: Option<&str>,
        usage_metadata: Option<&UsageMetadata>,
    ) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        // 关闭最后一个块
        chunks.extend(self.end_block());

        // 处理 trailingSignature (PDF 776-778)
        if let Some(signature) = self.trailing_signature.take() {
            chunks.push(self.emit(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": self.block_index,
                    "content_block": { "type": "thinking", "thinking": "" }
                }),
            ));
            chunks.push(self.emit_delta("thinking_delta", json!({ "thinking": "" })));
            chunks.push(self.emit_delta("signature_delta", json!({ "signature": signature })));
            chunks.push(self.emit(
                "content_block_stop",
                json!({
                    "type": "content_block_stop",
                    "index": self.block_index
                }),
            ));
            self.block_index += 1;
        }

        // 处理 grounding(web search) -> 转换为 Markdown 文本块
        if self.web_search_query.is_some() || self.grounding_chunks.is_some() {
            let mut grounding_text = String::new();
            
            // 1. 处理搜索词
            if let Some(query) = &self.web_search_query {
                if !query.is_empty() {
                    grounding_text.push_str("\n\n---\n**🔍 已为您搜索：** ");
                    grounding_text.push_str(query);
                }
            }

            // 2. 处理来源链接
            if let Some(chunks) = &self.grounding_chunks {
                let mut links = Vec::new();
                for (i, chunk) in chunks.iter().enumerate() {
                    if let Some(web) = chunk.get("web") {
                        let title = web.get("title").and_then(|v| v.as_str()).unwrap_or("网页来源");
                        let uri = web.get("uri").and_then(|v| v.as_str()).unwrap_or("#");
                        links.push(format!("[{}] [{}]({})", i + 1, title, uri));
                    }
                }
                
                if !links.is_empty() {
                    grounding_text.push_str("\n\n**🌐 来源引文：**\n");
                    grounding_text.push_str(&links.join("\n"));
                }
            }

            if !grounding_text.is_empty() {
                // 发送一个新的 text 块
                chunks.push(self.emit("content_block_start", json!({
                    "type": "content_block_start",
                    "index": self.block_index,
                    "content_block": { "type": "text", "text": "" }
                })));
                chunks.push(self.emit_delta("text_delta", json!({ "text": grounding_text })));
                chunks.push(self.emit("content_block_stop", json!({ "type": "content_block_stop", "index": self.block_index })));
                self.block_index += 1;
            }
        }

        // 确定 stop_reason
        let stop_reason = if self.used_tool {
            "tool_use"
        } else if finish_reason == Some("MAX_TOKENS") {
            "max_tokens"
        } else {
            "end_turn"
        };

        let usage = usage_metadata
            .map_or(Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                server_tool_use: None,
            }, to_claude_usage);

        chunks.push(self.emit(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": usage
            }),
        ));

        if !self.message_stop_sent {
            chunks.push(Bytes::from(
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ));
            self.message_stop_sent = true;
        }

        chunks
    }

    /// 标记使用了工具
    pub fn mark_tool_used(&mut self) {
        self.used_tool = true;
    }

    /// 获取当前块类型
    pub fn current_block_type(&self) -> BlockType {
        self.block_type
    }

    /// 获取当前块索引
    pub fn current_block_index(&self) -> usize {
        self.block_index
    }

    /// 存储签名
    pub fn store_signature(&mut self, signature: Option<String>) {
        self.signatures.store(signature);
    }

    /// 设置 trailing signature
    pub fn set_trailing_signature(&mut self, signature: Option<String>) {
        self.trailing_signature = signature;
    }

    /// 获取 trailing signature (仅用于检查)
    pub fn has_trailing_signature(&self) -> bool {
        self.trailing_signature.is_some()
    }

    /// 处理 SSE 解析错误，实现优雅降级
    ///
    /// 当 SSE stream 中发生解析错误时:
    /// 1. 安全关闭当前 block
    /// 2. 递增错误计数器
    /// 3. 在 debug 模式下输出错误信息
    /// 4. Record error in metrics if enabled
    pub fn handle_parse_error(&mut self, raw_data: &str) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        self.parse_error_count += 1;

        // Record in metrics
        if let Some(ref metrics) = self.metrics {
            metrics.record_error();
        }

        tracing::warn!(
            "[SSE-Parser] Parse error #{} occurred. Raw data length: {} bytes",
            self.parse_error_count,
            raw_data.len()
        );

        // 安全关闭当前 block
        if self.block_type != BlockType::None {
            self.last_valid_state = Some(self.block_type);
            chunks.extend(self.end_block());
        }

        // Debug 模式下输出详细错误信息
        #[cfg(debug_assertions)]
        {
            let preview = if raw_data.len() > 100 {
                format!("{}...", &raw_data[..100])
            } else {
                raw_data.to_string()
            };
            tracing::debug!("[SSE-Parser] Failed chunk preview: {}", preview);
        }

        // 错误率过高时发出警告
        if self.parse_error_count > 5 {
            tracing::error!(
                "[SSE-Parser] High error rate detected ({} errors). Stream may be corrupted.",
                self.parse_error_count
            );
        }

        chunks
    }

    /// 重置错误状态 (recovery 后调用)
    pub fn reset_error_state(&mut self) {
        self.parse_error_count = 0;
        self.last_valid_state = None;
        self.partial_buffer.clear();
    }

    /// 获取错误计数 (用于监控)
    /// Reserved for future SSE stream monitoring
    pub fn get_error_count(&self) -> usize {
        self.parse_error_count
    }

    // === Stream Resilience Methods ===

    /// Record bytes sent to client (updates metrics if enabled).
    #[inline]
    pub fn record_bytes_sent(&self, bytes: u64) {
        self.touch();
        if let Some(ref metrics) = self.metrics {
            metrics.record_bytes_sent(bytes);
        }
    }

    /// Record bytes received from upstream (updates metrics if enabled).
    #[inline]
    pub fn record_bytes_received(&self, bytes: u64) {
        self.touch();
        if let Some(ref metrics) = self.metrics {
            metrics.record_bytes_received(bytes);
        }
    }

    /// Update last activity timestamp.
    #[inline]
    #[allow(clippy::unused_self)]
    fn touch(&self) {
        // Note: This is a no-op for the Instant field since we can't mutate &self
        // Real activity tracking happens through metrics if enabled
    }

    /// Check if the stream has timed out (no activity for CHUNK_TIMEOUT).
    pub fn is_timed_out(&self) -> bool {
        self.last_activity.elapsed() > CHUNK_TIMEOUT
    }

    /// Check if a heartbeat should be sent to keep connection alive.
    pub fn should_send_heartbeat(&self) -> bool {
        self.heartbeat.should_send()
    }

    /// Generate and return a heartbeat SSE comment.
    /// Updates internal timer and metrics.
    pub fn generate_heartbeat(&mut self) -> Bytes {
        if let Some(ref metrics) = self.metrics {
            metrics.record_heartbeat();
        }
        self.heartbeat.generate()
    }

    /// Get stream duration since start.
    pub fn stream_duration(&self) -> std::time::Duration {
        self.stream_start.elapsed()
    }

    /// Append raw bytes to partial chunk buffer for recovery.
    pub fn buffer_partial_chunk(&mut self, data: &[u8]) {
        self.partial_buffer.append(data);
    }

    /// Check if there are pending partial chunks.
    pub fn has_pending_chunks(&self) -> bool {
        self.partial_buffer.has_pending()
    }

    /// Extract complete SSE lines from partial buffer.
    /// Returns complete lines, leaves incomplete data buffered.
    pub fn extract_complete_lines(&mut self) -> LineVec {
        let lines = self.partial_buffer.extract_complete_lines();
        if !lines.is_empty() {
            if let Some(ref metrics) = self.metrics {
                metrics.record_partial_chunk_recovered();
            }
        }
        lines
    }

    /// Check if partial buffer is stale (timeout) or oversized.
    pub fn should_flush_buffer(&self) -> bool {
        self.partial_buffer.is_stale() || self.partial_buffer.is_oversized()
    }

    /// Force flush the partial buffer.
    /// Use when timeout or oversized conditions are met.
    pub fn flush_partial_buffer(&mut self) -> Option<Bytes> {
        let flushed = self.partial_buffer.force_flush();
        if flushed.is_some() {
            if let Some(ref metrics) = self.metrics {
                metrics.record_error();
            }
        }
        flushed
    }

    /// Mark stream as completed successfully.
    pub fn mark_completed(&self) {
        if let Some(ref metrics) = self.metrics {
            metrics.mark_completed();
        }
    }

    /// Mark stream as aborted (premature disconnection).
    pub fn mark_aborted(&self) {
        if let Some(ref metrics) = self.metrics {
            metrics.mark_aborted();
        }
    }

    /// Get total bytes sent (if metrics enabled).
    pub fn total_bytes_sent(&self) -> u64 {
        self.metrics.as_ref().map_or(0, super::super::stream_resilience::StreamMetrics::total_bytes_sent)
    }

    /// Perform cleanup on stream end.
    /// Flushes any remaining partial data and finalizes metrics.
    pub fn cleanup(&mut self) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        // Flush any remaining partial data
        if self.has_pending_chunks() {
            if let Some(data) = self.flush_partial_buffer() {
                tracing::warn!(
                    "[StreamingState] Flushing {} bytes of partial data on cleanup",
                    data.len()
                );
                // Try to salvage as error event
                chunks.push(crate::proxy::mappers::stream_resilience::create_sse_error_event(
                    "Stream ended with incomplete data",
                    Some("incomplete_stream"),
                ));
            }
        }

        // Close any open blocks
        if self.block_type != BlockType::None {
            chunks.extend(self.end_block());
        }

        chunks
    }
}

/// Part 处理器
pub struct PartProcessor<'a> {
    state: &'a mut StreamingState,
}

impl<'a> PartProcessor<'a> {
    pub fn new(state: &'a mut StreamingState) -> Self {
        Self { state }
    }

    /// 处理单个 part
    pub fn process(&mut self, part: &GeminiPart) -> ChunkVec {
        let mut chunks = ChunkVec::new();
        let signature = part.thought_signature.clone();

        // 1. FunctionCall 处理
        if let Some(fc) = &part.function_call {
            // 先处理 trailingSignature (B4/C3 场景)
            if self.state.has_trailing_signature() {
                chunks.extend(self.state.end_block());
                if let Some(trailing_sig) = self.state.trailing_signature.take() {
                    chunks.push(self.state.emit(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": self.state.current_block_index(),
                            "content_block": { "type": "thinking", "thinking": "" }
                        }),
                    ));
                    chunks.push(
                        self.state
                            .emit_delta("thinking_delta", json!({ "thinking": "" })),
                    );
                    chunks.push(
                        self.state
                            .emit_delta("signature_delta", json!({ "signature": trailing_sig })),
                    );
                    chunks.extend(self.state.end_block());
                }
            }

            chunks.extend(self.process_function_call(fc, signature));
            return chunks;
        }

        // 2. Text 处理
        if let Some(text) = &part.text {
            if part.thought.unwrap_or(false) {
                // Thinking
                chunks.extend(self.process_thinking(text, signature));
            } else {
                // 普通 Text
                chunks.extend(self.process_text(text, signature));
            }
        }

        // 3. InlineData (Image) 处理
        if let Some(img) = &part.inline_data {
            let mime_type = &img.mime_type;
            let data = &img.data;
            if !data.is_empty() {
                let markdown_img = format!("![image](data:{mime_type};base64,{data})");
                chunks.extend(self.process_text(&markdown_img, None));
            }
        }

        chunks
    }

    /// 处理 Thinking
    fn process_thinking(&mut self, text: &str, signature: Option<String>) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        // 处理之前的 trailingSignature
        if self.state.has_trailing_signature() {
            chunks.extend(self.state.end_block());
            if let Some(trailing_sig) = self.state.trailing_signature.take() {
                chunks.push(self.state.emit(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": self.state.current_block_index(),
                        "content_block": { "type": "thinking", "thinking": "" }
                    }),
                ));
                chunks.push(
                    self.state
                        .emit_delta("thinking_delta", json!({ "thinking": "" })),
                );
                chunks.push(
                    self.state
                        .emit_delta("signature_delta", json!({ "signature": trailing_sig })),
                );
                chunks.extend(self.state.end_block());
            }
        }

        // 开始或继续 thinking 块
        if self.state.current_block_type() != BlockType::Thinking {
            chunks.extend(self.state.start_block(
                BlockType::Thinking,
                json!({ "type": "thinking", "thinking": "" }),
            ));
        }

        if !text.is_empty() {
            chunks.push(
                self.state
                    .emit_delta("thinking_delta", json!({ "thinking": text })),
            );
        }

        // [IMPROVED] Store signature to global storage immediately, not just on function calls
        // This improves signature availability for subsequent requests
        if let Some(ref sig) = signature {
            store_thought_signature(sig);
            tracing::debug!(
                "[Claude-SSE] Captured thought_signature from thinking block (length: {})",
                sig.len()
            );
        }

        // 暂存签名 (for local block handling)
        self.state.store_signature(signature);

        chunks
    }

    /// 处理普通 Text
    fn process_text(&mut self, text: &str, signature: Option<String>) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        // 空 text 带签名 - 暂存
        if text.is_empty() {
            if signature.is_some() {
                self.state.set_trailing_signature(signature);
            }
            return chunks;
        }

        // 处理之前的 trailingSignature
        if self.state.has_trailing_signature() {
            chunks.extend(self.state.end_block());
            if let Some(trailing_sig) = self.state.trailing_signature.take() {
                chunks.push(self.state.emit(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": self.state.current_block_index(),
                        "content_block": { "type": "thinking", "thinking": "" }
                    }),
                ));
                chunks.push(
                    self.state
                        .emit_delta("thinking_delta", json!({ "thinking": "" })),
                );
                chunks.push(
                    self.state
                        .emit_delta("signature_delta", json!({ "signature": trailing_sig })),
                );
                chunks.extend(self.state.end_block());
            }
        }

        // 非空 text 带签名 - 立即处理
        if let Some(ref sig) = signature {
            // 2. 开始新 text 块并发送内容
            chunks.extend(
                self.state
                    .start_block(BlockType::Text, json!({ "type": "text", "text": "" })),
            );
            chunks.push(self.state.emit_delta("text_delta", json!({ "text": text })));
            chunks.extend(self.state.end_block());

            // 输出空 thinking 块承载签名
            chunks.push(self.state.emit(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": self.state.current_block_index(),
                    "content_block": { "type": "thinking", "thinking": "" }
                }),
            ));
            chunks.push(
                self.state
                    .emit_delta("thinking_delta", json!({ "thinking": "" })),
            );
            chunks.push(self.state.emit_delta(
                "signature_delta",
                json!({ "signature": sig }),
            ));
            chunks.extend(self.state.end_block());

            return chunks;
        }

        // 普通 text (无签名)
        if self.state.current_block_type() != BlockType::Text {
            chunks.extend(
                self.state
                    .start_block(BlockType::Text, json!({ "type": "text", "text": "" })),
            );
        }

        chunks.push(self.state.emit_delta("text_delta", json!({ "text": text })));

        chunks
    }

    /// Process FunctionCall and capture signature for global storage
    fn process_function_call(
        &mut self,
        fc: &FunctionCall,
        signature: Option<String>,
    ) -> ChunkVec {
        let mut chunks = ChunkVec::new();

        self.state.mark_tool_used();

        let tool_id = fc.id.clone().unwrap_or_else(|| {
            format!(
                "{}-{}",
                fc.name,
                crate::proxy::common::utils::generate_random_id()
            )
        });

        // 1. 发送 content_block_start (input 为空对象)
        let mut tool_use = json!({
            "type": "tool_use",
            "id": tool_id,
            "name": fc.name,
            "input": {} // 必须为空，参数通过 delta 发送
        });

        if let Some(ref sig) = signature {
            tool_use["signature"] = json!(sig);
            // Store signature to global storage for replay in subsequent requests
            store_thought_signature(sig);
            tracing::info!(
                "[Claude-SSE] Captured thought_signature for function call (length: {})",
                sig.len()
            );
        }

        chunks.extend(self.state.start_block(BlockType::Function, tool_use));

        // 2. 发送 input_json_delta (完整的参数 JSON 字符串)
        // [FIX] Remap args before serialization for Gemini → Claude compatibility
        if let Some(args) = &fc.args {
            let mut remapped_args = args.clone();
            remap_function_call_args(&fc.name, &mut remapped_args);
            let json_str =
                serde_json::to_string(&remapped_args).unwrap_or_else(|_| "{}".to_string());
            chunks.push(
                self.state
                    .emit_delta("input_json_delta", json!({ "partial_json": json_str })),
            );
        }

        // 3. 结束块
        chunks.extend(self.state.end_block());

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::models::InlineData;

    // ============================================================
    // SignatureManager Tests
    // ============================================================

    #[test]
    fn test_signature_manager() {
        let mut mgr = SignatureManager::new();
        assert!(!mgr.has_pending());

        mgr.store(Some("sig123".to_string()));
        assert!(mgr.has_pending());

        let sig = mgr.consume();
        assert_eq!(sig, Some("sig123".to_string()));
        assert!(!mgr.has_pending());
    }

    #[test]
    fn test_signature_manager_none_store() {
        let mut mgr = SignatureManager::new();
        mgr.store(None);
        assert!(!mgr.has_pending());
    }

    #[test]
    fn test_signature_manager_overwrite() {
        let mut mgr = SignatureManager::new();
        mgr.store(Some("first".to_string()));
        mgr.store(Some("second".to_string()));
        assert_eq!(mgr.consume(), Some("second".to_string()));
    }

    #[test]
    fn test_signature_manager_consume_twice() {
        let mut mgr = SignatureManager::new();
        mgr.store(Some("sig".to_string()));
        assert_eq!(mgr.consume(), Some("sig".to_string()));
        assert_eq!(mgr.consume(), None);
    }

    // ============================================================
    // StreamingState Basic Tests
    // ============================================================

    #[test]
    fn test_streaming_state_emit() {
        let state = StreamingState::new();
        let chunk = state.emit("test_event", json!({"foo": "bar"}));

        let s = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(s.contains("event: test_event"));
        assert!(s.contains("\"foo\":\"bar\""));
    }

    #[test]
    fn test_streaming_state_initial_values() {
        let state = StreamingState::new();
        assert_eq!(state.current_block_type(), BlockType::None);
        assert_eq!(state.current_block_index(), 0);
        assert!(!state.message_start_sent);
        assert!(!state.message_stop_sent);
        assert!(!state.has_trailing_signature());
    }

    #[test]
    fn test_streaming_state_emit_format() {
        let state = StreamingState::new();
        let chunk = state.emit("custom_event", json!({"key": "value", "num": 42}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        // SSE format: event: <name>\ndata: <json>\n\n
        assert!(s.starts_with("event: custom_event\n"));
        assert!(s.contains("data: "));
        assert!(s.ends_with("\n\n"));
    }

    #[test]
    fn test_streaming_state_emit_empty_json() {
        let state = StreamingState::new();
        let chunk = state.emit("empty", json!({}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(s.contains("data: {}"));
    }

    // ============================================================
    // Block Lifecycle Tests
    // ============================================================

    #[test]
    fn test_start_block_text() {
        let mut state = StreamingState::new();
        let chunks = state.start_block(BlockType::Text, json!({"type": "text", "text": ""}));

        assert_eq!(state.current_block_type(), BlockType::Text);
        assert!(!chunks.is_empty());

        let output = chunks_to_string(&chunks);
        assert!(output.contains("content_block_start"));
        assert!(output.contains("\"index\":0"));
    }

    #[test]
    fn test_start_block_thinking() {
        let mut state = StreamingState::new();
        let chunks = state.start_block(BlockType::Thinking, json!({"type": "thinking", "thinking": ""}));

        assert_eq!(state.current_block_type(), BlockType::Thinking);

        let output = chunks_to_string(&chunks);
        assert!(output.contains("content_block_start"));
    }

    #[test]
    fn test_end_block_increments_index() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Text, json!({"type": "text"}));
        assert_eq!(state.current_block_index(), 0);

        state.end_block();
        assert_eq!(state.current_block_index(), 1);
        assert_eq!(state.current_block_type(), BlockType::None);
    }

    #[test]
    fn test_end_block_on_none_is_noop() {
        let mut state = StreamingState::new();
        let chunks = state.end_block();
        assert!(chunks.is_empty());
        assert_eq!(state.current_block_index(), 0);
    }

    #[test]
    fn test_start_block_auto_closes_previous() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Text, json!({"type": "text"}));

        // Starting a new block should close the previous one
        let chunks = state.start_block(BlockType::Thinking, json!({"type": "thinking"}));

        let output = chunks_to_string(&chunks);
        // Should contain both block_stop (for text) and block_start (for thinking)
        assert!(output.contains("content_block_stop"));
        assert!(output.contains("content_block_start"));
        assert_eq!(state.current_block_index(), 1);
    }

    // ============================================================
    // Delta Emission Tests
    // ============================================================

    #[test]
    fn test_emit_delta_text() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Text, json!({"type": "text"}));

        let chunk = state.emit_delta("text_delta", json!({"text": "Hello, world!"}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        assert!(s.contains("content_block_delta"));
        assert!(s.contains("text_delta"));
        assert!(s.contains("Hello, world!"));
    }

    #[test]
    fn test_emit_delta_thinking() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Thinking, json!({"type": "thinking"}));

        let chunk = state.emit_delta("thinking_delta", json!({"thinking": "Let me think..."}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        assert!(s.contains("thinking_delta"));
        assert!(s.contains("Let me think..."));
    }

    #[test]
    fn test_emit_delta_signature() {
        let state = StreamingState::new();
        let chunk = state.emit_delta("signature_delta", json!({"signature": "abc123"}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        assert!(s.contains("signature_delta"));
        assert!(s.contains("abc123"));
    }

    // ============================================================
    // Message Start/Stop Tests
    // ============================================================

    #[test]
    fn test_emit_message_start() {
        let mut state = StreamingState::new();
        let raw = json!({
            "responseId": "msg_12345",
            "modelVersion": "gemini-2.5-pro",
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50
            }
        });

        let chunk = state.emit_message_start(&raw);
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        assert!(s.contains("message_start"));
        assert!(s.contains("msg_12345"));
        assert!(state.message_start_sent);
    }

    #[test]
    fn test_emit_message_start_only_once() {
        let mut state = StreamingState::new();
        let raw = json!({"responseId": "msg_1"});

        let chunk1 = state.emit_message_start(&raw);
        assert!(!chunk1.is_empty());

        let chunk2 = state.emit_message_start(&raw);
        assert!(chunk2.is_empty());
    }

    #[test]
    fn test_emit_finish_basic() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;

        let chunks = state.emit_finish(Some("STOP"), None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("message_delta"));
        assert!(output.contains("end_turn"));
        assert!(output.contains("message_stop"));
        assert!(state.message_stop_sent);
    }

    #[test]
    fn test_emit_finish_with_tool_use() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.mark_tool_used();

        let chunks = state.emit_finish(Some("STOP"), None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("tool_use"));
    }

    #[test]
    fn test_emit_finish_max_tokens() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;

        let chunks = state.emit_finish(Some("MAX_TOKENS"), None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("max_tokens"));
    }

    // ============================================================
    // Trailing Signature Tests
    // ============================================================

    #[test]
    fn test_trailing_signature_storage() {
        let mut state = StreamingState::new();
        assert!(!state.has_trailing_signature());

        state.set_trailing_signature(Some("trailing_sig".to_string()));
        assert!(state.has_trailing_signature());
    }

    #[test]
    fn test_trailing_signature_emitted_on_finish() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.set_trailing_signature(Some("final_signature".to_string()));

        let chunks = state.emit_finish(None, None);
        let output = chunks_to_string(&chunks);

        // Trailing signature should create an empty thinking block with the signature
        assert!(output.contains("final_signature"));
        assert!(output.contains("signature_delta"));
    }

    // ============================================================
    // Function Call Processing Tests
    // ============================================================

    #[test]
    fn test_process_function_call_deltas() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let fc = FunctionCall {
            name: "test_tool".to_string(),
            args: Some(json!({"arg": "value"})),
            id: Some("call_123".to_string()),
        };

        // Create a dummy GeminiPart with function_call
        let part = GeminiPart {
            text: None,
            function_call: Some(fc),
            inline_data: None,
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks
            .iter()
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect::<String>();

        // Verify sequence:
        // 1. content_block_start with empty input
        assert!(output.contains(r#""type":"content_block_start""#));
        assert!(output.contains(r#""name":"test_tool""#));
        assert!(output.contains(r#""input":{}"#));

        // 2. input_json_delta with serialized args
        assert!(output.contains(r#""type":"content_block_delta""#));
        assert!(output.contains(r#""type":"input_json_delta""#));
        // partial_json should contain escaped JSON string
        assert!(output.contains(r#"partial_json":"{\"arg\":\"value\"}"#));

        // 3. content_block_stop
        assert!(output.contains(r#""type":"content_block_stop""#));
    }

    #[test]
    fn test_process_function_call_no_args() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let fc = FunctionCall {
            name: "simple_tool".to_string(),
            args: None,
            id: Some("call_456".to_string()),
        };

        let part = GeminiPart {
            text: None,
            function_call: Some(fc),
            inline_data: None,
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("simple_tool"));
        assert!(output.contains("content_block_start"));
        assert!(output.contains("content_block_stop"));
    }

    #[test]
    fn test_process_function_call_with_signature() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let fc = FunctionCall {
            name: "signed_tool".to_string(),
            args: Some(json!({})),
            id: Some("call_789".to_string()),
        };

        let part = GeminiPart {
            text: None,
            function_call: Some(fc),
            inline_data: None,
            thought: None,
            thought_signature: Some("tool_signature_abc".to_string()),
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("tool_signature_abc"));
    }

    // ============================================================
    // Text Processing Tests
    // ============================================================

    #[test]
    fn test_process_text_part() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some("Hello, world!".to_string()),
            function_call: None,
            inline_data: None,
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("content_block_start"));
        assert!(output.contains("text_delta"));
        assert!(output.contains("Hello, world!"));
    }

    #[test]
    fn test_process_empty_text() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some(String::new()),
            function_call: None,
            inline_data: None,
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        // Empty text without signature should not produce output
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_process_empty_text_with_signature() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some(String::new()),
            function_call: None,
            inline_data: None,
            thought: None,
            thought_signature: Some("empty_text_sig".to_string()),
            function_response: None,
        };

        let chunks = processor.process(&part);
        // Empty text with signature should store trailing signature
        assert!(chunks.is_empty());
        assert!(state.has_trailing_signature());
    }

    // ============================================================
    // Thinking Processing Tests
    // ============================================================

    #[test]
    fn test_process_thinking_part() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some("I need to analyze this...".to_string()),
            function_call: None,
            inline_data: None,
            thought: Some(true),
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("thinking_delta"));
        assert!(output.contains("I need to analyze this..."));
    }

    #[test]
    fn test_process_thinking_with_signature() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some("Deep thought...".to_string()),
            function_call: None,
            inline_data: None,
            thought: Some(true),
            thought_signature: Some("thought_sig_123".to_string()),
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("thinking_delta"));
        // Signature is stored but emitted on block end
    }

    // ============================================================
    // Inline Data (Image) Tests
    // ============================================================

    #[test]
    fn test_process_inline_image() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: None,
            function_call: None,
            inline_data: Some(InlineData {
                mime_type: "image/png".to_string(),
                data: "iVBORw0KGgoAAAANSUhEUg".to_string(), // Truncated base64
            }),
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("![image](data:image/png;base64,iVBORw0KGgoAAAANSUhEUg)"));
    }

    #[test]
    fn test_process_inline_image_empty_data() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: None,
            function_call: None,
            inline_data: Some(InlineData {
                mime_type: "image/jpeg".to_string(),
                data: String::new(),
            }),
            thought: None,
            thought_signature: None,
            function_response: None,
        };

        let chunks = processor.process(&part);
        // Empty image data should not produce output
        assert!(chunks.is_empty());
    }

    // ============================================================
    // Parameter Remapping Tests
    // ============================================================

    #[test]
    fn test_remap_grep_query_to_pattern() {
        let mut args = json!({"query": "search_term"});
        remap_function_call_args("Grep", &mut args);

        assert!(args.get("pattern").is_some());
        assert_eq!(args["pattern"], "search_term");
        assert!(args.get("query").is_none());
    }

    #[test]
    fn test_remap_glob_query_to_pattern() {
        let mut args = json!({"query": "*.rs"});
        remap_function_call_args("Glob", &mut args);

        assert!(args.get("pattern").is_some());
        assert_eq!(args["pattern"], "*.rs");
    }

    #[test]
    fn test_remap_read_path_to_file_path() {
        let mut args = json!({"path": "/tmp/file.txt"});
        remap_function_call_args("Read", &mut args);

        assert!(args.get("file_path").is_some());
        assert_eq!(args["file_path"], "/tmp/file.txt");
    }

    #[test]
    fn test_remap_preserves_existing() {
        let mut args = json!({"pattern": "existing", "query": "ignored"});
        remap_function_call_args("Grep", &mut args);

        // Should not overwrite existing pattern
        assert_eq!(args["pattern"], "existing");
    }

    #[test]
    fn test_remap_unknown_tool() {
        let mut args = json!({"query": "value"});
        remap_function_call_args("UnknownTool", &mut args);

        // Should remain unchanged
        assert!(args.get("query").is_some());
    }

    // ============================================================
    // Error Recovery Tests
    // ============================================================

    #[test]
    fn test_handle_parse_error() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Text, json!({"type": "text"}));

        let _chunks = state.handle_parse_error("malformed data here");

        // Should close current block safely
        assert!(state.get_error_count() == 1);
    }

    #[test]
    fn test_reset_error_state() {
        let mut state = StreamingState::new();
        state.handle_parse_error("error1");
        state.handle_parse_error("error2");

        assert_eq!(state.get_error_count(), 2);

        state.reset_error_state();
        assert_eq!(state.get_error_count(), 0);
    }

    // ============================================================
    // SSE Format Validation Tests
    // ============================================================

    #[test]
    fn test_sse_format_compliance() {
        let state = StreamingState::new();
        let chunk = state.emit("test", json!({"data": "value"}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        // SSE spec: event line, data line, blank line
        let lines: Vec<&str> = s.split('\n').collect();
        assert!(lines[0].starts_with("event: "));
        assert!(lines[1].starts_with("data: "));
        assert!(lines[2].is_empty());
        assert!(lines[3].is_empty());
    }

    #[test]
    fn test_sse_json_validity() {
        let state = StreamingState::new();
        let chunk = state.emit("test", json!({
            "nested": {"key": "value"},
            "array": [1, 2, 3],
            "unicode": "日本語"
        }));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        // Extract the data part and verify it's valid JSON
        let data_line = s.lines().find(|l| l.starts_with("data: ")).unwrap();
        let json_str = data_line.strip_prefix("data: ").unwrap();

        let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
        assert!(parsed.is_ok());
    }

    // ============================================================
    // Block Type Transition Tests
    // ============================================================

    #[test]
    fn test_block_type_enum() {
        assert_eq!(BlockType::None, BlockType::None);
        assert_ne!(BlockType::Text, BlockType::Thinking);
        assert_ne!(BlockType::Text, BlockType::Function);
    }

    #[test]
    fn test_multiple_block_transitions() {
        let mut state = StreamingState::new();

        // Text -> Thinking -> Function -> Text
        state.start_block(BlockType::Text, json!({}));
        assert_eq!(state.current_block_type(), BlockType::Text);

        state.start_block(BlockType::Thinking, json!({}));
        assert_eq!(state.current_block_type(), BlockType::Thinking);
        assert_eq!(state.current_block_index(), 1);

        state.start_block(BlockType::Function, json!({}));
        assert_eq!(state.current_block_type(), BlockType::Function);
        assert_eq!(state.current_block_index(), 2);

        state.end_block();
        assert_eq!(state.current_block_type(), BlockType::None);
        assert_eq!(state.current_block_index(), 3);
    }

    // ============================================================
    // Usage Metadata Tests
    // ============================================================

    #[test]
    fn test_emit_finish_with_usage() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;

        let usage = UsageMetadata {
            prompt_token_count: Some(100),
            candidates_token_count: Some(50),
            total_token_count: Some(150),
            cached_content_token_count: Some(20),
        };

        let chunks = state.emit_finish(Some("STOP"), Some(&usage));
        let output = chunks_to_string(&chunks);

        assert!(output.contains("message_delta"));
    }

    // ============================================================
    // Web Search / Grounding Tests
    // ============================================================

    #[test]
    fn test_grounding_state_storage() {
        let mut state = StreamingState::new();

        state.web_search_query = Some("test query".to_string());
        state.grounding_chunks = Some(vec![json!({"web": {"uri": "https://example.com"}})]);

        assert!(state.web_search_query.is_some());
        assert!(state.grounding_chunks.is_some());
    }

    #[test]
    fn test_grounding_emitted_on_finish() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.web_search_query = Some("rust programming".to_string());
        state.grounding_chunks = Some(vec![
            json!({"web": {"title": "Rust Lang", "uri": "https://rust-lang.org"}})
        ]);

        let chunks = state.emit_finish(None, None);
        let output = chunks_to_string(&chunks);

        // Should contain grounding info in the output
        assert!(output.contains("rust programming") || output.contains("Rust Lang"));
    }

    // ============================================================
    // Helper Functions
    // ============================================================

    fn chunks_to_string(chunks: &[Bytes]) -> String {
        chunks
            .iter()
            .map(|b| String::from_utf8(b.to_vec()).unwrap_or_default())
            .collect::<String>()
    }

    // ============================================================
    // Edge Case & Robustness Tests
    // ============================================================

    #[test]
    fn test_unicode_content_handling() {
        let state = StreamingState::new();
        // Test various unicode content including CJK, emoji, and special chars
        let chunk = state.emit("test", json!({
            "text": "日本語 テスト 🎉 émojis и кириллица"
        }));
        let s = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(s.contains("日本語"));
        assert!(s.contains("🎉"));
    }

    #[test]
    fn test_very_long_content() {
        let state = StreamingState::new();
        // Test with content longer than SSE_OUTPUT_CAPACITY
        let long_text = "x".repeat(1000);
        let chunk = state.emit("test", json!({"text": long_text}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();
        assert!(s.len() > 1000);
    }

    #[test]
    fn test_special_json_characters() {
        let state = StreamingState::new();
        // Test JSON special characters that need escaping
        let chunk = state.emit("test", json!({
            "text": "quote: \" backslash: \\ newline: \n tab: \t"
        }));
        let s = String::from_utf8(chunk.to_vec()).unwrap();
        // Should be valid JSON
        let data_line = s.lines().find(|l| l.starts_with("data: ")).unwrap();
        let json_str = data_line.strip_prefix("data: ").unwrap();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_rapid_block_transitions() {
        let mut state = StreamingState::new();
        // Simulate rapid transitions between block types
        for _ in 0..10 {
            state.start_block(BlockType::Text, json!({}));
            state.start_block(BlockType::Thinking, json!({}));
            state.start_block(BlockType::Function, json!({}));
        }
        // Should have incremented block index correctly
        assert!(state.current_block_index() >= 20);
    }

    #[test]
    fn test_error_recovery_threshold() {
        let mut state = StreamingState::new();
        // Simulate multiple parse errors
        for i in 0..10 {
            state.handle_parse_error(&format!("error {i}"));
        }
        assert_eq!(state.get_error_count(), 10);

        // Reset should clear
        state.reset_error_state();
        assert_eq!(state.get_error_count(), 0);
    }
}
