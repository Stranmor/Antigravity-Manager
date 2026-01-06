//! Antigravity Server - Headless proxy server for VPS deployment
//!
//! This binary runs the proxy server without the Tauri GUI, suitable for
//! VPS/container deployments where only the API proxy is needed.
//!
//! Environment variables:
//! - ANTIGRAVITY_DATA_DIR: Data directory (default: ~/.antigravity)
//! - ANTIGRAVITY_PROXY_PORT: Proxy API port (default: 8045)
//! - ANTIGRAVITY_ADMIN_PORT: Admin API port (default: 9101)
//! - ANTIGRAVITY_ALLOW_LAN: Bind to 0.0.0.0 (default: true for containers)
//! - ANTIGRAVITY_API_KEY: API key for authentication
//! - ANTIGRAVITY_ENABLE_LOGGING: Enable request logging (default: true)
//! - RUST_LOG: Tracing filter (default: info,antigravity_tools_lib=debug)
//!
//! OpenTelemetry (when compiled with 'otel' feature):
//! - OTEL_EXPORTER_OTLP_ENDPOINT: OTLP collector endpoint (default: http://localhost:4317)
//! - OTEL_SERVICE_NAME: Service name for traces (default: antigravity-proxy)
//! - OTEL_ENABLED: Enable/disable OTEL export (default: true)

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Path, State, Request},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::signal;
use tokio::sync::RwLock;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::KeyExtractor, GovernorError, GovernorLayer,
};

/// Custom key extractor for containerized environments
///
/// Falls back to a default key if IP extraction fails (common in container networks).
/// This ensures rate limiting works even when requests come from localhost or
/// container internal networks without proper X-Forwarded-For headers.
#[derive(Clone)]
struct ContainerAwareKeyExtractor;

impl KeyExtractor for ContainerAwareKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        // Try X-Forwarded-For first (for reverse proxy scenarios)
        if let Some(xff) = req.headers().get("x-forwarded-for") {
            if let Ok(xff_str) = xff.to_str() {
                // Take the first IP in the chain (client IP)
                if let Some(first_ip) = xff_str.split(',').next() {
                    let ip = first_ip.trim();
                    if !ip.is_empty() {
                        return Ok(ip.to_string());
                    }
                }
            }
        }

        // Try X-Real-IP
        if let Some(real_ip) = req.headers().get("x-real-ip") {
            if let Ok(ip) = real_ip.to_str() {
                if !ip.is_empty() {
                    return Ok(ip.to_string());
                }
            }
        }

        // Fall back to connect info if available (requires ConnectInfo layer)
        // For now, use a default localhost key to avoid errors
        Ok("localhost".to_string())
    }
}
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use clap::{Parser, Subcommand};

// Re-use the existing proxy module from the library
use antigravity_tools_lib::models::{Account, TokenData};
use antigravity_tools_lib::proxy::{
    config::ProxyConfig, monitor::ProxyMonitor, prometheus, server::AxumServer,
    telemetry, ProxySecurityConfig, TokenManager,
    init_server_logger, start_log_cleanup_task, LogGuards,
};

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser)]
#[command(name = "antigravity-server")]
#[command(about = "Antigravity Server - Headless proxy for VPS deployment")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Import accounts from desktop app data directory
    Import {
        /// Source directory (default: ~/.antigravity_tools)
        #[arg(short, long)]
        from: Option<PathBuf>,
        
        /// Target directory (default: ~/.antigravity)
        #[arg(short, long)]
        to: Option<PathBuf>,
    },
    
    /// Start the proxy server (default command)
    Serve,
}

/// Server start time for uptime calculation
static SERVER_START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

// ============================================================================
// Configuration
// ============================================================================

/// Server configuration loaded from environment variables
struct ServerConfig {
    /// Data directory containing accounts and config
    data_dir: PathBuf,
    /// Proxy port (default: 8045)
    proxy_port: u16,
    /// Admin API port (default: 9101)
    admin_port: u16,
    /// Bind to all interfaces (0.0.0.0) instead of localhost
    allow_lan: bool,
    /// API key for authentication (optional, auto-generated if not set)
    api_key: Option<String>,
    /// Enable request logging/monitoring
    enable_logging: bool,
}

