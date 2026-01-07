use serde::{Deserialize, Serialize};
// use std::path::PathBuf;
use std::collections::HashMap;

/// Log rotation configuration for the headless server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRotationConfig {
    /// Enable log rotation (default: true)
    #[serde(default = "default_log_rotation_enabled")]
    pub enabled: bool,

    /// Rotation strategy: "daily", "hourly", or "size"
    #[serde(default = "default_log_rotation_strategy")]
    pub strategy: String,

    /// Maximum number of log files to keep (default: 7)
    /// Old files beyond this limit are automatically deleted
    #[serde(default = "default_log_max_files")]
    pub max_files: usize,

    /// Enable compression for rotated logs (gzip)
    #[serde(default)]
    pub compress: bool,

    /// Maximum file size in MB before rotation (only for "size" strategy)
    #[serde(default = "default_log_max_size_mb")]
    pub max_size_mb: u64,

    /// Use UTC timezone for log file naming (default: true)
    #[serde(default = "default_log_use_utc")]
    pub use_utc: bool,

    /// Separate log files by level (errors.log, debug.log, etc.)
    #[serde(default)]
    pub separate_by_level: bool,
}

fn default_log_rotation_enabled() -> bool {
    true
}

fn default_log_rotation_strategy() -> String {
    "daily".to_string()
}

fn default_log_max_files() -> usize {
    7
}

fn default_log_max_size_mb() -> u64 {
    100
}

fn default_log_use_utc() -> bool {
    true
}

impl Default for LogRotationConfig {
    fn default() -> Self {
        Self {
            enabled: default_log_rotation_enabled(),
            strategy: default_log_rotation_strategy(),
            max_files: default_log_max_files(),
            compress: false,
            max_size_mb: default_log_max_size_mb(),
            use_utc: default_log_use_utc(),
            separate_by_level: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProxyAuthMode {
    #[default]
    Off,
    Strict,
    AllExceptHealth,
    Auto,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ZaiDispatchMode {
    /// Never use z.ai.
    #[default]
    Off,
    /// Use z.ai for all Anthropic protocol requests.
    Exclusive,
    /// Treat z.ai as one additional slot in the shared pool.
    Pooled,
    /// Use z.ai only when the Google pool is unavailable.
    Fallback,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiModelDefaults {
    /// Default model for "opus" family (when the incoming model is a Claude id).
    #[serde(default = "default_zai_opus_model")]
    pub opus: String,
    /// Default model for "sonnet" family (when the incoming model is a Claude id).
    #[serde(default = "default_zai_sonnet_model")]
    pub sonnet: String,
    /// Default model for "haiku" family (when the incoming model is a Claude id).
    #[serde(default = "default_zai_haiku_model")]
    pub haiku: String,
}

impl Default for ZaiModelDefaults {
    fn default() -> Self {
        Self {
            opus: default_zai_opus_model(),
            sonnet: default_zai_sonnet_model(),
            haiku: default_zai_haiku_model(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct ZaiMcpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub web_search_enabled: bool,
    #[serde(default)]
    pub web_reader_enabled: bool,
    #[serde(default)]
    pub vision_enabled: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_zai_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub dispatch_mode: ZaiDispatchMode,
    /// Optional per-model mapping overrides for Anthropic/Claude model ids.
    /// Key: incoming `model` string, Value: upstream z.ai model id (e.g. `glm-4.7`).
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    #[serde(default)]
    pub models: ZaiModelDefaults,
    #[serde(default)]
    pub mcp: ZaiMcpConfig,
}

impl Default for ZaiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_zai_base_url(),
            api_key: String::new(),
            dispatch_mode: ZaiDispatchMode::Off,
            model_mapping: HashMap::new(),
            models: ZaiModelDefaults::default(),
            mcp: ZaiMcpConfig::default(),
        }
    }
}

/// 反代服务配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 是否启用反代服务
    pub enabled: bool,

    /// 是否允许局域网访问
    /// - false: 仅本机访问 127.0.0.1（默认，隐私优先）
    /// - true: 允许局域网访问 0.0.0.0
    #[serde(default)]
    pub allow_lan_access: bool,

    /// Authorization policy for the proxy.
    /// - off: no auth required
    /// - strict: auth required for all routes
    /// - all_except_health: auth required for all routes except `/healthz`
    /// - auto: recommended defaults (currently: allow_lan_access => all_except_health, else off)
    #[serde(default)]
    pub auth_mode: ProxyAuthMode,
    
    /// 监听端口
    pub port: u16,
    
    /// API 密钥
    pub api_key: String,
    

    /// 是否自动启动
    pub auto_start: bool,

    /// Anthropic 模型映射表 (key: Claude模型名, value: Gemini模型名)
    #[serde(default)]
    pub anthropic_mapping: std::collections::HashMap<String, String>,

    /// OpenAI 模型映射表 (key: OpenAI模型组, value: Gemini模型名)
    #[serde(default)]
    pub openai_mapping: std::collections::HashMap<String, String>,

    /// 自定义精确模型映射表 (key: 原始模型名, value: 目标模型名)
    #[serde(default)]
    pub custom_mapping: std::collections::HashMap<String, String>,

    /// API 请求超时时间(秒)
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// 是否开启请求日志记录 (监控)
    #[serde(default)]
    pub enable_logging: bool,

    /// 上游代理配置
    #[serde(default)]
    pub upstream_proxy: UpstreamProxyConfig,

    /// z.ai provider configuration (Anthropic-compatible).
    #[serde(default)]
    pub zai: ZaiConfig,
    
    /// 账号调度配置 (粘性会话/限流重试)
    #[serde(default)]
    pub scheduling: crate::proxy::sticky_config::StickySessionConfig,

    /// Log rotation configuration (headless server)
    #[serde(default)]
    pub log_rotation: LogRotationConfig,

    /// Connection pool warming configuration
    #[serde(default)]
    pub pool_warming: PoolWarmingConfig,

    /// Semantic request logging with sampling
    #[serde(default)]
    pub sampling: SamplingConfig,

    /// Request hedging configuration (speculative retry)
    #[serde(default)]
    pub hedging: HedgingConfig,
}

/// 上游代理配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpstreamProxyConfig {
    /// 是否启用
    pub enabled: bool,
    /// 代理地址 (http://, https://, socks5://)
    pub url: String,
}

/// Connection pool warming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolWarmingConfig {
    /// Enable connection pool warming (default: true)
    #[serde(default = "default_pool_warming_enabled")]
    pub enabled: bool,

    /// Interval between warming pings in seconds (default: 30)
    #[serde(default = "default_pool_warming_interval")]
    pub interval_secs: u64,
}

/// Semantic request logging with sampling configuration
///
/// This feature logs a percentage of request/response bodies for debugging
/// and observability purposes without impacting performance significantly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingConfig {
    /// Enable semantic request sampling (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Percentage of requests to sample (0.0 - 1.0, default: 0.01 = 1%)
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,

    /// Maximum body size to log in bytes (default: 4096)
    /// Bodies exceeding this limit will be truncated
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,

    /// Include headers in sampled logs (default: false)
    /// Sensitive headers (Authorization, X-API-Key) are always sanitized
    #[serde(default)]
    pub include_headers: bool,
}

fn default_sample_rate() -> f64 {
    0.01 // 1% sampling by default
}

fn default_max_body_size() -> usize {
    4096 // 4KB default
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_rate: default_sample_rate(),
            max_body_size: default_max_body_size(),
            include_headers: false,
        }
    }
}

