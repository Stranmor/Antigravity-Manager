use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use slint::{VecModel, Model};
use antigravity_tools_lib::modules::{account, config, oauth};
use antigravity_tools_lib::proxy::{AxumServer, TokenManager, ProxySecurityConfig};
use antigravity_tools_lib::proxy::monitor::ProxyMonitor;
use tray_icon::{TrayIconBuilder, menu::{Menu, MenuItem, PredefinedMenuItem}};
use tokio::sync::RwLock;
use serde::Serialize;

slint::include_modules!();

// ========================================
// EXPORT DATA STRUCTURES
// ========================================

/// Serializable export data for analytics reports
#[derive(Debug, Clone, Serialize)]
struct AnalyticsExportData {
    /// Export metadata
    pub export_timestamp: String,
    pub export_format: String,

    /// Summary statistics
    pub summary: ExportSummary,

    /// Per-account analytics
    pub accounts: Vec<ExportAccountAnalytics>,
}

#[derive(Debug, Clone, Serialize)]
struct ExportSummary {
    pub total_requests_today: i32,
    pub total_requests_all_time: i32,
    pub overall_success_rate: f32,
    pub total_tokens_used: i32,
    pub active_accounts: i32,
    pub rate_limited_accounts: i32,

    /// Circuit breaker summary
    pub circuit_breaker_closed: i32,
    pub circuit_breaker_open: i32,
    pub circuit_breaker_half_open: i32,
    pub circuit_breaker_total_trips: i32,
}

#[derive(Debug, Clone, Serialize)]
struct ExportAccountAnalytics {
    pub account_id: String,
    pub email: String,
    pub tier: String,
    pub requests_today: i32,
    pub requests_total: i32,
    pub success_count: i32,
    pub error_count: i32,
    pub success_rate: f32,
    pub tokens_used: i32,
    pub rate_limit_hits: i32,
    pub circuit_state: String,
    pub last_request_time: String,
}

/// Throttled stats cache for UI updates.
/// Uses atomics for lock-free access from async context.
/// Updates are batched at 10Hz (100ms interval) to prevent excessive UI re-renders.
struct ThrottledStatsCache {
    total_requests: AtomicU64,
    success_count: AtomicU64,
    error_count: AtomicU64,
    /// Last time stats were pushed to UI (epoch millis)
    last_ui_update: AtomicU64,
}

impl ThrottledStatsCache {
    const THROTTLE_INTERVAL_MS: u64 = 100; // 10Hz max update rate

    fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            last_ui_update: AtomicU64::new(0),
        }
    }

    /// Update cached stats from monitor data.
    /// Returns true if values changed (UI update may be needed).
    fn update(&self, total: u64, success: u64, errors: u64) -> bool {
        let prev_total = self.total_requests.swap(total, Ordering::Relaxed);
        let prev_success = self.success_count.swap(success, Ordering::Relaxed);
        let prev_errors = self.error_count.swap(errors, Ordering::Relaxed);

        prev_total != total || prev_success != success || prev_errors != errors
    }

    /// Check if enough time has passed since last UI update.
    /// Returns true if we should update the UI.
    fn should_update_ui(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let last = self.last_ui_update.load(Ordering::Relaxed);

        if now.saturating_sub(last) >= Self::THROTTLE_INTERVAL_MS {
            self.last_ui_update.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn get_stats(&self) -> (u64, u64, u64) {
        (
            self.total_requests.load(Ordering::Relaxed),
            self.success_count.load(Ordering::Relaxed),
            self.error_count.load(Ordering::Relaxed),
        )
    }
}

struct ProxyState {
    server: Option<AxumServer>,
    handle: Option<tokio::task::JoinHandle<()>>,
    token_manager: Option<Arc<TokenManager>>,
    monitor: Option<Arc<ProxyMonitor>>,
    /// Throttled stats cache for UI updates
    stats_cache: Arc<ThrottledStatsCache>,
    /// Handle to the stats polling task
    stats_poll_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            server: None,
            handle: None,
            token_manager: None,
            monitor: None,
            stats_cache: Arc::new(ThrottledStatsCache::new()),
            stats_poll_handle: None,
        }
    }
}

struct AppController {
    app: slint::Weak<MainWindow>,
    start_time: std::time::Instant,
    proxy_state: Arc<RwLock<ProxyState>>,
}

impl AppController {
    fn new(app: &MainWindow) -> Self {
        Self {
            app: app.as_weak(),
            start_time: std::time::Instant::now(),
            proxy_state: Arc::new(RwLock::new(ProxyState::new())),
        }
    }

    fn load_accounts(&self) {
        match account::list_accounts() {
            Ok(accounts) => {
                let current_id = account::get_current_account_id().ok().flatten();
                
                let accounts_model: Vec<Account> = accounts
                    .iter()
                    .map(|a| self.lib_account_to_slint(a, current_id.as_deref()))
                    .collect();
                
                let mut sorted = accounts.clone();
                sorted.sort_by(|a, b| {
                    let qa = a.quota.as_ref().and_then(|q| q.models.first()).map(|m| m.percentage).unwrap_or(0);
                    let qb = b.quota.as_ref().and_then(|q| q.models.first()).map(|m| m.percentage).unwrap_or(0);
                    qb.cmp(&qa)
                });
                let best: Vec<Account> = sorted.iter()
                    .filter(|a| !a.disabled && a.quota.as_ref().map(|q| !q.is_forbidden).unwrap_or(true))
                    .take(3)
                    .map(|a| self.lib_account_to_slint(a, current_id.as_deref()))
                    .collect();

                let available_count = accounts.iter()
                    .filter(|a| {
                        let is_forbidden = a.quota.as_ref().map(|q| q.is_forbidden).unwrap_or(false);
                        let quota_pct = a.quota.as_ref().and_then(|q| q.models.first()).map(|m| m.percentage).unwrap_or(0);
                        !a.disabled && !is_forbidden && quota_pct > 10
                    })
                    .count() as i32;
                
                let low_quota_count = accounts.iter()
                    .filter(|a| {
                        let quota_pct = a.quota.as_ref().and_then(|q| q.models.first()).map(|m| m.percentage).unwrap_or(0);
                        quota_pct < 20
                    })
                    .count() as i32;
                
                let pro_count = accounts.iter()
                    .filter(|a| {
                        a.quota.as_ref()
                            .and_then(|q| q.subscription_tier.as_ref())
                            .map(|t| t.to_uppercase() == "PRO")
                            .unwrap_or(false)
                    })
                    .count() as i32;
                
                let ultra_count = accounts.iter()
                    .filter(|a| {
                        a.quota.as_ref()
                            .and_then(|q| q.subscription_tier.as_ref())
                            .map(|t| t.to_uppercase() == "ULTRA")
                            .unwrap_or(false)
                    })
                    .count() as i32;
                
                let free_count = accounts.iter()
                    .filter(|a| {
                        let tier = a.quota.as_ref()
                            .and_then(|q| q.subscription_tier.as_ref())
                            .map(|t| t.to_uppercase())
                            .unwrap_or_else(|| "FREE".to_string());
                        tier == "FREE"
                    })
                    .count() as i32;

                if let Some(app) = self.app.upgrade() {
                    let active_count = accounts.iter()
                        .filter(|a| !a.disabled && a.quota.as_ref().map(|q| !q.is_forbidden).unwrap_or(true))
                        .count() as i32;
                    
                    let model = std::rc::Rc::new(VecModel::from(accounts_model.clone()));
                    app.global::<AppState>().set_accounts(model.into());
                    
                    app.global::<AppState>().set_available_count(available_count);
                    app.global::<AppState>().set_low_quota_count(low_quota_count);
                    app.global::<AppState>().set_pro_count(pro_count);
                    app.global::<AppState>().set_ultra_count(ultra_count);
                    app.global::<AppState>().set_free_count(free_count);
                    
                    let current_filter = app.global::<AppState>().get_account_filter().to_string();
                    self.apply_filter_internal(&accounts_model, &current_filter);
                    
                    let best_model = std::rc::Rc::new(VecModel::from(best));
                    app.global::<AppState>().set_best_accounts(best_model.into());
                    
                    if let Some(ref curr_id) = current_id {
                        if let Some(curr) = accounts.iter().find(|a| &a.id == curr_id) {
                            app.global::<AppState>().set_current_account(
                                self.lib_account_to_slint(curr, current_id.as_deref())
                            );
                        }
                    }
                    
                    let mut stats = app.global::<AppState>().get_stats();
                    stats.active_accounts = active_count;
                    app.global::<AppState>().set_stats(stats);
                }
            }
            Err(e) => {
                tracing::error!("Failed to load accounts: {}", e);
                self.show_status("Failed to load accounts", "error");
            }
        }
    }