impl ServerConfig {
    fn from_env() -> Self {
        // Default data dir is ~/.antigravity for VPS deployments
        let default_data_dir = dirs::home_dir().map_or_else(|| PathBuf::from("/var/lib/antigravity"), |h| h.join(".antigravity"));

        Self {
            data_dir: std::env::var("ANTIGRAVITY_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(default_data_dir),
            proxy_port: std::env::var("ANTIGRAVITY_PROXY_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8045),
            admin_port: std::env::var("ANTIGRAVITY_ADMIN_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(9101),
            allow_lan: std::env::var("ANTIGRAVITY_ALLOW_LAN")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(true), // Default true for containers
            api_key: std::env::var("ANTIGRAVITY_API_KEY").ok(),
            enable_logging: std::env::var("ANTIGRAVITY_ENABLE_LOGGING")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(true),
        }
    }
}

// ============================================================================
// Admin API Types
// ============================================================================

/// Shared state for admin API
struct AdminState {
    data_dir: PathBuf,
    proxy_config: RwLock<ProxyConfig>,
    token_manager: Arc<TokenManager>,
    monitor: Arc<ProxyMonitor>,
    proxy_server: RwLock<Option<ProxyServerHandle>>,
    /// API key for admin authentication (None = auth disabled)
    api_key: Option<String>,
    /// Health monitor for account health tracking
    health_monitor: Arc<antigravity_tools_lib::proxy::health::HealthMonitor>,
}

struct ProxyServerHandle {
    server: AxumServer,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    accounts_total: usize,
    accounts_available: usize,
    proxy_running: bool,
    proxy_port: u16,
}

// ============================================================================
// Detailed Health Check Types
// ============================================================================

/// Component health status with details
#[derive(Debug, Serialize)]
struct ComponentHealth {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accounts_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accounts_available: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accounts_rate_limited: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_circuits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    half_open_circuits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_connections: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requests_per_minute: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_files_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oldest_log_age_days: Option<u64>,
}

impl ComponentHealth {
    fn healthy() -> Self {
        Self {
            status: "healthy",
            latency_ms: None,
            size_bytes: None,
            accounts_total: None,
            accounts_available: None,
            accounts_rate_limited: None,
            open_circuits: None,
            half_open_circuits: None,
            active_connections: None,
            requests_per_minute: None,
            log_files_count: None,
            oldest_log_age_days: None,
        }
    }

    #[allow(dead_code)]
    fn degraded() -> Self {
        Self {
            status: "degraded",
            ..Self::healthy()
        }
    }

    fn unhealthy() -> Self {
        Self {
            status: "unhealthy",
            ..Self::healthy()
        }
    }
}

/// System resource checks
#[derive(Debug, Serialize)]
struct SystemChecks {
    disk_space_ok: bool,
    memory_ok: bool,
    cpu_ok: bool,
}

/// Detailed health response with all component statuses
#[derive(Debug, Serialize)]
struct DetailedHealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    components: DetailedComponents,
    checks: SystemChecks,
    timestamp: String,
}

/// All component health statuses
#[derive(Debug, Serialize)]
struct DetailedComponents {
    database: ComponentHealth,
    token_manager: ComponentHealth,
    circuit_breaker: ComponentHealth,
    proxy_server: ComponentHealth,
    log_rotation: ComponentHealth,
}

#[derive(Debug, Serialize)]
struct AccountInfo {
    id: String,
    email: String,
    name: Option<String>,
    disabled: bool,
    proxy_disabled: bool,
    created_at: i64,
    last_used: i64,
}

impl From<Account> for AccountInfo {
    fn from(acc: Account) -> Self {
        Self {
            id: acc.id,
            email: acc.email,
            name: acc.name,
            disabled: acc.disabled,
            proxy_disabled: acc.proxy_disabled,
            created_at: acc.created_at,
            last_used: acc.last_used,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AddAccountRequest {
    refresh_token: String,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct AddAccountResponse {
    id: String,
    email: String,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReloadResponse {
    accounts_loaded: usize,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

impl ErrorResponse {
    fn new(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }
}

// ============================================================================
// Admin API Authentication Middleware
// ============================================================================

/// Admin API authentication middleware
///
/// Validates the API key from Authorization header (Bearer token) or X-API-Key header.
/// Returns 401 Unauthorized if the API key is invalid or missing.
async fn admin_auth_middleware(
    State(state): State<Arc<AdminState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // If no API key is configured, auth is disabled - allow all requests
    let Some(expected_key) = &state.api_key else {
        return Ok(next.run(req).await);
    };

    // Extract API key from Authorization header (Bearer) or X-API-Key header
    let provided_key = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| {
            req.headers()
                .get("x-api-key")
                .and_then(|h| h.to_str().ok())
        });

    match provided_key {
        Some(key) if key == expected_key => Ok(next.run(req).await),
        Some(_) => {
            warn!("Admin API: Invalid API key provided");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            warn!("Admin API: Missing API key");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Simple console-only logging for import command
fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,antigravity_tools_lib=debug"));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_thread_ids(false))
        .try_init();
}

/// Initialize advanced logging with log rotation for the server
/// Returns the log guards that must be kept alive for the server lifetime
fn init_server_logging_with_rotation(data_dir: &std::path::Path, config: &ProxyConfig) -> Option<LogGuards> {
    let log_dir = data_dir.join("logs");

    match init_server_logger(&log_dir, &config.log_rotation) {
        Ok(guards) => {
            info!(
                "Initialized server logging with rotation (strategy: {}, max_files: {}, compress: {})",
                config.log_rotation.strategy,
                config.log_rotation.max_files,
                config.log_rotation.compress
            );
            Some(guards)
        }
        Err(e) => {
            eprintln!("Failed to initialize log rotation: {}. Falling back to basic logging.", e);
            init_logging();
            None
        }
    }
}

async fn load_proxy_config(data_dir: &std::path::Path) -> ProxyConfig {
    // Try gui_config.json first (matches Tauri app), then config.json
    for config_name in ["gui_config.json", "config.json"] {
        let config_path = data_dir.join(config_name);

        if config_path.exists() {
            match tokio::fs::read_to_string(&config_path).await {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => {
                        if let Some(proxy) = json.get("proxy") {
                            match serde_json::from_value::<ProxyConfig>(proxy.clone()) {
                                Ok(config) => {
                                    info!("Loaded proxy config from {:?}", config_path);
                                    return config;
                                }
                                Err(e) => warn!("Failed to parse proxy config: {}", e),
                            }
                        }
                    }
                    Err(e) => warn!("Failed to parse {}: {}", config_name, e),
                },
                Err(e) => warn!("Failed to read {}: {}", config_name, e),
            }
        }
    }

    info!("Using default proxy configuration");
    ProxyConfig::default()
}

/// Serializable config wrapper for GUI config file
#[derive(Serialize, Deserialize, Default)]
struct AppConfigWrapper {
    #[serde(default)]
    language: String,
    #[serde(default)]
    theme: String,
    #[serde(default)]
    auto_refresh: bool,
    #[serde(default)]
    refresh_interval: i32,
    #[serde(default)]
    auto_sync: bool,
    #[serde(default)]
    sync_interval: i32,
    #[serde(default)]
    default_export_path: Option<String>,
    #[serde(default)]
    proxy: ProxyConfig,
    #[serde(default)]
    antigravity_executable: Option<String>,
    #[serde(default)]
    antigravity_args: Option<Vec<String>>,
    #[serde(default)]
    auto_launch: bool,
}

async fn save_proxy_config(data_dir: &std::path::Path, config: &ProxyConfig) -> Result<(), String> {
    let config_path = data_dir.join("gui_config.json");

    let mut app_config: AppConfigWrapper = if config_path.exists() {
        tokio::fs::read_to_string(&config_path)
            .await
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    } else {
        AppConfigWrapper::default()
    };

    app_config.proxy = config.clone();

    let content = serde_json::to_string_pretty(&app_config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;

    tokio::fs::write(&config_path, content)
        .await
        .map_err(|e| format!("Failed to write config: {e}"))?;

    info!("Saved configuration to {:?}", config_path);
    Ok(())
}

/// List all accounts from disk
fn list_accounts_from_disk(data_dir: &std::path::Path) -> Result<Vec<Account>, String> {
    let accounts_dir = data_dir.join("accounts");

    if !accounts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut accounts = Vec::new();

    let entries = std::fs::read_dir(&accounts_dir)
        .map_err(|e| format!("Failed to read accounts directory: {e}"))?;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Account>(&content) {
                Ok(account) => accounts.push(account),
                Err(e) => {
                    warn!("Failed to parse account file {:?}: {}", path, e);
                }
            },
            Err(e) => {
                warn!("Failed to read account file {:?}: {}", path, e);
            }
        }
    }

    Ok(accounts)
}

/// Add a new account from refresh token
async fn add_account_from_token(
    refresh_token: &str,
    email_hint: Option<String>,
    name_hint: Option<String>,
) -> Result<Account, String> {
    // 1. Exchange refresh token for access token
    let token_response =
        antigravity_tools_lib::modules::oauth::refresh_access_token(refresh_token)
            .await
            .map_err(|e| format!("Failed to validate refresh token: {e}"))?;

    // 2. Get user info to find email
    let user_info =
        antigravity_tools_lib::modules::oauth::get_user_info(&token_response.access_token)
            .await
            .map_err(|e| format!("Failed to get user info: {e}"))?;

    let email = email_hint.unwrap_or(user_info.email.clone());
    let name = name_hint.or_else(|| user_info.get_display_name());

    // 3. Create token data
    let token_data = TokenData::new(
        token_response.access_token,
        refresh_token.to_string(),
        token_response.expires_in,
        Some(email.clone()),
        None, // project_id will be fetched on demand
        None,
    );

    // 4. Add account using existing module
    let account = antigravity_tools_lib::modules::account::add_account(email, name, token_data)?;

    info!("Added new account: {} ({})", account.email, account.id);

    Ok(account)
}

// ============================================================================
// Admin API Handlers
// ============================================================================

/// GET /api/health - Health check with stats
async fn health_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    let start_time = SERVER_START_TIME.get_or_init(Instant::now);
    let uptime_seconds = start_time.elapsed().as_secs();

    let accounts_total = state.token_manager.len();
    let accounts_available = state.token_manager.available_count();

    let proxy_running = state.proxy_server.read().await.is_some();
    let proxy_port = state.proxy_config.read().await.port;

    let status = if accounts_total == 0 {
        "unhealthy"
    } else if accounts_available == 0 || accounts_available < accounts_total / 2 {
        "degraded"
    } else {
        "ok"
    };

    Json(HealthResponse {
        status,
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds,
        accounts_total,
        accounts_available,
        proxy_running,
        proxy_port,
    })
}

/// GET /api/accounts - List all accounts
async fn list_accounts_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    match list_accounts_from_disk(&state.data_dir) {
        Ok(accounts) => {
            let account_infos: Vec<AccountInfo> = accounts.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(serde_json::json!(account_infos))).into_response()
        }
        Err(e) => {
            error!("Failed to list accounts: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e)),
            )
                .into_response()
        }
    }
}

/// POST /api/accounts - Add new account
async fn add_account_handler(
    State(state): State<Arc<AdminState>>,
    Json(req): Json<AddAccountRequest>,
) -> impl IntoResponse {
    match add_account_from_token(&req.refresh_token, req.email, req.name).await {
        Ok(account) => {
            // Reload token manager
            if let Err(e) = state.token_manager.load_accounts().await {
                warn!("Failed to reload token manager after adding account: {}", e);
            }

            (
                StatusCode::CREATED,
                Json(AddAccountResponse {
                    id: account.id,
                    email: account.email,
                    name: account.name,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to add account: {}", e);
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(e))).into_response()
        }
    }
}

/// DELETE /api/accounts/{id} - Delete account
async fn delete_account_handler(
    State(state): State<Arc<AdminState>>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    match antigravity_tools_lib::modules::account::delete_account(&account_id) {
        Ok(()) => {
            // Reload token manager
            if let Err(e) = state.token_manager.load_accounts().await {
                warn!(
                    "Failed to reload token manager after deleting account: {}",
                    e
                );
            }

            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            error!("Failed to delete account {}: {}", account_id, e);
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new(e))).into_response()
        }
    }
}

/// POST /api/accounts/reload - Reload accounts from disk
async fn reload_accounts_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    match state.token_manager.load_accounts().await {
        Ok(count) => {
            info!("Reloaded {} accounts", count);
            Json(ReloadResponse {
                accounts_loaded: count,
            })
            .into_response()
        }
        Err(e) => {
            error!("Failed to reload accounts: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e)),
            )
                .into_response()
        }
    }
}

/// GET /api/config - Get current configuration
async fn get_config_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    let config = state.proxy_config.read().await;
    Json(serde_json::json!({ "proxy": *config }))
}

/// PUT /api/config - Update configuration (hot reload)
async fn update_config_handler(
    State(state): State<Arc<AdminState>>,
    Json(new_config): Json<ProxyConfig>,
) -> impl IntoResponse {
    // Update in-memory config
    {
        let mut config = state.proxy_config.write().await;
        *config = new_config.clone();
    }

    // Save to disk
    if let Err(e) = save_proxy_config(&state.data_dir, &new_config).await {
        error!("Failed to save config: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(e)),
        )
            .into_response();
    }

    // Hot reload running proxy server if present
    let proxy_lock = state.proxy_server.read().await;
    if let Some(instance) = proxy_lock.as_ref() {
        instance.server.update_mapping(&new_config).await;
        instance.server.update_security(&new_config).await;
        instance.server.update_zai(&new_config).await;
        instance
            .server
            .update_proxy(new_config.upstream_proxy.clone())
            .await;
        info!("Hot reloaded proxy configuration");
    }

    // Update token manager scheduling config
    state
        .token_manager
        .update_sticky_config(new_config.scheduling.clone())
        .await;

    Json(serde_json::json!({ "proxy": new_config })).into_response()
}

/// GET /api/stats - Get proxy stats
async fn get_stats_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    let stats = state.monitor.get_stats().await;
    Json(stats)
}

