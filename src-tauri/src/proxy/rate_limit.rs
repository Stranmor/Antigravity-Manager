use dashmap::DashMap;
use regex::Regex;
use std::time::{Duration, SystemTime};

// [OPTIMIZATION] Pre-compiled static regex patterns
// Previous: Compiled on every call with Regex::new().ok()
// Current: Compiled once at startup, reused for all parsing
// Benefit: ~100x faster regex matching on hot paths

/// Duration string parser: supports "2h1m1s", "1h30m", "5m", "30s", "500ms"
static DURATION_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?:(\d+)h)?(?:(\d+)m)?(?:(\d+(?:\.\d+)?)s)?(?:(\d+)ms)?")
        .expect("DURATION_REGEX: Invalid regex pattern - this is a compile-time constant")
});

/// Patterns for parsing retry times from error messages
static RETRY_MIN_SEC_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)try again in (\d+)m\s*(\d+)s")
        .expect("RETRY_MIN_SEC_REGEX: Invalid regex pattern - this is a compile-time constant")
});

static RETRY_SEC_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)(?:try again in|backoff for|wait)\s*(\d+)s")
        .expect("RETRY_SEC_REGEX: Invalid regex pattern - this is a compile-time constant")
});

static QUOTA_RESET_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)quota will reset in (\d+) second")
        .expect("QUOTA_RESET_REGEX: Invalid regex pattern - this is a compile-time constant")
});

static RETRY_AFTER_SEC_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)retry after (\d+) second")
        .expect("RETRY_AFTER_SEC_REGEX: Invalid regex pattern - this is a compile-time constant")
});

static WAIT_PAREN_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"\(wait (\d+)s\)")
        .expect("WAIT_PAREN_REGEX: Invalid regex pattern - this is a compile-time constant")
});

/// 限流原因类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateLimitReason {
    /// 配额耗尽 (QUOTA_EXHAUSTED)
    QuotaExhausted,
    /// 速率限制 (RATE_LIMIT_EXCEEDED)
    RateLimitExceeded,
    /// 服务器错误 (5xx)
    ServerError,
    /// 未知原因
    Unknown,
}

/// 限流信息
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// 限流重置时间
    pub reset_time: SystemTime,
    /// 重试间隔(秒)
    #[allow(dead_code)]
    pub retry_after_sec: u64,
    /// 检测时间
    #[allow(dead_code)]
    pub detected_at: SystemTime,
    /// 限流原因 (logged during detection, reserved for future adaptive retry logic)
    #[allow(dead_code)]
    pub reason: RateLimitReason,
}

/// 限流跟踪器
///
/// [IMPROVEMENT] 支持 per-model rate limiting
/// Key format: "{account_id}" 或 "{account_id}:{quota_group}"
/// 这允许同一账号的 Claude 和 Gemini 配额独立跟踪
pub struct RateLimitTracker {
    limits: DashMap<String, RateLimitInfo>,
}

impl RateLimitTracker {
    /// 生成带 quota_group 的限流键
    /// 如果 quota_group 为空或 "gemini"，使用纯 account_id（向后兼容）
    /// 否则使用 "{account_id}:{quota_group}" 格式
    #[inline]
    pub fn make_key(account_id: &str, quota_group: Option<&str>) -> String {
        match quota_group {
            Some(group) if !group.is_empty() && group != "gemini" => {
                format!("{account_id}:{group}")
            }
            _ => account_id.to_string(),
        }
    }
}

impl RateLimitTracker {
    pub fn new() -> Self {
        Self {
            limits: DashMap::new(),
        }
    }

    /// 获取账号剩余的等待时间(秒)
    pub fn get_remaining_wait(&self, account_id: &str) -> u64 {
        if let Some(info) = self.limits.get(account_id) {
            let now = SystemTime::now();
            if info.reset_time > now {
                return info
                    .reset_time
                    .duration_since(now)
                    .unwrap_or(Duration::from_secs(0))
                    .as_secs();
            }
        }
        0
    }

    /// 从错误响应解析限流信息
    ///
    /// # Arguments
    /// * `account_id` - 账号 ID
    /// * `status` - HTTP 状态码
    /// * `retry_after_header` - Retry-After header 值
    /// * `body` - 错误响应 body
    /// * `quota_group` - 可选的配额组 (e.g., "claude", "agent", "gemini")
    ///   用于 per-model rate limiting
    #[allow(dead_code)]
    pub fn parse_from_error(
        &self,
        account_id: &str,
        status: u16,
        retry_after_header: Option<&str>,
        body: &str,
    ) -> Option<RateLimitInfo> {
        self.parse_from_error_with_group(account_id, status, retry_after_header, body, None)
    }