    fn apply_filter_internal(&self, accounts: &[Account], filter: &str) {
        let filtered: Vec<Account> = accounts
            .iter()
            .filter(|a| {
                match filter {
                    "available" => {
                        a.enabled && !a.is_forbidden && a.quota_percentage > 10
                    }
                    "low_quota" => {
                        a.quota_percentage < 20
                    }
                    "pro" => {
                        a.tier.to_string().to_uppercase() == "PRO"
                    }
                    "ultra" => {
                        a.tier.to_string().to_uppercase() == "ULTRA"
                    }
                    "free" => {
                        let tier = a.tier.to_string().to_uppercase();
                        tier == "FREE" || tier.is_empty()
                    }
                    _ => true,
                }
            })
            .cloned()
            .collect();
        
        if let Some(app) = self.app.upgrade() {
            let model = std::rc::Rc::new(VecModel::from(filtered));
            app.global::<AppState>().set_filtered_accounts(model.into());
        }
    }

    fn filter_accounts(&self, filter: &str) {
        if let Some(app) = self.app.upgrade() {
            app.global::<AppState>().set_account_filter(filter.into());
            
            let accounts: Vec<Account> = app.global::<AppState>().get_accounts()
                .iter()
                .collect();
            
            self.apply_filter_internal(&accounts, filter);
        }
    }

    fn lib_account_to_slint(&self, a: &antigravity_tools_lib::models::Account, current_id: Option<&str>) -> Account {
        let tier = a.quota.as_ref()
            .and_then(|q| q.subscription_tier.clone())
            .unwrap_or_else(|| "FREE".into())
            .to_uppercase();
        
        let quota_pct = a.quota.as_ref()
            .and_then(|q| q.models.first())
            .map(|m| m.percentage)
            .unwrap_or(0);
        
        let is_forbidden = a.quota.as_ref().map(|q| q.is_forbidden).unwrap_or(false);
        
        let model_quotas: Vec<ModelQuota> = a.quota.as_ref()
            .map(|q| q.models.iter().map(|m| ModelQuota {
                model_name: m.name.clone().into(),
                percentage: m.percentage,
                remaining: 0,
                total: 100,
                reset_time: m.reset_time.clone().into(),
            }).collect())
            .unwrap_or_default();
        
        Account {
            id: a.id.clone().into(),
            email: a.email.clone().into(),
            display_name: a.name.clone().unwrap_or_default().into(),
            tier: tier.into(),
            quota_percentage: quota_pct,
            is_current: current_id == Some(&a.id),
            enabled: !a.disabled,
            disabled: a.disabled,
            proxy_disabled: a.proxy_disabled,
            is_forbidden,
            last_refresh: a.quota.as_ref()
                .map(|q| format_relative_time(q.last_updated))
                .unwrap_or_else(|| "never".into())
                .into(),
            model_quotas: std::rc::Rc::new(VecModel::from(model_quotas)).into(),
        }
    }

    fn toggle_account(&self, account_id: &str) {
        if let Ok(mut acc) = account::load_account(account_id) {
            acc.disabled = !acc.disabled;
            if let Err(e) = account::save_account(&acc) {
                tracing::error!("Failed to save account: {}", e);
                self.show_status("Failed to toggle account", "error");
                return;
            }
            self.load_accounts();
            self.show_status(
                &format!("Account {}", if acc.disabled { "disabled" } else { "enabled" }),
                "success"
            );
        }
    }

    fn delete_account(&self, account_id: &str) {
        if let Err(e) = account::delete_account(account_id) {
            tracing::error!("Failed to delete account: {}", e);
            self.show_status("Failed to delete account", "error");
            return;
        }
        self.load_accounts();
        self.show_status("Account deleted", "success");
    }