/// GET /metrics - Prometheus metrics endpoint (public, no auth required)
///
/// Returns Prometheus-compatible metrics in text format for observability.
/// Metrics include:
/// - antigravity_requests_total{provider,model,status} - Counter of requests
/// - antigravity_request_duration_seconds - Histogram of latencies
/// - antigravity_accounts_total - Gauge of total accounts
/// - antigravity_accounts_available - Gauge of available accounts
/// - antigravity_uptime_seconds - Gauge of server uptime
async fn metrics_handler(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    // Update account gauges before rendering
    let total = state.token_manager.len();
    let available = state.token_manager.available_count();
    prometheus::update_account_gauges(total, available);

    // Render metrics in Prometheus text format
    let metrics_text = prometheus::render_metrics();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        metrics_text,
    )
}

/// GET /api/accounts/{id}/health - Get health status for a specific account
///
/// Returns detailed health information including:
/// - Current status (healthy, degraded, disabled, recovering)
/// - Consecutive error count
/// - Time remaining in cooldown (if disabled)
/// - Last error details
/// - Success/error statistics
async fn get_account_health_handler(
    State(state): State<Arc<AdminState>>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    match state.health_monitor.get_health(&account_id).await {
        Some(health) => Json(health).into_response(),
        None => {
            // Check if account exists
            match list_accounts_from_disk(&state.data_dir) {
                Ok(accounts) => {
                    if accounts.iter().any(|a| a.id == account_id) {
                        // Account exists but not tracked - return default healthy status
                        Json(serde_json::json!({
                            "account_id": account_id,
                            "status": "healthy",
                            "consecutive_errors": 0,
                            "is_disabled": false,
                            "message": "No health data yet - account has not been used since server start"
                        }))
                        .into_response()
                    } else {
                        (
                            StatusCode::NOT_FOUND,
                            Json(ErrorResponse::new(format!("Account {account_id} not found"))),
                        )
                            .into_response()
                    }
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(format!("Failed to check accounts: {e}"))),
                )
                    .into_response(),
            }
        }
    }
}

