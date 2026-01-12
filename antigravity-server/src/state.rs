//! Application State
//!
//! Holds shared state for the server including account manager and proxy config.

use anyhow::Result;
use axum::Router;
use std::sync::Arc;

use antigravity_core::modules::account;
use antigravity_core::models::Account;
use antigravity_core::proxy::{build_proxy_router, ProxyMonitor, ProxySecurityConfig, TokenManager};
use antigravity_shared::proxy::config::ProxyConfig;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    /// Token manager for account rotation
    token_manager: Arc<TokenManager>,
    /// Proxy monitor for logging
    monitor: Arc<ProxyMonitor>,
    /// Proxy configuration
    proxy_config: ProxyConfig,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        // Get data directory
        let data_dir = account::get_data_dir()
            .map_err(|e| anyhow::anyhow!("Failed to get data directory: {}", e))?;
        
        tracing::info!("📁 Data directory: {:?}", data_dir);
        
        // Initialize token manager
        let token_manager = Arc::new(TokenManager::new(data_dir.clone()));
        
        // Load accounts into token manager
        match token_manager.load_accounts().await {
            Ok(count) => {
                tracing::info!("📊 Loaded {} accounts into token manager", count);
            }
            Err(e) => {
                tracing::warn!("⚠️ Could not load accounts: {}", e);
            }
        }
        
        // Initialize proxy monitor
        let monitor = Arc::new(ProxyMonitor::new());
        
        // Load proxy config (or use defaults)
        let proxy_config = load_proxy_config(&data_dir).unwrap_or_default();
        
        Ok(Self {
            inner: Arc::new(AppStateInner {
                token_manager,
                monitor,
                proxy_config,
            }),
        })
    }
    
    /// Build proxy router for integration into main server
    pub fn build_proxy_router(&self) -> Router {
        let config = &self.inner.proxy_config;
        
        // Build security config
        let security_config = ProxySecurityConfig::from_proxy_config(config);
        
        // Build proxy router (returns Router<()> with its own internal state)
        build_proxy_router(
            self.inner.token_manager.clone(),
            config.custom_mapping.clone(),
            config.upstream_proxy.clone(),
            security_config,
            config.zai.clone(),
            self.inner.monitor.clone(),
            config.experimental.clone(),
        )
    }
    
    /// List all accounts
    pub fn list_accounts(&self) -> Result<Vec<Account>, String> {
        account::list_accounts()
    }
    
    /// Get current active account
    pub fn get_current_account(&self) -> Result<Option<Account>, String> {
        account::get_current_account()
    }
    
    /// Switch to a different account
    pub async fn switch_account(&self, account_id: &str) -> Result<(), String> {
        account::switch_account(account_id).await
    }
    
    /// Get enabled account count
    pub fn get_account_count(&self) -> usize {
        match account::list_accounts() {
            Ok(accounts) => accounts.iter().filter(|a| !a.disabled).count(),
            Err(_) => 0,
        }
    }
    
    /// Get proxy port from config
    pub fn get_proxy_port(&self) -> u16 {
        self.inner.proxy_config.port
    }
    
    /// Get proxy stats
    pub async fn get_proxy_stats(&self) -> antigravity_shared::models::ProxyStats {
        self.inner.monitor.get_stats().await
    }
    
    /// Get proxy logs
    pub async fn get_proxy_logs(&self, limit: Option<usize>) -> Vec<antigravity_shared::models::ProxyRequestLog> {
        self.inner.monitor.get_logs(limit).await
    }
    
    /// Clear proxy logs
    pub async fn clear_proxy_logs(&self) {
        self.inner.monitor.clear_logs().await;
    }
    
    /// Get token manager account count
    pub fn get_token_manager_count(&self) -> usize {
        self.inner.token_manager.len()
    }
}

/// Helper to extract quota percentage by model name
pub fn get_model_quota(account: &Account, model_prefix: &str) -> Option<i32> {
    account.quota.as_ref().and_then(|q| {
        q.models.iter()
            .find(|m| m.name.to_lowercase().contains(&model_prefix.to_lowercase()))
            .map(|m| m.percentage)
    })
}

/// Load proxy config from disk
fn load_proxy_config(data_dir: &std::path::Path) -> Option<ProxyConfig> {
    let config_path = data_dir.join("config.json");
    
    if !config_path.exists() {
        return None;
    }
    
    let content = std::fs::read_to_string(&config_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    
    // Extract proxy config from the main config
    if let Some(proxy) = value.get("proxy") {
        serde_json::from_value(proxy.clone()).ok()
    } else {
        None
    }
}