    /// 从错误响应解析限流信息（带配额组支持）
    pub fn parse_from_error_with_group(
        &self,
        account_id: &str,
        status: u16,
        retry_after_header: Option<&str>,
        body: &str,
        quota_group: Option<&str>,
    ) -> Option<RateLimitInfo> {
        let key = Self::make_key(account_id, quota_group);
        // 支持 429 (限流) 以及 500/503/529 (后端故障软避让)
        if status != 429 && status != 500 && status != 503 && status != 529 {
            return None;
        }

        // 1. 解析限流原因类型
        let reason = if status == 429 {
            Self::parse_rate_limit_reason(body)
        } else {
            RateLimitReason::ServerError
        };

        let mut retry_after_sec = None;

        // 2. 从 Retry-After header 提取
        if let Some(retry_after) = retry_after_header {
            if let Ok(seconds) = retry_after.parse::<u64>() {
                retry_after_sec = Some(seconds);
            }
        }

        // 3. 从错误消息提取 (优先尝试 JSON 解析，再试正则)
        if retry_after_sec.is_none() {
            retry_after_sec = Self::parse_retry_time_from_body(body);
        }

        // 4. 处理默认值与软避让逻辑（根据限流类型设置不同默认值）
        let retry_sec = match retry_after_sec {
            Some(s) => {
                // 引入 PR #28 的安全缓冲区：最小 2 秒，防止极高频无效重试
                if s < 2 {
                    2
                } else {
                    s
                }
            }
            None => {
                match reason {
                    RateLimitReason::QuotaExhausted => {
                        // 配额耗尽：使用较长的默认值（1小时），避免频繁重试
                        tracing::warn!(
                            "检测到配额耗尽 (QUOTA_EXHAUSTED)，使用默认值 3600秒 (1小时)"
                        );
                        3600
                    }
                    RateLimitReason::RateLimitExceeded => {
                        // 速率限制：使用较短的默认值（30秒），可以较快恢复
                        tracing::debug!("检测到速率限制 (RATE_LIMIT_EXCEEDED)，使用默认值 30秒");
                        30
                    }
                    RateLimitReason::ServerError => {
                        // 服务器错误：执行"软避让"，默认锁定 20 秒
                        tracing::warn!("检测到 5xx 错误 ({}), 执行 20s 软避让...", status);
                        20
                    }
                    RateLimitReason::Unknown => {
                        // 未知原因：使用中等默认值（60秒）
                        tracing::debug!("无法解析 429 限流原因, 使用默认值 60秒");
                        60
                    }
                }
            }
        };

        let info = RateLimitInfo {
            reset_time: SystemTime::now() + Duration::from_secs(retry_sec),
            retry_after_sec: retry_sec,
            detected_at: SystemTime::now(),
            reason,
        };

        // 存储 (使用 key 而非 account_id，支持 per-model rate limiting)
        self.limits.insert(key.clone(), info.clone());

        tracing::warn!(
            "账号 {} [{}] 限流类型: {:?}, 重置延时: {}秒 (key: {})",
            account_id,
            status,
            reason,
            retry_sec,
            key
        );

        Some(info)
    }