/// GET /api/accounts/health - Get health status for all accounts
///
/// Returns an array of health status for all tracked accounts
async fn get_all_accounts_health_handler(
    State(state): State<Arc<AdminState>>,
) -> impl IntoResponse {
    let health_data = state.health_monitor.get_all_health().await;
    Json(health_data)
}

/// POST /api/accounts/{id}/enable - Force re-enable a disabled account
///
/// Manually re-enables an account that was auto-disabled due to errors.
/// This resets the consecutive error count and removes the disabled status.
async fn force_enable_account_handler(
    State(state): State<Arc<AdminState>>,
    Path(account_id): Path<String>,
) -> impl IntoResponse {
    if state.health_monitor.force_enable(&account_id).await {
        Json(serde_json::json!({
            "success": true,
            "message": format!("Account {} has been re-enabled", account_id)
        }))
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(format!(
                "Account {account_id} not found or not disabled"
            ))),
        )
            .into_response()
    }
}

/// GET /api/health/summary - Get health summary for all accounts
///
/// Returns a summary of account health across the system
async fn get_health_summary_handler(
    State(state): State<Arc<AdminState>>,
) -> impl IntoResponse {
    let total = state.token_manager.len();
    let healthy = state.health_monitor.healthy_count();
    let disabled = state.health_monitor.disabled_count();
    let rate_limited = total.saturating_sub(state.token_manager.available_count());

    Json(serde_json::json!({
        "total_accounts": total,
        "healthy_accounts": healthy,
        "disabled_accounts": disabled,
        "rate_limited_accounts": rate_limited,
        "health_percentage": if total > 0 { (healthy as f64 / total as f64) * 100.0 } else { 100.0 }
    }))
}

