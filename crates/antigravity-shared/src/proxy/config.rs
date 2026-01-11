use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// Re-export UpstreamProxyConfig from utils
pub use crate::utils::http::UpstreamProxyConfig;

// --- Enums ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyAuthMode {
    #[default]
    Off,
    Strict,
    AllExceptHealth,
    Auto,
}

impl fmt::Display for ProxyAuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyAuthMode::Off => write!(f, "off"),
            ProxyAuthMode::Strict => write!(f, "strict"),
            ProxyAuthMode::AllExceptHealth => write!(f, "all_except_health"),
            ProxyAuthMode::Auto => write!(f, "auto"),
        }
    }
}

impl ProxyAuthMode {
    pub fn from_string(s: &str) -> Self {
        match s {
            "strict" => ProxyAuthMode::Strict,
            "all_except_health" => ProxyAuthMode::AllExceptHealth,
            "auto" => ProxyAuthMode::Auto,
            _ => ProxyAuthMode::Off,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZaiDispatchMode {
    #[default]
    Off,
    Exclusive,
    Pooled,
    Fallback,
}

impl fmt::Display for ZaiDispatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZaiDispatchMode::Off => write!(f, "off"),
            ZaiDispatchMode::Exclusive => write!(f, "exclusive"),
            ZaiDispatchMode::Pooled => write!(f, "pooled"),
            ZaiDispatchMode::Fallback => write!(f, "fallback"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    OpenAI,
    Anthropic,
    Gemini,
}

// --- Z.ai Structs ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZaiModelDefaults {
    #[serde(default = "default_zai_opus_model")]
    pub opus: String,
    #[serde(default = "default_zai_sonnet_model")]
    pub sonnet: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ZaiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_zai_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub dispatch_mode: ZaiDispatchMode,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    #[serde(default)]
    pub models: ZaiModelDefaults,
    #[serde(default)]
    pub mcp: ZaiMcpConfig,
}

// --- Other Config Structs ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExperimentalConfig {
    #[serde(default = "default_true")]
    pub enable_signature_cache: bool,
    #[serde(default = "default_true")]
    pub enable_tool_loop_recovery: bool,
    #[serde(default = "default_true")]
    pub enable_cross_model_checks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StickySessionConfig {
    pub enabled: bool,
    pub mode: String,
    pub ttl: u32,
}

// --- Proxy Config ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyConfig {
    pub enabled: bool,
    #[serde(default)]
    pub allow_lan_access: bool,
    #[serde(default)]
    pub auth_mode: ProxyAuthMode,
    pub port: u16,
    pub api_key: String,
    pub auto_start: bool,
    #[serde(default)]
    pub custom_mapping: HashMap<String, String>,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default)]
    pub enable_logging: bool,
    #[serde(default)]
    pub upstream_proxy: UpstreamProxyConfig,
    #[serde(default)]
    pub zai: ZaiConfig,
    #[serde(default)]
    pub scheduling: StickySessionConfig,
    #[serde(default)]
    pub experimental: ExperimentalConfig,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_lan_access: false,
            auth_mode: ProxyAuthMode::default(),
            port: 8045,
            api_key: String::new(),
            auto_start: true,
            custom_mapping: HashMap::new(),
            request_timeout: 120,
            enable_logging: false,
            upstream_proxy: UpstreamProxyConfig::default(),
            zai: ZaiConfig::default(),
            scheduling: StickySessionConfig::default(),
            experimental: ExperimentalConfig::default(),
        }
    }
}

impl ProxyConfig {
    pub fn get_bind_address(&self) -> String {
        if self.allow_lan_access {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    }
}

// --- Defaults ---

fn default_true() -> bool {
    true
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

fn default_request_timeout() -> u64 {
    120
}
