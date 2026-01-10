//! Backend bridge for Slint UI.
//!
//! This module provides the interface between the Slint UI and the core business logic.

use antigravity_core::models::Account;
use antigravity_core::modules;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Backend state shared with the UI.
pub struct BackendState {
    /// Currently loaded accounts.
    accounts: Vec<Account>,
    /// Current account index.
    current_account_id: Option<String>,
}

impl BackendState {
    /// Create a new backend state.
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            current_account_id: None,
        }
    }

    /// Load all accounts from storage.
    pub fn load_accounts(&mut self) -> Result<(), String> {
        self.accounts = modules::list_accounts()?;
        self.current_account_id = modules::get_current_account_id()?;
        tracing::info!("Loaded {} accounts", self.accounts.len());
        Ok(())
    }

    /// Get all accounts.
    pub fn get_accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Get current account.
    pub fn get_current_account(&self) -> Option<&Account> {
        self.current_account_id.as_ref().and_then(|id| {
            self.accounts.iter().find(|a| &a.id == id)
        })
    }

    /// Get account count.
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Calculate average quota for a model type (e.g., "gemini", "claude").
    fn avg_quota_for_model(&self, model_prefix: &str) -> f32 {
        let quotas: Vec<f32> = self.accounts.iter()
            .filter_map(|a| a.quota.as_ref())
            .flat_map(|q| {
                q.models.iter()
                    .filter(|m| m.name.to_lowercase().contains(model_prefix))
                    .map(|m| m.percentage as f32)
            })
            .collect();
        
        if quotas.is_empty() { 0.0 } else { quotas.iter().sum::<f32>() / quotas.len() as f32 }
    }

    /// Calculate average Gemini quota.
    pub fn avg_gemini_quota(&self) -> f32 {
        self.avg_quota_for_model("gemini")
    }

    /// Calculate average Gemini Image quota.
    pub fn avg_gemini_image_quota(&self) -> f32 {
        self.avg_quota_for_model("image")
    }

    /// Calculate average Claude quota.
    pub fn avg_claude_quota(&self) -> f32 {
        self.avg_quota_for_model("claude")
    }

    /// Count accounts with low quota (< 20%).
    pub fn low_quota_count(&self) -> usize {
        self.accounts.iter()
            .filter(|a| {
                if let Some(q) = &a.quota {
                    q.models.iter().any(|m| m.percentage < 20)
                } else {
                    false
                }
            })
            .count()
    }
}

impl Default for BackendState {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe backend handle.
pub type Backend = Arc<Mutex<BackendState>>;

/// Create a new backend handle.
pub fn create_backend() -> Backend {
    Arc::new(Mutex::new(BackendState::new()))
}