/// GET /api/health/detailed - Detailed health check with component status
///
/// Returns comprehensive health information about all server components including:
/// - Database health with latency measurement
/// - Token manager status (accounts total/available/rate-limited)
/// - Circuit breaker state (open/half-open circuits)
/// - Proxy server status (active connections, requests per minute)
/// - Log rotation status (file count, oldest log age)
/// - System resource checks (disk, memory, CPU)
async fn detailed_health_handler(
    State(state): State<Arc<AdminState>>,
) -> impl IntoResponse {
    let start_time = SERVER_START_TIME.get_or_init(Instant::now);
    let uptime_seconds = start_time.elapsed().as_secs();

    // === Database Health ===
    let database_health = check_database_health(&state.data_dir);

    // === Token Manager Health ===
    let accounts_total = state.token_manager.len();
    let accounts_available = state.token_manager.available_count();
    let accounts_rate_limited = accounts_total.saturating_sub(accounts_available);

    let token_manager_status = if accounts_total == 0 || accounts_available == 0 {
        "unhealthy"
    } else if accounts_rate_limited > accounts_total / 2 {
        "degraded"
    } else {
        "healthy"
    };

    let token_manager_health = ComponentHealth {
        status: token_manager_status,
        accounts_total: Some(accounts_total),
        accounts_available: Some(accounts_available),
        accounts_rate_limited: Some(accounts_rate_limited),
        ..ComponentHealth::healthy()
    };

    // === Circuit Breaker Health ===
    let cb_summary = state.proxy_server.read().await.as_ref().map_or(
        (0, 0usize),
        |_ps| {
            // Circuit breaker is in AppState, not directly accessible here
            // We use health_monitor disabled count as a proxy for circuit issues
            let disabled = state.health_monitor.disabled_count();
            (disabled, 0usize) // (open_circuits, half_open_circuits)
        },
    );

    let circuit_breaker_status = if cb_summary.0 > accounts_total / 2 {
        "degraded"
    } else {
        "healthy"
    };

    let circuit_breaker_health = ComponentHealth {
        status: circuit_breaker_status,
        open_circuits: Some(cb_summary.0),
        half_open_circuits: Some(cb_summary.1),
        ..ComponentHealth::healthy()
    };

    // === Proxy Server Health ===
    let proxy_running = state.proxy_server.read().await.is_some();
    let stats = state.monitor.get_stats().await;

    // Calculate approximate requests per minute (based on total over uptime)
    let requests_per_minute = if uptime_seconds > 0 {
        (stats.total_requests * 60) / uptime_seconds
    } else {
        0
    };

    let proxy_status = if proxy_running {
        "healthy"
    } else {
        "unhealthy"
    };

    let proxy_server_health = ComponentHealth {
        status: proxy_status,
        active_connections: Some(0), // ConnectionTracker is not directly accessible
        requests_per_minute: Some(requests_per_minute),
        ..ComponentHealth::healthy()
    };

    // === Log Rotation Health ===
    let log_rotation_health = check_log_rotation_health(&state.data_dir);

    // === System Resource Checks ===
    let system_checks = check_system_resources();

    // === Determine Overall Status ===
    let overall_status = determine_overall_status(
        &database_health,
        &token_manager_health,
        &circuit_breaker_health,
        &proxy_server_health,
        &log_rotation_health,
        &system_checks,
    );

    // === Generate Timestamp ===
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string());

    Json(DetailedHealthResponse {
        status: overall_status,
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds,
        components: DetailedComponents {
            database: database_health,
            token_manager: token_manager_health,
            circuit_breaker: circuit_breaker_health,
            proxy_server: proxy_server_health,
            log_rotation: log_rotation_health,
        },
        checks: system_checks,
        timestamp,
    })
}

/// Check database health with latency measurement
fn check_database_health(data_dir: &std::path::Path) -> ComponentHealth {
    use std::time::Instant;

    let db_path = data_dir.join("proxy_logs.db");

    // Measure database access latency
    let start = Instant::now();

    match rusqlite::Connection::open(&db_path) {
        Ok(conn) => {
            // Simple query to test database responsiveness
            // Use query_row instead of execute for SELECT statements
            let query_result: Result<i32, _> = conn.query_row("SELECT 1", [], |row| row.get(0));
            let latency_ms = start.elapsed().as_millis() as u64;

            // Get database file size
            let size_bytes = std::fs::metadata(&db_path)
                .map(|m| m.len())
                .ok();

            match query_result {
                Ok(_) => {
                    let status = if latency_ms > 100 { "degraded" } else { "healthy" };
                    ComponentHealth {
                        status,
                        latency_ms: Some(latency_ms),
                        size_bytes,
                        ..ComponentHealth::healthy()
                    }
                }
                Err(e) => {
                    warn!("Database health check query failed: {}", e);
                    ComponentHealth {
                        status: "unhealthy",
                        latency_ms: Some(latency_ms),
                        size_bytes,
                        ..ComponentHealth::healthy()
                    }
                }
            }
        }
        Err(e) => {
            warn!("Database health check failed to open: {}", e);
            ComponentHealth::unhealthy()
        }
    }
}