    /// 解析限流原因类型
    fn parse_rate_limit_reason(body: &str) -> RateLimitReason {
        // 尝试从 JSON 中提取 reason 字段
        let trimmed = body.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(reason_str) = json
                    .get("error")
                    .and_then(|e| e.get("details"))
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("reason"))
                    .and_then(|v| v.as_str())
                {
                    return match reason_str {
                        "QUOTA_EXHAUSTED" => RateLimitReason::QuotaExhausted,
                        "RATE_LIMIT_EXCEEDED" => RateLimitReason::RateLimitExceeded,
                        _ => RateLimitReason::Unknown,
                    };
                }
            }
        }

        // 如果无法从 JSON 解析，尝试从消息文本判断
        if body.contains("exhausted") || body.contains("quota") {
            RateLimitReason::QuotaExhausted
        } else if body.contains("rate limit") || body.contains("too many requests") {
            RateLimitReason::RateLimitExceeded
        } else {
            RateLimitReason::Unknown
        }
    }

    /// 通用时间解析函数：支持 "2h1m1s" 等所有格式组合
    /// [OPTIMIZATION] Uses pre-compiled static regex instead of compiling on each call
    fn parse_duration_string(s: &str) -> Option<u64> {
        tracing::debug!("[时间解析] 尝试解析: '{}'", s);

        // Use pre-compiled regex for better performance
        let Some(caps) = DURATION_REGEX.captures(s) else {
            tracing::warn!("[时间解析] 正则未匹配: '{}'", s);
            return None;
        };

        let hours = caps
            .get(1)
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        let minutes = caps
            .get(2)
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        let seconds = caps
            .get(3)
            .and_then(|m| m.as_str().parse::<f64>().ok())
            .unwrap_or(0.0);
        let milliseconds = caps
            .get(4)
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(0);

        tracing::debug!(
            "[时间解析] 提取结果: {}h {}m {:.3}s {}ms",
            hours,
            minutes,
            seconds,
            milliseconds
        );

        // 计算总秒数
        let total_seconds =
            hours * 3600 + minutes * 60 + seconds.ceil() as u64 + milliseconds.div_ceil(1000);

        // 如果总秒数为 0，说明解析失败
        if total_seconds == 0 {
            tracing::warn!("[时间解析] 失败: '{}' (总秒数为0)", s);
            None
        } else {
            tracing::info!(
                "[时间解析] ✓ 成功: '{}' => {}秒 ({}h {}m {:.1}s)",
                s,
                total_seconds,
                hours,
                minutes,
                seconds
            );
            Some(total_seconds)
        }
    }

    /// 从错误消息 body 中解析重置时间
    /// [OPTIMIZATION] Uses pre-compiled static regex patterns instead of compiling on each call
    fn parse_retry_time_from_body(body: &str) -> Option<u64> {
        // A. 优先尝试 JSON 精准解析 (借鉴 PR #28)
        let trimmed = body.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                // 1. Google 常见的 quotaResetDelay 格式 (支持所有格式："2h1m1s", "1h30m", "42s", "500ms" 等)
                // 路径: error.details[0].metadata.quotaResetDelay
                if let Some(delay_str) = json
                    .get("error")
                    .and_then(|e| e.get("details"))
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("metadata")) // 添加 metadata 层级
                    .and_then(|m| m.get("quotaResetDelay"))
                    .and_then(|v| v.as_str())
                {
                    tracing::debug!("[JSON解析] 找到 quotaResetDelay: '{}'", delay_str);

                    // 使用通用时间解析函数
                    if let Some(seconds) = Self::parse_duration_string(delay_str) {
                        return Some(seconds);
                    }
                }

                // 2. OpenAI 常见的 retry_after 字段 (数字)
                if let Some(retry) = json
                    .get("error")
                    .and_then(|e| e.get("retry_after"))
                    .and_then(serde_json::Value::as_u64)
                {
                    return Some(retry);
                }
            }
        }

        // B. 正则匹配模式 (兜底) - Using pre-compiled static patterns
        // 模式 1: "Try again in 2m 30s"
        if let Some(caps) = RETRY_MIN_SEC_REGEX.captures(body) {
            if let (Ok(m), Ok(s)) = (caps[1].parse::<u64>(), caps[2].parse::<u64>()) {
                return Some(m * 60 + s);
            }
        }

        // 模式 2: "Try again in 30s" 或 "backoff for 42s"
        if let Some(caps) = RETRY_SEC_REGEX.captures(body) {
            if let Ok(s) = caps[1].parse::<u64>() {
                return Some(s);
            }
        }

        // 模式 3: "quota will reset in X seconds"
        if let Some(caps) = QUOTA_RESET_REGEX.captures(body) {
            if let Ok(s) = caps[1].parse::<u64>() {
                return Some(s);
            }
        }

        // 模式 4: OpenAI 风格的 "Retry after (\d+) seconds"
        if let Some(caps) = RETRY_AFTER_SEC_REGEX.captures(body) {
            if let Ok(s) = caps[1].parse::<u64>() {
                return Some(s);
            }
        }

        // 模式 5: 括号形式 "(wait (\d+)s)"
        if let Some(caps) = WAIT_PAREN_REGEX.captures(body) {
            if let Ok(s) = caps[1].parse::<u64>() {
                return Some(s);
            }
        }

        None
    }

    /// 获取账号的限流信息
    pub fn get(&self, account_id: &str) -> Option<RateLimitInfo> {
        self.limits.get(account_id).map(|r| r.clone())
    }

    /// 获取账号的限流信息（带配额组）
    #[allow(dead_code)]
    pub fn get_for_group(
        &self,
        account_id: &str,
        quota_group: Option<&str>,
    ) -> Option<RateLimitInfo> {
        let key = Self::make_key(account_id, quota_group);
        self.limits.get(&key).map(|r| r.clone())
    }

    /// 检查账号是否仍在限流中
    pub fn is_rate_limited(&self, account_id: &str) -> bool {
        if let Some(info) = self.get(account_id) {
            info.reset_time > SystemTime::now()
        } else {
            false
        }
    }

    /// 检查账号是否仍在限流中（带配额组）
    /// 这允许同一账号的 Claude 和 Gemini 配额独立检查
    #[allow(dead_code)]
    pub fn is_rate_limited_for_group(&self, account_id: &str, quota_group: Option<&str>) -> bool {
        if let Some(info) = self.get_for_group(account_id, quota_group) {
            info.reset_time > SystemTime::now()
        } else {
            false
        }
    }

    /// 获取距离限流重置还有多少秒
    pub fn get_reset_seconds(&self, account_id: &str) -> Option<u64> {
        if let Some(info) = self.get(account_id) {
            info.reset_time
                .duration_since(SystemTime::now())
                .ok()
                .map(|d| d.as_secs())
        } else {
            None
        }
    }

    pub fn get_reset_seconds_for_group(
        &self,
        account_id: &str,
        quota_group: Option<&str>,
    ) -> Option<u64> {
        if let Some(info) = self.get_for_group(account_id, quota_group) {
            info.reset_time
                .duration_since(SystemTime::now())
                .ok()
                .map(|d| d.as_secs())
        } else {
            None
        }
    }

    pub fn get_remaining_wait_for_group(&self, account_id: &str, quota_group: Option<&str>) -> u64 {
        let key = Self::make_key(account_id, quota_group);
        if let Some(info) = self.limits.get(&key) {
            let now = SystemTime::now();
            if info.reset_time > now {
                return info
                    .reset_time
                    .duration_since(now)
                    .unwrap_or(Duration::from_secs(0))
                    .as_secs();
            }
        }
        0
    }

    /// 清除过期的限流记录
    #[allow(dead_code)]
    pub fn cleanup_expired(&self) -> usize {
        let now = SystemTime::now();
        let mut count = 0;

        self.limits.retain(|_k, v| {
            if v.reset_time <= now {
                count += 1;
                false
            } else {
                true
            }
        });

        if count > 0 {
            tracing::debug!("清除了 {} 个过期的限流记录", count);
        }

        count
    }

    /// 清除指定账号的限流记录
    #[allow(dead_code)]
    pub fn clear(&self, account_id: &str) -> bool {
        self.limits.remove(account_id).is_some()
    }

    /// 清除所有限流记录
    #[allow(dead_code)]
    pub fn clear_all(&self) {
        let count = self.limits.len();
        self.limits.clear();
        tracing::debug!("清除了所有 {} 条限流记录", count);
    }
}

