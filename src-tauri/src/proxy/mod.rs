// proxy 模块 - API 反代服务

// 现有模块 (保留)
pub mod project_resolver;
pub mod security;
pub mod server;
pub mod token_manager;

// 新架构模块
pub mod audio; // 音频处理模块 (PR #311)
pub mod common; // 公共工具
pub mod handlers; // API 端点处理器
pub mod mappers; // 协议转换器
pub mod middleware; // Axum 中间件
pub mod monitor; // 监控
pub mod providers; // Extra upstream providers (z.ai, etc.)
pub mod rate_limit; // 限流跟踪
pub mod session_manager; // 会话指纹管理
pub mod signature_cache;
pub mod sticky_config; // 粘性调度配置
pub mod upstream; // 上游客户端
pub mod zai_vision_mcp; // Built-in Vision MCP server state
pub mod zai_vision_tools; // Built-in Vision MCP tools (z.ai vision API)

// Restored: AIMD Predictive Rate Limiting System (lost during headless migration 2026-01-12)
pub mod adaptive_limit; // AIMD per-account limit tracking with TCP-style congestion control
pub mod health;
pub mod prometheus; // Prometheus metrics for adaptive rate limiting
pub mod smart_prober; // Speculative hedging and cheap probing for limit discovery // Account health monitoring and proactive rotation

pub use antigravity_shared::proxy::config;
pub use antigravity_shared::proxy::config::{ProxyAuthMode, ZaiConfig, ZaiDispatchMode};
pub use security::ProxySecurityConfig;
pub use server::AxumServer;
pub use signature_cache::SignatureCache;
pub use token_manager::TokenManager;

#[cfg(test)]
pub mod tests;