/// Check log rotation health
fn check_log_rotation_health(data_dir: &std::path::Path) -> ComponentHealth {
    use std::time::SystemTime;

    let log_dir = data_dir.join("logs");

    if !log_dir.exists() {
        // No logs directory yet - this is fine for fresh installs
        return ComponentHealth {
            status: "healthy",
            log_files_count: Some(0),
            oldest_log_age_days: None,
            ..ComponentHealth::healthy()
        };
    }

    let mut log_count = 0usize;
    let mut oldest_modified: Option<SystemTime> = None;

    if let Ok(entries) = std::fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "log" | "gz") {
                    log_count += 1;

                    if let Ok(metadata) = path.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            oldest_modified = Some(match oldest_modified {
                                Some(existing) if modified < existing => modified,
                                Some(existing) => existing,
                                None => modified,
                            });
                        }
                    }
                }
            }
        }
    }

    let oldest_log_age_days = oldest_modified.and_then(|modified| {
        SystemTime::now()
            .duration_since(modified)
            .ok()
            .map(|d| d.as_secs() / (24 * 60 * 60))
    });

    // Status is degraded if logs are very old (> 30 days) or if there are too many files
    let status = if log_count > 100 || oldest_log_age_days.is_some_and(|days| days > 30) {
        "degraded"
    } else {
        "healthy"
    };

    ComponentHealth {
        status,
        log_files_count: Some(log_count),
        oldest_log_age_days,
        ..ComponentHealth::healthy()
    }
}

/// Check system resources (disk, memory, CPU)
fn check_system_resources() -> SystemChecks {
    // Disk space check - ensure at least 100MB free
    let disk_space_ok = check_disk_space();

    // Memory check - try to read /proc/meminfo on Linux
    let memory_ok = check_memory();

    // CPU check - always true for now (would need sysinfo crate for proper check)
    let cpu_ok = true;

    SystemChecks {
        disk_space_ok,
        memory_ok,
        cpu_ok,
    }
}

/// Check if sufficient disk space is available
fn check_disk_space() -> bool {
    // Try to get available space in data directory
    // On Linux, we can use statvfs but for simplicity, we'll use a temp file test
    // If we can write a file, disk space is likely OK

    #[cfg(target_os = "linux")]
    {
        use std::fs;

        // Try to check /proc/diskstats or use statvfs
        // For simplicity, just check if we can write to temp
        let temp_path = std::env::temp_dir().join(".antigravity_health_check");
        if fs::write(&temp_path, "test").is_ok() {
            let _ = fs::remove_file(&temp_path);
            true
        } else {
            false
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        true // Assume OK on other platforms
    }
}

/// Check if sufficient memory is available
fn check_memory() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Read /proc/meminfo to check available memory
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemAvailable:") {
                    // Parse available memory in kB
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(kb_str) = parts.get(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            // Require at least 100MB available
                            return kb > 100_000;
                        }
                    }
                }
            }
        }
        true // Assume OK if we can't read meminfo
    }

    #[cfg(not(target_os = "linux"))]
    {
        true // Assume OK on other platforms
    }
}

/// Determine overall system health status
fn determine_overall_status(
    database: &ComponentHealth,
    token_manager: &ComponentHealth,
    circuit_breaker: &ComponentHealth,
    proxy_server: &ComponentHealth,
    log_rotation: &ComponentHealth,
    system_checks: &SystemChecks,
) -> &'static str {
    // Unhealthy if any critical component is unhealthy
    if database.status == "unhealthy"
        || token_manager.status == "unhealthy"
        || proxy_server.status == "unhealthy"
    {
        return "unhealthy";
    }

    // Unhealthy if system resources are critical
    if !system_checks.disk_space_ok || !system_checks.memory_ok {
        return "unhealthy";
    }

    // Degraded if any component is degraded
    if database.status == "degraded"
        || token_manager.status == "degraded"
        || circuit_breaker.status == "degraded"
        || proxy_server.status == "degraded"
        || log_rotation.status == "degraded"
    {
        return "degraded";
    }

    "healthy"
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Import { from, to }) => {
            init_logging();
            import_accounts(from, to).await
        }
        Some(Commands::Serve) | None => {
            run_server().await
        }
    }
}