impl Default for RateLimitTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_retry_time_minutes_seconds() {
        let body = "Rate limit exceeded. Try again in 2m 30s";
        let time = RateLimitTracker::parse_retry_time_from_body(body);
        assert_eq!(time, Some(150));
    }

    #[test]
    fn test_parse_google_json_delay() {
        // Test the actual Google API format: error.details[0].metadata.quotaResetDelay
        let body = r#"{
            "error": {
                "details": [
                    { "metadata": { "quotaResetDelay": "42s" } }
                ]
            }
        }"#;
        let time = RateLimitTracker::parse_retry_time_from_body(body);
        assert_eq!(time, Some(42));
    }

    #[test]
    fn test_parse_retry_after_ignore_case() {
        let body = "Quota limit hit. Retry After 99 Seconds";
        let time = RateLimitTracker::parse_retry_time_from_body(body);
        assert_eq!(time, Some(99));
    }

    #[test]
    fn test_get_remaining_wait() {
        let tracker = RateLimitTracker::new();
        tracker.parse_from_error("acc1", 429, Some("30"), "");
        let wait = tracker.get_remaining_wait("acc1");
        assert!(wait > 25 && wait <= 30);
    }

    #[test]
    fn test_safety_buffer() {
        let tracker = RateLimitTracker::new();
        // If API returns 1s, we force it to 2s minimum (safety buffer)
        tracker.parse_from_error("acc1", 429, Some("1"), "");
        let wait = tracker.get_remaining_wait("acc1");
        // Use range assertion to handle timing edge cases (as_secs() truncates)
        assert!((1..=2).contains(&wait), "expected wait 1-2s, got {wait}");
    }
}