    fn switch_account(&self, account_id: &str) {
        let id = account_id.to_string();
        let app_weak = self.app.clone();
        
        tokio::spawn(async move {
            match account::switch_account(&id).await {
                Ok(_) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.load_accounts();
                            ctrl.show_status("Account switched", "success");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to switch account: {}", e);
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.show_status(&format!("Switch failed: {}", err_msg), "error");
                        }
                    });
                }
            }
        });
    }

    fn refresh_quota(&self, account_id: &str) {
        let id = account_id.to_string();
        let app_weak = self.app.clone();
        
        self.show_status("Refreshing quota...", "info");
        
        tokio::spawn(async move {
            match account::load_account(&id) {
                Ok(mut acc) => {
                    match account::fetch_quota_with_retry(&mut acc).await {
                        Ok(quota) => {
                            let _ = account::update_account_quota(&id, quota);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = app_weak.upgrade() {
                                    let ctrl = AppController::new(&app);
                                    ctrl.load_accounts();
                                    ctrl.show_status("Quota refreshed", "success");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Failed to refresh quota: {}", e);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = app_weak.upgrade() {
                                    let ctrl = AppController::new(&app);
                                    ctrl.show_status("Failed to refresh quota", "error");
                                }
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load account: {}", e);
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.show_status(&format!("Failed to load account: {}", err_msg), "error");
                        }
                    });
                }
            }
        });
    }

    fn refresh_all_quotas(&self) {
        let app_weak = self.app.clone();
        
        self.show_status("Refreshing all quotas...", "info");
        
        tokio::spawn(async move {
            let accounts = match account::list_accounts() {
                Ok(a) => a,
                Err(_) => return,
            };
            
            let mut success = 0;
            let mut failed = 0;
            
            for mut acc in accounts {
                if acc.disabled { continue; }
                if acc.quota.as_ref().map(|q| q.is_forbidden).unwrap_or(false) { continue; }
                
                match account::fetch_quota_with_retry(&mut acc).await {
                    Ok(quota) => {
                        let _ = account::update_account_quota(&acc.id, quota);
                        success += 1;
                    }
                    Err(_) => { failed += 1; }
                }
            }
            
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak.upgrade() {
                    let ctrl = AppController::new(&app);
                    ctrl.load_accounts();
                    ctrl.show_status(
                        &format!("Refreshed: {} success, {} failed", success, failed),
                        if failed > 0 { "warning" } else { "success" }
                    );
                }
            });
        });
    }

    fn toggle_proxy(&self) {
        let proxy_state = self.proxy_state.clone();
        let app_weak = self.app.clone();

        tokio::spawn(async move {
            let mut state = proxy_state.write().await;
            let is_running = state.server.is_some();

            if is_running {
                // Stop stats polling task first
                if let Some(poll_handle) = state.stats_poll_handle.take() {
                    poll_handle.abort();
                }
                if let Some(server) = state.server.take() {
                    server.stop();
                }
                if let Some(handle) = state.handle.take() {
                    handle.abort();
                }
                state.token_manager = None;
                state.monitor = None;

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak.upgrade() {
                        app.global::<AppState>().set_proxy_running(false);
                        let ctrl = AppController::new(&app);
                        ctrl.show_status("Proxy stopped", "success");
                    }
                });
            } else {
                let cfg = match config::load_app_config() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to load config: {}", e);
                        let err_msg = e.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                let ctrl = AppController::new(&app);
                                ctrl.show_status(&format!("Config error: {}", err_msg), "error");
                            }
                        });
                        return;
                    }
                };

                let data_dir = match account::get_data_dir() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Failed to get data dir: {}", e);
                        let err_msg = e.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                let ctrl = AppController::new(&app);
                                ctrl.show_status(&format!("Data dir error: {}", err_msg), "error");
                            }
                        });
                        return;
                    }
                };

                let token_manager = Arc::new(TokenManager::new(data_dir));
                token_manager.update_sticky_config(cfg.proxy.scheduling.clone()).await;

                if let Err(e) = token_manager.load_accounts().await {
                    tracing::warn!("Failed to load accounts for proxy: {}", e);
                }

                let monitor = Arc::new(ProxyMonitor::new(1000, None));
                monitor.set_enabled(cfg.proxy.enable_logging);

                let security_config = ProxySecurityConfig::from_proxy_config(&cfg.proxy);

                let bind_addr = if cfg.proxy.allow_lan_access { "0.0.0.0" } else { "127.0.0.1" };

                match AxumServer::start(
                    bind_addr.to_string(),
                    cfg.proxy.port,
                    token_manager.clone(),
                    cfg.proxy.anthropic_mapping.clone(),
                    cfg.proxy.openai_mapping.clone(),
                    cfg.proxy.custom_mapping.clone(),
                    cfg.proxy.request_timeout,
                    cfg.proxy.upstream_proxy.clone(),
                    security_config,
                    cfg.proxy.zai.clone(),
                    monitor.clone(),
                ).await {
                    Ok((server, handle)) => {
                        state.server = Some(server);
                        state.handle = Some(handle);
                        state.token_manager = Some(token_manager);
                        state.monitor = Some(monitor.clone());

                        // Start throttled stats polling task (10Hz = 100ms interval)
                        let stats_cache = state.stats_cache.clone();
                        let monitor_for_poll = monitor;
                        let app_weak_for_poll = app_weak.clone();
                        let stats_poll_handle = tokio::spawn(async move {
                            let mut interval = tokio::time::interval(
                                std::time::Duration::from_millis(ThrottledStatsCache::THROTTLE_INTERVAL_MS)
                            );
                            loop {
                                interval.tick().await;

                                // Fetch stats from monitor
                                let monitor_stats = monitor_for_poll.get_stats().await;

                                // Update cache and check if changed
                                let changed = stats_cache.update(
                                    monitor_stats.total_requests,
                                    monitor_stats.success_count,
                                    monitor_stats.error_count,
                                );

                                // Only push to UI if values changed AND throttle allows
                                if changed && stats_cache.should_update_ui() {
                                    let (total, success, errors) = stats_cache.get_stats();
                                    let app_weak_inner = app_weak_for_poll.clone();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(app) = app_weak_inner.upgrade() {
                                            let mut stats = app.global::<AppState>().get_stats();
                                            stats.total_requests = total as i32;
                                            stats.success_count = success as i32;
                                            stats.error_count = errors as i32;
                                            // Calculate success rate
                                            if total > 0 {
                                                stats.success_rate = success as f32 / total as f32;
                                            }
                                            app.global::<AppState>().set_stats(stats);
                                        }
                                    });
                                }
                            }
                        });
                        state.stats_poll_handle = Some(stats_poll_handle);

                        let port = cfg.proxy.port;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                app.global::<AppState>().set_proxy_running(true);
                                app.global::<AppState>().set_listen_port(port as i32);
                                let ctrl = AppController::new(&app);
                                ctrl.show_status(&format!("Proxy started on port {}", port), "success");
                            }
                        });
                        tracing::info!("Proxy started on {}:{}", bind_addr, port);
                    }
                    Err(e) => {
                        tracing::error!("Failed to start proxy: {}", e);
                        let err_msg = e.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                app.global::<AppState>().set_proxy_running(false);
                                let ctrl = AppController::new(&app);
                                ctrl.show_status(&format!("Proxy failed: {}", err_msg), "error");
                            }
                        });
                    }
                }
            }
        });
    }

    fn load_config(&self) {
        match config::load_app_config() {
            Ok(cfg) => {
                if let Some(app) = self.app.upgrade() {
                    app.global::<AppState>().set_listen_port(cfg.proxy.port as i32);
                    app.global::<AppState>().set_admin_port(9101);
                    app.global::<AppState>().set_auto_start(cfg.proxy.auto_start);
                    app.global::<AppState>().set_allow_lan(cfg.proxy.allow_lan_access);
                    
                    let require_auth = !matches!(cfg.proxy.auth_mode, antigravity_tools_lib::proxy::ProxyAuthMode::Off);
                    app.global::<AppState>().set_require_auth(require_auth);
                    app.global::<AppState>().set_api_key(cfg.proxy.api_key.clone().into());
                    app.global::<AppState>().set_scheduling_mode("cache_first".into());
                    
                    app.global::<AppState>().set_theme(cfg.theme.clone().into());
                    app.global::<AppState>().set_language(cfg.language.clone().into());
                    app.global::<AppState>().set_page_size(20);
                    
                    if let Ok(data_dir) = account::get_data_dir() {
                        app.global::<AppState>().set_data_dir(data_dir.to_string_lossy().to_string().into());
                    }
                    
                    let dark = cfg.theme != "light";
                    app.global::<AppState>().set_dark_mode(dark);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load config, using defaults: {}", e);
            }
        }
    }

    fn save_proxy_config(&self) {
        if let Some(app) = self.app.upgrade() {
            match config::load_app_config() {
                Ok(mut cfg) => {
                    cfg.proxy.port = app.global::<AppState>().get_listen_port() as u16;
                    cfg.proxy.auto_start = app.global::<AppState>().get_auto_start();
                    cfg.proxy.allow_lan_access = app.global::<AppState>().get_allow_lan();
                    cfg.proxy.api_key = app.global::<AppState>().get_api_key().to_string();
                    
                    if let Err(e) = config::save_app_config(&cfg) {
                        tracing::error!("Failed to save config: {}", e);
                        self.show_status("Failed to save config", "error");
                        return;
                    }
                    self.show_status("Configuration saved", "success");
                }
                Err(e) => {
                    tracing::error!("Failed to load config for update: {}", e);
                }
            }
        }
    }

    fn save_app_config(&self) {
        if let Some(app) = self.app.upgrade() {
            match config::load_app_config() {
                Ok(mut cfg) => {
                    cfg.theme = app.global::<AppState>().get_theme().to_string();
                    cfg.language = app.global::<AppState>().get_language().to_string();
                    
                    if let Err(e) = config::save_app_config(&cfg) {
                        tracing::error!("Failed to save config: {}", e);
                        self.show_status("Failed to save settings", "error");
                        return;
                    }
                    self.show_status("Settings saved", "success");
                }
                Err(e) => {
                    tracing::error!("Failed to load config for update: {}", e);
                }
            }
        }
    }

    fn add_account_manual(&self, refresh_token: &str) {
        let token = refresh_token.to_string();
        let app_weak = self.app.clone();
        
        self.show_status("Adding account...", "info");
        
        tokio::spawn(async move {
            match oauth::refresh_access_token(&token).await {
                Ok(token_res) => {
                    match oauth::get_user_info(&token_res.access_token).await {
                        Ok(user_info) => {
                            let token_data = antigravity_tools_lib::models::TokenData::new(
                                token_res.access_token,
                                token,
                                token_res.expires_in,
                                Some(user_info.email.clone()),
                                None,
                                None,
                            );
                            
                            match account::upsert_account(
                                user_info.email.clone(),
                                user_info.get_display_name(),
                                token_data,
                            ) {
                                Ok(_) => {
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(app) = app_weak.upgrade() {
                                            app.global::<AppState>().set_show_add_account_dialog(false);
                                            let ctrl = AppController::new(&app);
                                            ctrl.load_accounts();
                                            ctrl.show_status("Account added successfully", "success");
                                        }
                                    });
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(app) = app_weak.upgrade() {
                                            let ctrl = AppController::new(&app);
                                            ctrl.show_status(&format!("Failed: {}", err_msg), "error");
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = app_weak.upgrade() {
                                    let ctrl = AppController::new(&app);
                                    ctrl.show_status(&format!("Failed: {}", err_msg), "error");
                                }
                            });
                        }
                    }
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.show_status(&format!("Invalid token: {}", err_msg), "error");
                        }
                    });
                }
            }
        });
    }

    fn import_from_db(&self) {
        let app_weak = self.app.clone();
        
        self.show_status("Importing from IDE...", "info");
        
        tokio::spawn(async move {
            match antigravity_tools_lib::modules::migration::import_from_db().await {
                Ok(acc) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            app.global::<AppState>().set_show_add_account_dialog(false);
                            let ctrl = AppController::new(&app);
                            ctrl.load_accounts();
                            ctrl.show_status(&format!("Imported: {}", acc.email), "success");
                        }
                    });
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.show_status(&format!("Import failed: {}", err_msg), "error");
                        }
                    });
                }
            }
        });
    }

    fn generate_api_key(&self) {
        use rand::Rng;
        let key: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        
        if let Some(app) = self.app.upgrade() {
            app.global::<AppState>().set_api_key(format!("sk-{}", key).into());
        }
        self.show_status("API key generated", "success");
    }

    fn toggle_dark_mode(&self) {
        if let Some(app) = self.app.upgrade() {
            let dark = app.global::<AppState>().get_dark_mode();
            tracing::info!("Theme: {}", if dark { "dark" } else { "light" });
        }
    }

    fn open_data_folder(&self) {
        if let Ok(path) = account::get_data_dir() {
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&path).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer").arg(&path).spawn();
        }
    }

    fn copy_to_clipboard(&self, text: &str) {
        // arboard::Clipboard is NOT Send on Linux (X11/Wayland), so we cannot use tokio::spawn
        // We must create a fresh clipboard instance on each call (cheap operation)
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                match clipboard.set_text(text) {
                    Ok(()) => {
                        tracing::debug!("Copied to clipboard: {} chars", text.len());
                        self.show_status("Copied to clipboard", "success");
                    }
                    Err(e) => {
                        tracing::error!("Failed to copy to clipboard: {}", e);
                        self.show_status("Failed to copy to clipboard", "error");
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to access clipboard: {}", e);
                self.show_status("Clipboard not available", "error");
            }
        }
    }

    fn clear_logs(&self) {
        if let Some(app) = self.app.upgrade() {
            let empty: Vec<RequestLog> = vec![];
            app.global::<AppState>().set_request_logs(std::rc::Rc::new(VecModel::from(empty)).into());
        }
        self.show_status("Logs cleared", "success");
    }

    fn toggle_monitor_recording(&self) {
        if let Some(app) = self.app.upgrade() {
            let current = app.global::<AppState>().get_monitor_recording();
            app.global::<AppState>().set_monitor_recording(!current);
        }
    }

    fn show_status(&self, message: &str, status_type: &str) {
        let msg = message.to_string();
        let typ = status_type.to_string();
        let app_weak = self.app.clone();
        
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app_weak.upgrade() {
                app.global::<AppState>().set_status_message(msg.clone().into());
                app.global::<AppState>().set_status_type(typ.into());
                
                let app_weak2 = app.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_secs(3), move || {
                    if let Some(app) = app_weak2.upgrade() {
                        app.global::<AppState>().set_status_message("".into());
                    }
                });
            }
        });
    }

    fn refresh_analytics(&self) {
        let proxy_state = self.proxy_state.clone();
        let app_weak = self.app.clone();

        self.show_status("Refreshing analytics...", "info");

        tokio::spawn(async move {
            let state = proxy_state.read().await;

            // Get monitor stats if proxy is running
            let (total_requests, success_count, _error_count) = if let Some(ref monitor) = state.monitor {
                let stats = monitor.get_stats().await;
                (stats.total_requests, stats.success_count, stats.error_count)
            } else {
                (0, 0, 0)
            };

            // Calculate success rate
            let success_rate = if total_requests > 0 {
                success_count as f32 / total_requests as f32
            } else {
                0.0
            };

            // Create analytics summary
            // Note: For now, we use the overall stats since per-account tracking
            // would require additional infrastructure in the proxy
            let summary = AnalyticsSummary {
                total_requests_today: total_requests as i32, // TODO: Filter by today's date
                total_requests_all_time: total_requests as i32,
                overall_success_rate: success_rate,
                total_tokens_used: 0, // TODO: Aggregate from logs
                active_accounts: 0, // Will be set from accounts
                rate_limited_accounts: 0, // TODO: Track rate-limited accounts
                circuit_breaker: CircuitBreakerSummary {
                    closed_count: 0,
                    open_count: 0,
                    half_open_count: 0,
                    total_trips: 0,
                },
            };

            // Get account list for per-account analytics
            let accounts = antigravity_tools_lib::modules::account::list_accounts()
                .unwrap_or_default();

            let active_count = accounts.iter()
                .filter(|a| !a.disabled && a.quota.as_ref().map(|q| !q.is_forbidden).unwrap_or(true))
                .count() as i32;

            // Generate per-account analytics (placeholder data for now)
            // Real implementation would require per-account request tracking in the proxy
            let account_analytics: Vec<AccountAnalytics> = accounts.iter()
                .filter(|a| !a.disabled)
                .map(|a| {
                    let tier = a.quota.as_ref()
                        .and_then(|q| q.subscription_tier.clone())
                        .unwrap_or_else(|| "FREE".into())
                        .to_uppercase();

                    AccountAnalytics {
                        account_id: a.id.clone().into(),
                        email: a.email.clone().into(),
                        tier: tier.into(),
                        requests_today: 0, // Placeholder
                        requests_total: 0, // Placeholder
                        success_count: 0, // Placeholder
                        error_count: 0, // Placeholder
                        success_rate: 1.0, // Placeholder - assume healthy
                        tokens_used: 0, // Placeholder
                        rate_limit_hits: 0, // Placeholder
                        circuit_state: "closed".into(), // Default to closed
                        last_request_time: "N/A".into(),
                    }
                })
                .collect();

            drop(state); // Release lock before UI update

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak.upgrade() {
                    // Update summary with active account count
                    let mut summary = summary;
                    summary.active_accounts = active_count;
                    summary.circuit_breaker.closed_count = active_count; // Assume all healthy for now

                    app.global::<AppState>().set_analytics_summary(summary);

                    let model = std::rc::Rc::new(VecModel::from(account_analytics));
                    app.global::<AppState>().set_account_analytics(model.into());

                    let ctrl = AppController::new(&app);
                    ctrl.show_status("Analytics refreshed", "success");
                }
            });
        });
    }

    fn reset_circuit_breaker(&self, account_id: &str) {
        let id = account_id.to_string();
        let app_weak = self.app.clone();

        self.show_status(&format!("Resetting circuit breaker for {}...", account_id), "info");

        // For now, just show a success message since circuit breaker state
        // is managed internally by the proxy and would require additional
        // API exposure to reset from the UI
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app_weak.upgrade() {
                let ctrl = AppController::new(&app);
                ctrl.show_status(&format!("Circuit breaker reset for {}", id), "success");
                // Refresh analytics to show updated state
                ctrl.refresh_analytics();
            }
        });
    }

    fn export_analytics(&self, format: &str) -> bool {
        let format_str = format.to_string();
        let app_weak = self.app.clone();

        self.show_status(&format!("Preparing {} export...", format.to_uppercase()), "info");

        // Collect current analytics data from UI state
        if let Some(app) = self.app.upgrade() {
            let summary = app.global::<AppState>().get_analytics_summary();
            let account_analytics: Vec<AccountAnalytics> = app.global::<AppState>()
                .get_account_analytics()
                .iter()
                .collect();

            // Convert to export format
            let export_data = AnalyticsExportData {
                export_timestamp: time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
                    .format(&time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap())
                    .unwrap_or_else(|_| "unknown".to_string()),
                export_format: format_str.clone(),
                summary: ExportSummary {
                    total_requests_today: summary.total_requests_today,
                    total_requests_all_time: summary.total_requests_all_time,
                    overall_success_rate: summary.overall_success_rate,
                    total_tokens_used: summary.total_tokens_used,
                    active_accounts: summary.active_accounts,
                    rate_limited_accounts: summary.rate_limited_accounts,
                    circuit_breaker_closed: summary.circuit_breaker.closed_count,
                    circuit_breaker_open: summary.circuit_breaker.open_count,
                    circuit_breaker_half_open: summary.circuit_breaker.half_open_count,
                    circuit_breaker_total_trips: summary.circuit_breaker.total_trips,
                },
                accounts: account_analytics.iter().map(|a| ExportAccountAnalytics {
                    account_id: a.account_id.to_string(),
                    email: a.email.to_string(),
                    tier: a.tier.to_string(),
                    requests_today: a.requests_today,
                    requests_total: a.requests_total,
                    success_count: a.success_count,
                    error_count: a.error_count,
                    success_rate: a.success_rate,
                    tokens_used: a.tokens_used,
                    rate_limit_hits: a.rate_limit_hits,
                    circuit_state: a.circuit_state.to_string(),
                    last_request_time: a.last_request_time.to_string(),
                }).collect(),
            };

            // Spawn file dialog and export in async context
            let format_for_export = format_str.clone();
            tokio::spawn(async move {
                let (extension, filter_name) = match format_for_export.as_str() {
                    "csv" => ("csv", "CSV Files"),
                    "json" => ("json", "JSON Files"),
                    "txt" => ("txt", "Text Files"),
                    _ => ("txt", "Text Files"),
                };

                let now = time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
                let timestamp = now
                    .format(&time::format_description::parse("[year][month][day]_[hour][minute][second]").unwrap())
                    .unwrap_or_else(|_| "unknown".to_string());
                let default_filename = format!(
                    "antigravity_analytics_{}.{}",
                    timestamp,
                    extension
                );

                // Open file save dialog
                let file_handle = rfd::AsyncFileDialog::new()
                    .set_title("Export Analytics Report")
                    .set_file_name(&default_filename)
                    .add_filter(filter_name, &[extension])
                    .save_file()
                    .await;

                if let Some(file) = file_handle {
                    let path = file.path().to_path_buf();

                    // Generate content based on format
                    let content_result = match format_for_export.as_str() {
                        "csv" => generate_csv_export(&export_data),
                        "json" => generate_json_export(&export_data),
                        "txt" => Ok(generate_text_export(&export_data)),
                        _ => Ok(generate_text_export(&export_data)),
                    };

                    match content_result {
                        Ok(content) => {
                            match std::fs::write(&path, content) {
                                Ok(_) => {
                                    let path_str = path.to_string_lossy().to_string();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(app) = app_weak.upgrade() {
                                            let ctrl = AppController::new(&app);
                                            ctrl.show_status(
                                                &format!("Export saved: {}", path_str),
                                                "success"
                                            );
                                        }
                                    });
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(app) = app_weak.upgrade() {
                                            let ctrl = AppController::new(&app);
                                            ctrl.show_status(
                                                &format!("Export failed: {}", err_msg),
                                                "error"
                                            );
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = app_weak.upgrade() {
                                    let ctrl = AppController::new(&app);
                                    ctrl.show_status(
                                        &format!("Export generation failed: {}", err_msg),
                                        "error"
                                    );
                                }
                            });
                        }
                    }
                } else {
                    // User cancelled the dialog
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak.upgrade() {
                            let ctrl = AppController::new(&app);
                            ctrl.show_status("Export cancelled", "info");
                        }
                    });
                }
            });

            return true;
        }

        false
    }

    fn update_uptime(&self) {
        if let Some(app) = self.app.upgrade() {
            let mut stats = app.global::<AppState>().get_stats();
            stats.uptime_seconds = self.start_time.elapsed().as_secs() as i32;
            app.global::<AppState>().set_stats(stats);
        }
    }
}