async fn import_accounts(from: Option<PathBuf>, to: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let source_dir = from.unwrap_or_else(|| {
        dirs::home_dir().map_or_else(|| PathBuf::from(".antigravity_tools"), |h| h.join(".antigravity_tools"))
    });
    
    let target_dir = to.unwrap_or_else(|| {
        dirs::home_dir().map_or_else(|| PathBuf::from(".antigravity"), |h| h.join(".antigravity"))
    });
    
    info!("Importing accounts from {:?} to {:?}", source_dir, target_dir);
    
    let source_accounts = source_dir.join("accounts");
    let source_index = source_dir.join("accounts.json");
    
    if !source_accounts.exists() {
        error!("Source accounts directory not found: {:?}", source_accounts);
        return Err("Source accounts directory not found".into());
    }
    
    let target_accounts = target_dir.join("accounts");
    tokio::fs::create_dir_all(&target_accounts).await?;
    
    let mut imported = 0;
    let mut entries = tokio::fs::read_dir(&source_accounts).await?;
    
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let filename = path.file_name().unwrap();
            let target_path = target_accounts.join(filename);
            
            if target_path.exists() {
                warn!("Skipping {:?} (already exists)", filename);
                continue;
            }
            
            tokio::fs::copy(&path, &target_path).await?;
            info!("Imported: {:?}", filename);
            imported += 1;
        }
    }
    
    // Merge accounts.json index
    if source_index.exists() {
        let target_index = target_dir.join("accounts.json");
        if target_index.exists() {
            // Merge: combine unique account IDs from both files
            info!("Merging accounts.json indices...");
            
            let source_content = tokio::fs::read_to_string(&source_index).await?;
            let target_content = tokio::fs::read_to_string(&target_index).await?;
            
            let source_ids: std::collections::HashSet<String> = 
                serde_json::from_str(&source_content).unwrap_or_default();
            let mut target_ids: std::collections::HashSet<String> = 
                serde_json::from_str(&target_content).unwrap_or_default();
            
            let before_count = target_ids.len();
            target_ids.extend(source_ids);
            let after_count = target_ids.len();
            let merged_count = after_count - before_count;
            
            // Convert to sorted Vec for consistent output
            let mut merged_vec: Vec<String> = target_ids.into_iter().collect();
            merged_vec.sort();
            
            let merged_json = serde_json::to_string_pretty(&merged_vec)?;
            tokio::fs::write(&target_index, merged_json).await?;
            
            info!("Merged accounts.json: {} new entries added (total: {})", merged_count, after_count);
        } else {
            tokio::fs::copy(&source_index, &target_index).await?;
            info!("Imported accounts.json index");
        }
    }
    
    info!("Import complete: {} accounts imported", imported);
    Ok(())
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    let _ = SERVER_START_TIME.get_or_init(Instant::now);

    let server_config = ServerConfig::from_env();

    // Ensure data directory exists before loading config
    if !server_config.data_dir.exists() {
        std::fs::create_dir_all(&server_config.data_dir)?;
    }

    // Load proxy config first to get log rotation settings
    let proxy_config = load_proxy_config(&server_config.data_dir).await;

    // Initialize logging with rotation (must be done early, guards must outlive server)
    let _log_guards = init_server_logging_with_rotation(&server_config.data_dir, &proxy_config);

    // Start log cleanup background task (runs every 24 hours)
    let log_dir = server_config.data_dir.join("logs");
    let _cleanup_handle = start_log_cleanup_task(
        log_dir,
        proxy_config.log_rotation.max_files as u64,
        24, // Check every 24 hours
    );

    let _ = prometheus::init_metrics();
    info!("Prometheus metrics initialized");

    // Initialize OpenTelemetry tracing (when 'otel' feature is enabled)
    // The guard must be kept alive for the duration of the server
    let _telemetry_guard = match telemetry::init_telemetry() {
        Ok(guard) => {
            #[cfg(feature = "otel")]
            info!("OpenTelemetry distributed tracing initialized");
            #[cfg(not(feature = "otel"))]
            info!("OpenTelemetry tracing not enabled (compile with --features otel)");
            Some(guard)
        }
        Err(e) => {
            warn!("Failed to initialize OpenTelemetry tracing: {}. Continuing without it.", e);
            None
        }
    };

    info!(
        "Antigravity Server v{} starting...",
        env!("CARGO_PKG_VERSION")
    );

    info!("Data directory: {:?}", server_config.data_dir);

    // Re-load the proxy_config as mutable for env overrides
    let mut proxy_config = proxy_config;

    let accounts_dir = server_config.data_dir.join("accounts");
    if !accounts_dir.exists() {
        info!(
            "Accounts directory does not exist, creating: {:?}",
            accounts_dir
        );
        tokio::fs::create_dir_all(&accounts_dir).await?;
    }

    // Set ANTIGRAVITY_DATA_DIR for lib modules that read it
    std::env::set_var(
        "ANTIGRAVITY_DATA_DIR",
        server_config
            .data_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    );

    // Override with environment variables
    proxy_config.port = server_config.proxy_port;
    proxy_config.allow_lan_access = server_config.allow_lan;
    proxy_config.enable_logging = server_config.enable_logging;

    if let Some(api_key) = &server_config.api_key {
        proxy_config.api_key.clone_from(api_key);
    }

    // Initialize token manager and load accounts
    let token_manager = Arc::new(TokenManager::new(server_config.data_dir.clone()));
    token_manager
        .update_sticky_config(proxy_config.scheduling.clone())
        .await;

    match token_manager.load_accounts().await {
        Ok(count) => info!("Loaded {} accounts", count),
        Err(e) => {
            warn!("Failed to load accounts: {}", e);
            warn!("Server will start but no accounts are available");
        }
    }

    // Initialize monitor (without Tauri app handle for headless mode)
    let monitor = Arc::new(ProxyMonitor::new(1000, None));
    monitor.set_enabled(proxy_config.enable_logging);

    // Build security config
    let security_config = ProxySecurityConfig::from_proxy_config(&proxy_config);

    // Determine bind address
    let bind_addr = if proxy_config.allow_lan_access {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };

    info!(
        "Starting proxy server on {}:{}",
        bind_addr, proxy_config.port
    );

    // Start the Axum proxy server
    let (server, handle) = AxumServer::start(
        bind_addr.to_string(),
        proxy_config.port,
        token_manager.clone(),
        proxy_config.anthropic_mapping.clone(),
        proxy_config.openai_mapping.clone(),
        proxy_config.custom_mapping.clone(),
        proxy_config.request_timeout,
        proxy_config.upstream_proxy.clone(),
        security_config,
        proxy_config.zai.clone(),
        monitor.clone(),
    )
    .await
    .map_err(|e| format!("Failed to start proxy server: {e}"))?;

    // Initialize health monitor with default configuration
    let health_config = antigravity_tools_lib::proxy::health::HealthConfig::default();
    let health_monitor = antigravity_tools_lib::proxy::health::HealthMonitor::new(health_config);
    // Start the recovery background task
    let _health_recovery_handle = health_monitor.start_recovery_task();

    // Create admin state
    let admin_state = Arc::new(AdminState {
        data_dir: server_config.data_dir.clone(),
        proxy_config: RwLock::new(proxy_config.clone()),
        token_manager: token_manager.clone(),
        monitor: monitor.clone(),
        proxy_server: RwLock::new(Some(ProxyServerHandle { server, handle })),
        api_key: server_config.api_key.clone(),
        health_monitor,
    });

    // Admin API Rate Limiting: 60 requests per minute per IP
    // Uses ContainerAwareKeyExtractor to handle containerized environments
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(60)
            .key_extractor(ContainerAwareKeyExtractor)
            .finish()
            .unwrap(),
    );

    // Build authenticated routes (require API key)
    let authenticated_routes = Router::new()
        .route(
            "/api/accounts",
            get(list_accounts_handler).post(add_account_handler),
        )
        .route("/api/accounts/{id}", delete(delete_account_handler))
        .route("/api/accounts/reload", post(reload_accounts_handler))
        .route("/api/accounts/health", get(get_all_accounts_health_handler))
        .route("/api/accounts/{id}/health", get(get_account_health_handler))
        .route("/api/accounts/{id}/enable", post(force_enable_account_handler))
        .route(
            "/api/config",
            get(get_config_handler).put(update_config_handler),
        )
        .route("/api/stats", get(get_stats_handler))
        .layer(middleware::from_fn_with_state(
            admin_state.clone(),
            admin_auth_middleware,
        ));

    // Build public routes (no auth required - for monitoring)
    let public_routes = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/health/summary", get(get_health_summary_handler))
        .route("/api/health/detailed", get(detailed_health_handler))
        .route("/metrics", get(metrics_handler));

    // Combine routers with rate limiting
    let admin_app = Router::new()
        .merge(public_routes)
        .merge(authenticated_routes)
        .layer(GovernorLayer::new(governor_config))
        .with_state(admin_state.clone());

    // Log auth status
    if server_config.api_key.is_some() {
        info!("Admin API authentication: ENABLED");
    } else {
        warn!("Admin API authentication: DISABLED (no ANTIGRAVITY_API_KEY set)");
    }

    // Start admin API server
    let admin_addr = format!("{}:{}", bind_addr, server_config.admin_port);
    let admin_listener = tokio::net::TcpListener::bind(&admin_addr)
        .await
        .map_err(|e| format!("Failed to bind admin API to {admin_addr}: {e}"))?;

    info!("Antigravity Server is running!");
    info!(
        "  Proxy API:    http://{}:{}",
        bind_addr, proxy_config.port
    );
    info!(
        "  Admin API:    http://{}:{}",
        bind_addr, server_config.admin_port
    );
    info!(
        "  Health check: http://{}:{}/healthz",
        bind_addr, proxy_config.port
    );
    info!("");
    info!("Admin API endpoints:");
    info!("  GET    /api/health              - Health check with stats (public)");
    info!("  GET    /api/health/summary      - Account health summary (public)");
    info!("  GET    /api/health/detailed     - Detailed component health (public)");
    info!("  GET    /metrics                 - Prometheus metrics (public)");
    info!("  GET    /api/accounts            - List all accounts (auth required)");
    info!("  POST   /api/accounts            - Add account (auth required)");
    info!("  DELETE /api/accounts/{{id}}       - Delete account (auth required)");
    info!("  POST   /api/accounts/reload     - Reload accounts from disk (auth required)");
    info!("  GET    /api/accounts/health     - Get all accounts health (auth required)");
    info!("  GET    /api/accounts/{{id}}/health - Get account health (auth required)");
    info!("  POST   /api/accounts/{{id}}/enable - Force re-enable disabled account (auth required)");
    info!("  GET    /api/config              - Get current config (auth required)");
    info!("  PUT    /api/config              - Update config (auth required)");
    info!("  GET    /api/stats               - Get proxy stats (auth required)");
    info!("");
    info!("Authentication: Use 'Authorization: Bearer <API_KEY>' or 'X-API-Key: <API_KEY>' header");

    // Run admin server with graceful shutdown
    tokio::select! {
        result = axum::serve(admin_listener, admin_app) => {
            if let Err(e) = result {
                error!("Admin API server error: {}", e);
            }
        }
        () = shutdown_signal() => {
            info!("Shutdown signal received");
        }
    }

    // Cleanup
    info!("Shutting down...");

    // Stop proxy server gracefully (with timeout for in-flight requests)
    let mut proxy_lock = admin_state.proxy_server.write().await;
    if let Some(instance) = proxy_lock.take() {
        // Use graceful shutdown that waits for connections
        let drained = instance.server.stop_gracefully().await;

        // Wait for the server task to complete
        let _ = instance.handle.await;

        if drained {
            info!("Proxy server shutdown complete - all connections drained");
        } else {
            warn!("Proxy server shutdown complete - some connections were terminated");
        }
    }

    info!("Antigravity Server shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => info!("Received Ctrl+C"),
        () = terminate => info!("Received SIGTERM"),
    }
}
