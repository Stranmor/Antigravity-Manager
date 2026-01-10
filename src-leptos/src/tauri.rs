//! Tauri IPC bindings for Leptos
//!
//! This module provides type-safe wrappers around Tauri's invoke() function.

use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

/// Call a Tauri command with typed arguments and return value.
pub async fn tauri_invoke<A, R>(cmd: &str, args: A) -> Result<R, String>
where
    A: Serialize,
    R: DeserializeOwned,
{
    let args_js = serde_wasm_bindgen::to_value(&args)
        .map_err(|e| format!("Failed to serialize args: {}", e))?;
    
    let result = invoke(cmd, args_js).await;
    
    // Check if result is an error
    if result.is_undefined() || result.is_null() {
        return Err("Command returned null/undefined".to_string());
    }
    
    serde_wasm_bindgen::from_value(result)
        .map_err(|e| format!("Failed to deserialize result: {}", e))
}

/// Call a Tauri command with no arguments.
pub async fn tauri_invoke_no_args<R>(cmd: &str) -> Result<R, String>
where
    R: DeserializeOwned,
{
    tauri_invoke(cmd, serde_json::json!({})).await
}

// Re-export common command wrappers
pub mod commands {
    use super::*;
    use crate::types::*;
    
    /// Load application configuration
    pub async fn load_config() -> Result<AppConfig, String> {
        tauri_invoke_no_args("load_config").await
    }
    
    /// Save application configuration
    pub async fn save_config(config: &AppConfig) -> Result<(), String> {
        tauri_invoke("save_config", serde_json::json!({ "config": config })).await
    }
    
    /// List all accounts
    pub async fn list_accounts() -> Result<Vec<Account>, String> {
        tauri_invoke_no_args("list_accounts").await
    }
    
    /// Get current account ID
    pub async fn get_current_account_id() -> Result<Option<String>, String> {
        tauri_invoke_no_args("get_current_account_id").await
    }
    
    /// Set current account
    pub async fn set_current_account_id(id: &str) -> Result<(), String> {
        tauri_invoke("set_current_account_id", serde_json::json!({ "id": id })).await
    }
    
    /// Delete account
    pub async fn delete_account(id: &str) -> Result<(), String> {
        tauri_invoke("delete_account", serde_json::json!({ "id": id })).await
    }
    
    /// Get proxy status
    pub async fn get_proxy_status() -> Result<ProxyStatus, String> {
        tauri_invoke_no_args("get_proxy_status").await
    }
    
    /// Start proxy service
    pub async fn start_proxy_service(config: &ProxyConfig) -> Result<(), String> {
        tauri_invoke("start_proxy_service", serde_json::json!({ "config": config })).await
    }
    
    /// Stop proxy service
    pub async fn stop_proxy_service() -> Result<(), String> {
        tauri_invoke_no_args("stop_proxy_service").await
    }
    
    /// Generate API key
    pub async fn generate_api_key() -> Result<String, String> {
        tauri_invoke_no_args("generate_api_key").await
    }
}