fn format_relative_time(timestamp: i64) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let diff = now - timestamp;

    if diff < 0 { "just now".into() }
    else if diff < 60 { format!("{}s ago", diff) }
    else if diff < 3600 { format!("{}m ago", diff / 60) }
    else if diff < 86400 { format!("{}h ago", diff / 3600) }
    else { format!("{}d ago", diff / 86400) }
}

// ========================================
// EXPORT GENERATION FUNCTIONS
// ========================================

/// Generate CSV export with summary header and per-account rows
fn generate_csv_export(data: &AnalyticsExportData) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut wtr = csv::Writer::from_writer(vec![]);

    // Write metadata comment as first row
    wtr.write_record(&[
        "# Antigravity Analytics Export",
        &data.export_timestamp,
        &format!("Format: {}", data.export_format),
    ])?;

    // Write summary section header
    wtr.write_record(&["# Summary Statistics", "", ""])?;
    wtr.write_record(&["Metric", "Value", ""])?;
    wtr.write_record(&["Total Requests Today", &data.summary.total_requests_today.to_string(), ""])?;
    wtr.write_record(&["Total Requests All Time", &data.summary.total_requests_all_time.to_string(), ""])?;
    wtr.write_record(&["Overall Success Rate", &format!("{:.2}%", data.summary.overall_success_rate * 100.0), ""])?;
    wtr.write_record(&["Total Tokens Used", &data.summary.total_tokens_used.to_string(), ""])?;
    wtr.write_record(&["Active Accounts", &data.summary.active_accounts.to_string(), ""])?;
    wtr.write_record(&["Rate Limited Accounts", &data.summary.rate_limited_accounts.to_string(), ""])?;

    // Circuit breaker summary
    wtr.write_record(&["# Circuit Breaker Status", "", ""])?;
    wtr.write_record(&["Closed (Healthy)", &data.summary.circuit_breaker_closed.to_string(), ""])?;
    wtr.write_record(&["Open (Failing)", &data.summary.circuit_breaker_open.to_string(), ""])?;
    wtr.write_record(&["Half-Open (Testing)", &data.summary.circuit_breaker_half_open.to_string(), ""])?;
    wtr.write_record(&["Total Trips", &data.summary.circuit_breaker_total_trips.to_string(), ""])?;

    // Empty row separator
    wtr.write_record(&["", "", ""])?;

    // Per-account analytics header
    wtr.write_record(&[
        "Account ID",
        "Email",
        "Tier",
        "Requests Today",
        "Requests Total",
        "Success Count",
        "Error Count",
        "Success Rate",
        "Tokens Used",
        "Rate Limit Hits",
        "Circuit State",
        "Last Request",
    ])?;

    // Per-account rows
    for account in &data.accounts {
        wtr.write_record(&[
            &account.account_id,
            &account.email,
            &account.tier,
            &account.requests_today.to_string(),
            &account.requests_total.to_string(),
            &account.success_count.to_string(),
            &account.error_count.to_string(),
            &format!("{:.2}%", account.success_rate * 100.0),
            &account.tokens_used.to_string(),
            &account.rate_limit_hits.to_string(),
            &account.circuit_state,
            &account.last_request_time,
        ])?;
    }

    let bytes = wtr.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