/// Request hedging (speculative retry) configuration
///
/// Hedging improves tail latency by firing a backup request if the primary
/// takes too long. When the winner completes, the loser is cancelled.
///
/// IMPORTANT: Only applies to non-streaming requests (streaming uses SSE which
/// is harder to hedge without duplicating data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HedgingConfig {
    /// Enable request hedging (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Delay in milliseconds before firing the hedge request (default: 2000ms)
    /// If the primary request doesn't respond within this time, a backup is sent.
    #[serde(default = "default_hedge_delay_ms")]
    pub hedge_delay_ms: u64,

    /// Maximum number of concurrent hedged requests (default: 1)
    /// Setting to 0 effectively disables hedging.
    #[serde(default = "default_max_hedged_requests")]
    pub max_hedged_requests: usize,
}

fn default_hedge_delay_ms() -> u64 {
    2000 // 2 seconds
}

fn default_max_hedged_requests() -> usize {
    1
}

impl Default for HedgingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            hedge_delay_ms: default_hedge_delay_ms(),
            max_hedged_requests: default_max_hedged_requests(),
        }
    }
}

fn default_pool_warming_enabled() -> bool {
    true
}

fn default_pool_warming_interval() -> u64 {
    30
}

impl Default for PoolWarmingConfig {
    fn default() -> Self {
        Self {
            enabled: default_pool_warming_enabled(),
            interval_secs: default_pool_warming_interval(),
        }
    }
}

/// Get default proxy port from env or use 8045
fn default_proxy_port() -> u16 {
    std::env::var("ANTIGRAVITY_PROXY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8045)
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_lan_access: false, // 默认仅本机访问，隐私优先
            auth_mode: ProxyAuthMode::default(),
            port: default_proxy_port(),
            api_key: format!("sk-{}", uuid::Uuid::new_v4().simple()),
            auto_start: false,
            anthropic_mapping: std::collections::HashMap::new(),
            openai_mapping: std::collections::HashMap::new(),
            custom_mapping: std::collections::HashMap::new(),
            request_timeout: default_request_timeout(),
            enable_logging: false, // 默认关闭，节省性能
            upstream_proxy: UpstreamProxyConfig::default(),
            zai: ZaiConfig::default(),
            scheduling: crate::proxy::sticky_config::StickySessionConfig::default(),
            log_rotation: LogRotationConfig::default(),
            pool_warming: PoolWarmingConfig::default(),
            sampling: SamplingConfig::default(),
            hedging: HedgingConfig::default(),
        }
    }
}

fn default_request_timeout() -> u64 {
    120  // 默认 120 秒,原来 60 秒太短
}

fn default_zai_base_url() -> String {
    "https://api.z.ai/api/anthropic".to_string()
}

fn default_zai_opus_model() -> String {
    "glm-4.7".to_string()
}

fn default_zai_sonnet_model() -> String {
    "glm-4.7".to_string()
}

fn default_zai_haiku_model() -> String {
    "glm-4.5-air".to_string()
}

impl ProxyConfig {
    /// 获取实际的监听地址
    /// - allow_lan_access = false: 返回 "127.0.0.1"（默认，隐私优先）
    /// - allow_lan_access = true: 返回 "0.0.0.0"（允许局域网访问）
    pub fn get_bind_address(&self) -> &str {
        if self.allow_lan_access {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        }
    }
}