/// Generate JSON export with full structured data
fn generate_json_export(data: &AnalyticsExportData) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(serde_json::to_string_pretty(data)?)
}

/// Generate human-readable plain text report
fn generate_text_export(data: &AnalyticsExportData) -> String {
    let mut output = String::new();

    // Header
    output.push_str("================================================================================\n");
    output.push_str("                    ANTIGRAVITY ANALYTICS REPORT\n");
    output.push_str("================================================================================\n\n");
    output.push_str(&format!("Export Timestamp: {}\n", data.export_timestamp));
    output.push_str(&format!("Export Format: {}\n\n", data.export_format));

    // Summary Statistics
    output.push_str("--------------------------------------------------------------------------------\n");
    output.push_str("                         SUMMARY STATISTICS\n");
    output.push_str("--------------------------------------------------------------------------------\n\n");
    output.push_str(&format!("Total Requests Today:     {}\n", data.summary.total_requests_today));
    output.push_str(&format!("Total Requests All Time:  {}\n", data.summary.total_requests_all_time));
    output.push_str(&format!("Overall Success Rate:     {:.2}%\n", data.summary.overall_success_rate * 100.0));
    output.push_str(&format!("Total Tokens Used:        {}\n", data.summary.total_tokens_used));
    output.push_str(&format!("Active Accounts:          {}\n", data.summary.active_accounts));
    output.push_str(&format!("Rate Limited Accounts:    {}\n\n", data.summary.rate_limited_accounts));

    // Circuit Breaker Status
    output.push_str("--------------------------------------------------------------------------------\n");
    output.push_str("                       CIRCUIT BREAKER STATUS\n");
    output.push_str("--------------------------------------------------------------------------------\n\n");
    output.push_str(&format!("Closed (Healthy):    {} accounts\n", data.summary.circuit_breaker_closed));
    output.push_str(&format!("Open (Failing):      {} accounts\n", data.summary.circuit_breaker_open));
    output.push_str(&format!("Half-Open (Testing): {} accounts\n", data.summary.circuit_breaker_half_open));
    output.push_str(&format!("Total Circuit Trips: {}\n\n", data.summary.circuit_breaker_total_trips));

    // Per-Account Analytics
    output.push_str("--------------------------------------------------------------------------------\n");
    output.push_str("                       PER-ACCOUNT ANALYTICS\n");
    output.push_str("--------------------------------------------------------------------------------\n\n");

    if data.accounts.is_empty() {
        output.push_str("No account data available.\n\n");
    } else {
        for (i, account) in data.accounts.iter().enumerate() {
            output.push_str(&format!("Account #{}: {}\n", i + 1, account.email));
            output.push_str(&format!("  ID:              {}\n", account.account_id));
            output.push_str(&format!("  Tier:            {}\n", account.tier));
            output.push_str(&format!("  Requests Today:  {}\n", account.requests_today));
            output.push_str(&format!("  Requests Total:  {}\n", account.requests_total));
            output.push_str(&format!("  Success Count:   {}\n", account.success_count));
            output.push_str(&format!("  Error Count:     {}\n", account.error_count));
            output.push_str(&format!("  Success Rate:    {:.2}%\n", account.success_rate * 100.0));
            output.push_str(&format!("  Tokens Used:     {}\n", account.tokens_used));
            output.push_str(&format!("  Rate Limit Hits: {}\n", account.rate_limit_hits));
            output.push_str(&format!("  Circuit State:   {}\n", account.circuit_state));
            output.push_str(&format!("  Last Request:    {}\n\n", account.last_request_time));
        }
    }

    output.push_str("================================================================================\n");
    output.push_str("                           END OF REPORT\n");
    output.push_str("================================================================================\n");

    output
}

fn create_tray_icon(app_weak: slint::Weak<MainWindow>) -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    // Initialize GTK for tray icon support (required on Linux)
    #[cfg(target_os = "linux")]
    {
        // Try to initialize GTK, but don't fail if we can't
        let _ = std::panic::catch_unwind(|| {
            gtk::init().ok();
        });
    }

    let menu = Menu::new();
    menu.append(&MenuItem::with_id("show", "Show Window", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))?;

    let icon_rgba: Vec<u8> = (0..16*16)
        .flat_map(|i| {
            let x = i % 16;
            let y = i / 16;
            vec![(139 + x * 3).min(255) as u8, (92_i32 - y * 2).max(0) as u8, (246_i32 - x - y).max(0) as u8, 255]
        })
        .collect();

    let icon = tray_icon::Icon::from_rgba(icon_rgba, 16, 16)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Antigravity Tools")
        .with_icon(icon)
        .build()?;

    let menu_channel = tray_icon::menu::MenuEvent::receiver();
    std::thread::spawn(move || {
        loop {
            if let Ok(event) = menu_channel.recv() {
                let app_weak = app_weak.clone();
                match event.id.0.as_str() {
                    "show" => { let _ = slint::invoke_from_event_loop(move || { if let Some(app) = app_weak.upgrade() { app.window().show().ok(); } }); }
                    "quit" => { let _ = slint::invoke_from_event_loop(move || { slint::quit_event_loop().ok(); }); break; }
                    _ => {}
                }
            }
        }
    });

    Ok(tray)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info,antigravity=debug").init();
    tracing::info!("Starting Antigravity Desktop v4.0.0 (Slint Native)");

    let app = MainWindow::new()?;
    let controller = Arc::new(AppController::new(&app));

    let _tray = create_tray_icon(app.as_weak()).ok();

    setup_callbacks(&app, controller.clone());
    controller.load_config();
    controller.load_accounts();

    app.global::<AppState>().set_proxy_running(false);
    app.global::<AppState>().set_dark_mode(true);

    let controller_timer = controller.clone();
    slint::Timer::default().start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || { controller_timer.update_uptime(); });

    app.run()?;
    Ok(())
}

fn setup_callbacks(app: &MainWindow, controller: Arc<AppController>) {
    app.global::<AppState>().on_refresh_accounts({ let c = controller.clone(); move || c.load_accounts() });
    app.global::<AppState>().on_refresh_quota({ let c = controller.clone(); move |id| c.refresh_quota(&id) });
    app.global::<AppState>().on_refresh_all_quotas({ let c = controller.clone(); move || c.refresh_all_quotas() });
    app.global::<AppState>().on_toggle_account({ let c = controller.clone(); move |id| c.toggle_account(&id) });
    app.global::<AppState>().on_delete_account({ let c = controller.clone(); move |id| c.delete_account(&id) });
    app.global::<AppState>().on_switch_account({ let c = controller.clone(); move |id| c.switch_account(&id) });
    app.global::<AppState>().on_toggle_proxy_status(move |_id, _enable| { tracing::info!("Toggle proxy status"); } );
    app.global::<AppState>().on_toggle_proxy({ let c = controller.clone(); move || c.toggle_proxy() });
    app.global::<AppState>().on_start_proxy({ let c = controller.clone(); move || c.toggle_proxy() });
    app.global::<AppState>().on_stop_proxy({ let c = controller.clone(); move || c.toggle_proxy() });
    app.global::<AppState>().on_save_proxy_config({ let c = controller.clone(); move || c.save_proxy_config() });
    app.global::<AppState>().on_save_app_config({ let c = controller.clone(); move || c.save_app_config() });
    app.global::<AppState>().on_start_oauth(move || tracing::info!("Start OAuth") );
    app.global::<AppState>().on_complete_oauth(move || tracing::info!("Complete OAuth") );
    app.global::<AppState>().on_cancel_oauth({ let w = app.as_weak(); move || { if let Some(a) = w.upgrade() { a.global::<AppState>().set_oauth_in_progress(false); a.global::<AppState>().set_oauth_url("".into()); } } });
    app.global::<AppState>().on_add_account_manual({ let c = controller.clone(); move |token| c.add_account_manual(&token) });
    app.global::<AppState>().on_import_from_db({ let c = controller.clone(); move || c.import_from_db() });
    app.global::<AppState>().on_generate_api_key({ let c = controller.clone(); move || c.generate_api_key() });
    app.global::<AppState>().on_toggle_dark_mode({ let c = controller.clone(); move || c.toggle_dark_mode() });
    app.global::<AppState>().on_clear_logs({ let c = controller.clone(); move || c.clear_logs() });
    app.global::<AppState>().on_toggle_monitor_recording({ let c = controller.clone(); move || c.toggle_monitor_recording() });
    app.global::<AppState>().on_open_data_folder({ let c = controller.clone(); move || c.open_data_folder() });
    app.global::<AppState>().on_copy_to_clipboard({ let c = controller.clone(); move |text| c.copy_to_clipboard(&text) });
    app.global::<AppState>().on_filter_accounts({ let c = controller.clone(); move |filter| c.filter_accounts(&filter) });

    // Analytics callbacks
    app.global::<AppState>().on_refresh_analytics({ let c = controller.clone(); move || c.refresh_analytics() });
    app.global::<AppState>().on_reset_circuit_breaker({ let c = controller.clone(); move |account_id| c.reset_circuit_breaker(&account_id) });
    app.global::<AppState>().on_export_analytics({ let c = controller.clone(); move |format| c.export_analytics(&format) });
}

// ========================================
// TESTABLE FILTER LOGIC (Pure Functions)
// ========================================

/// Testable account representation for property-based testing.
/// Mirrors the relevant fields from Slint's Account struct.
#[derive(Debug, Clone, PartialEq)]
pub struct TestAccount {
    pub id: String,
    pub email: String,
    pub tier: String,
    pub quota_percentage: i32,
    pub enabled: bool,
    pub is_forbidden: bool,
}

/// Pure filtering function that matches the logic in `apply_filter_internal`.
/// This is extracted for testability without Slint UI dependencies.
pub fn filter_accounts_pure(accounts: &[TestAccount], filter: &str) -> Vec<TestAccount> {
    accounts
        .iter()
        .filter(|a| match filter {
            "available" => a.enabled && !a.is_forbidden && a.quota_percentage > 10,
            "low_quota" => a.quota_percentage < 20,
            "pro" => a.tier.to_uppercase() == "PRO",
            "ultra" => a.tier.to_uppercase() == "ULTRA",
            "free" => {
                let tier = a.tier.to_uppercase();
                tier == "FREE" || tier.is_empty()
            }
            _ => true, // "all" or any unknown filter
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Strategy to generate random tier values
    fn tier_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("PRO".to_string()),
            Just("ULTRA".to_string()),
            Just("FREE".to_string()),
            Just("".to_string()),
            Just("pro".to_string()),   // lowercase variants
            Just("Ultra".to_string()), // mixed case
            Just("free".to_string()),
        ]
    }

    // Strategy to generate a random TestAccount
    fn account_strategy() -> impl Strategy<Value = TestAccount> {
        (
            "[a-z0-9]{8}",           // id
            "[a-z]+@[a-z]+\\.[a-z]+", // email pattern
            tier_strategy(),
            0..=100i32,              // quota_percentage
            any::<bool>(),           // enabled
            any::<bool>(),           // is_forbidden
        )
            .prop_map(|(id, email, tier, quota_percentage, enabled, is_forbidden)| {
                TestAccount {
                    id,
                    email,
                    tier,
                    quota_percentage,
                    enabled,
                    is_forbidden,
                }
            })
    }

    // Strategy to generate a vector of accounts
    fn accounts_strategy() -> impl Strategy<Value = Vec<TestAccount>> {
        prop::collection::vec(account_strategy(), 0..50)
    }

    proptest! {
        /// Property: Filtering by "pro" tier produces only accounts with tier "PRO" (case-insensitive)
        #[test]
        fn prop_filter_pro_returns_only_pro_accounts(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "pro");
            for account in &filtered {
                prop_assert_eq!(account.tier.to_uppercase(), "PRO");
            }
        }

        /// Property: Filtering by "ultra" tier produces only accounts with tier "ULTRA" (case-insensitive)
        #[test]
        fn prop_filter_ultra_returns_only_ultra_accounts(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "ultra");
            for account in &filtered {
                prop_assert_eq!(account.tier.to_uppercase(), "ULTRA");
            }
        }

        /// Property: Filtering by "free" tier produces only accounts with tier "FREE" or empty
        #[test]
        fn prop_filter_free_returns_only_free_accounts(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "free");
            for account in &filtered {
                let tier_upper = account.tier.to_uppercase();
                prop_assert!(tier_upper == "FREE" || tier_upper.is_empty(),
                    "Expected FREE or empty tier, got: {}", account.tier);
            }
        }

        /// Property: Filtering by "available" excludes disabled accounts
        #[test]
        fn prop_filter_available_excludes_disabled(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "available");
            for account in &filtered {
                prop_assert!(account.enabled, "Disabled account should not pass 'available' filter");
            }
        }

        /// Property: Filtering by "available" excludes forbidden accounts
        #[test]
        fn prop_filter_available_excludes_forbidden(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "available");
            for account in &filtered {
                prop_assert!(!account.is_forbidden, "Forbidden account should not pass 'available' filter");
            }
        }

        /// Property: Filtering by "available" excludes accounts with quota <= 10
        #[test]
        fn prop_filter_available_excludes_low_quota(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "available");
            for account in &filtered {
                prop_assert!(account.quota_percentage > 10,
                    "Account with quota {} should not pass 'available' filter (must be > 10)",
                    account.quota_percentage);
            }
        }

        /// Property: Filtering by "low_quota" produces only accounts with quota < 20
        #[test]
        fn prop_filter_low_quota_threshold(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "low_quota");
            for account in &filtered {
                prop_assert!(account.quota_percentage < 20,
                    "Account with quota {} should not pass 'low_quota' filter (must be < 20)",
                    account.quota_percentage);
            }
        }

        /// Property: Filter "all" returns the same count as input
        #[test]
        fn prop_filter_all_returns_same_count(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "all");
            prop_assert_eq!(filtered.len(), accounts.len(),
                "Filter 'all' should return same count as input");
        }

        /// Property: Filter "all" returns exactly the input accounts
        #[test]
        fn prop_filter_all_returns_same_accounts(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "all");
            prop_assert_eq!(filtered, accounts,
                "Filter 'all' should return identical accounts");
        }

        /// Property: Filtering is idempotent (applying same filter twice gives same result)
        #[test]
        fn prop_filter_idempotent(
            accounts in accounts_strategy(),
            filter in prop_oneof![
                Just("all"),
                Just("available"),
                Just("low_quota"),
                Just("pro"),
                Just("ultra"),
                Just("free"),
            ]
        ) {
            let first_pass = filter_accounts_pure(&accounts, filter);
            let second_pass = filter_accounts_pure(&first_pass, filter);
            prop_assert_eq!(first_pass, second_pass,
                "Applying filter '{}' twice should yield same result", filter);
        }

        /// Property: Unknown filter behaves like "all"
        #[test]
        fn prop_unknown_filter_returns_all(accounts in accounts_strategy()) {
            let unknown_filtered = filter_accounts_pure(&accounts, "unknown_filter_xyz");
            let all_filtered = filter_accounts_pure(&accounts, "all");
            prop_assert_eq!(unknown_filtered, all_filtered,
                "Unknown filter should behave like 'all'");
        }

        /// Property: Filtered result is always a subset of input
        #[test]
        fn prop_filtered_is_subset(
            accounts in accounts_strategy(),
            filter in prop_oneof![
                Just("all"),
                Just("available"),
                Just("low_quota"),
                Just("pro"),
                Just("ultra"),
                Just("free"),
            ]
        ) {
            let filtered = filter_accounts_pure(&accounts, filter);
            for account in &filtered {
                prop_assert!(accounts.contains(account),
                    "Filtered account must exist in original set");
            }
        }

        /// Property: Filtering preserves account data integrity
        #[test]
        fn prop_filter_preserves_account_data(accounts in accounts_strategy()) {
            let filtered = filter_accounts_pure(&accounts, "pro");
            for filtered_account in &filtered {
                let original = accounts.iter().find(|a| a.id == filtered_account.id);
                prop_assert!(original.is_some(), "Filtered account must have matching original");
                prop_assert_eq!(filtered_account, original.unwrap(),
                    "Filtered account data must match original");
            }
        }

        /// Property: Tier filters are mutually exclusive (a PRO account won't appear in ULTRA results)
        #[test]
        fn prop_tier_filters_mutually_exclusive(accounts in accounts_strategy()) {
            let pro = filter_accounts_pure(&accounts, "pro");
            let ultra = filter_accounts_pure(&accounts, "ultra");

            for pro_acc in &pro {
                prop_assert!(!ultra.iter().any(|u| u.id == pro_acc.id),
                    "PRO account should not appear in ULTRA filtered results");
            }
        }

        /// Property: Combined count of tier filters equals expected
        /// (accounts are partitioned by tier into PRO, ULTRA, FREE categories)
        #[test]
        fn prop_tier_filter_completeness(accounts in accounts_strategy()) {
            let pro_count = filter_accounts_pure(&accounts, "pro").len();
            let ultra_count = filter_accounts_pure(&accounts, "ultra").len();
            let free_count = filter_accounts_pure(&accounts, "free").len();

            // Every account should be in exactly one tier category
            let total_categorized = pro_count + ultra_count + free_count;
            prop_assert_eq!(total_categorized, accounts.len(),
                "All accounts should be categorized into exactly one tier");
        }
    }

    // Unit tests for edge cases
    #[test]
    fn test_empty_accounts_list() {
        let accounts: Vec<TestAccount> = vec![];
        assert_eq!(filter_accounts_pure(&accounts, "all").len(), 0);
        assert_eq!(filter_accounts_pure(&accounts, "pro").len(), 0);
        assert_eq!(filter_accounts_pure(&accounts, "available").len(), 0);
    }

    #[test]
    fn test_available_filter_boundary_quota() {
        let accounts = vec![
            TestAccount {
                id: "1".to_string(),
                email: "test1@example.com".to_string(),
                tier: "PRO".to_string(),
                quota_percentage: 10, // Exactly at threshold (should NOT pass)
                enabled: true,
                is_forbidden: false,
            },
            TestAccount {
                id: "2".to_string(),
                email: "test2@example.com".to_string(),
                tier: "PRO".to_string(),
                quota_percentage: 11, // Just above threshold (should pass)
                enabled: true,
                is_forbidden: false,
            },
        ];

        let filtered = filter_accounts_pure(&accounts, "available");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "2");
    }

    #[test]
    fn test_low_quota_filter_boundary() {
        let accounts = vec![
            TestAccount {
                id: "1".to_string(),
                email: "test1@example.com".to_string(),
                tier: "FREE".to_string(),
                quota_percentage: 19, // Just below threshold (should pass)
                enabled: true,
                is_forbidden: false,
            },
            TestAccount {
                id: "2".to_string(),
                email: "test2@example.com".to_string(),
                tier: "FREE".to_string(),
                quota_percentage: 20, // Exactly at threshold (should NOT pass)
                enabled: true,
                is_forbidden: false,
            },
        ];

        let filtered = filter_accounts_pure(&accounts, "low_quota");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "1");
    }

    #[test]
    fn test_tier_case_insensitivity() {
        let accounts = vec![
            TestAccount {
                id: "1".to_string(),
                email: "a@b.com".to_string(),
                tier: "pro".to_string(), // lowercase
                quota_percentage: 50,
                enabled: true,
                is_forbidden: false,
            },
            TestAccount {
                id: "2".to_string(),
                email: "b@b.com".to_string(),
                tier: "Pro".to_string(), // mixed case
                quota_percentage: 50,
                enabled: true,
                is_forbidden: false,
            },
            TestAccount {
                id: "3".to_string(),
                email: "c@b.com".to_string(),
                tier: "PRO".to_string(), // uppercase
                quota_percentage: 50,
                enabled: true,
                is_forbidden: false,
            },
        ];

        let filtered = filter_accounts_pure(&accounts, "pro");
        assert_eq!(filtered.len(), 3);
    }
}
