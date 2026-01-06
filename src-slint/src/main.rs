use std::sync::Arc;
use slint::VecModel;
use antigravity_tools_lib::modules::{account, config, oauth};
use antigravity_tools_lib::proxy::{AxumServer, TokenManager, ProxySecurityConfig};
use antigravity_tools_lib::proxy::monitor::ProxyMonitor;
use tray_icon::{TrayIconBuilder, menu::{Menu, MenuItem, PredefinedMenuItem}};
use tokio::sync::RwLock;

slint::include_modules!();

struct ProxyState {
    server: Option<AxumServer>,
    handle: Option<tokio::task::JoinHandle<()>>,
    token_manager: Option<Arc<TokenManager>>,
    monitor: Option<Arc<ProxyMonitor>>,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            server: None,
            handle: None,
            token_manager: None,
            monitor: None,
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

                if let Some(app) = self.app.upgrade() {
                    let active_count = accounts.iter()
                        .filter(|a| !a.disabled && a.quota.as_ref().map(|q| !q.is_forbidden).unwrap_or(true))
                        .count() as i32;
                    
                    let model = std::rc::Rc::new(VecModel::from(accounts_model));
                    app.global::<AppState>().set_accounts(model.into());
                    
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

    fn lib_account_to_slint(&self, a: &antigravity_tools_lib::models::Account, current_id: Option<&str>) -> Account {
        let tier = a.quota.as_ref()
            .and_then(|q| q.subscription_tier.clone())
            .unwrap_or_else(|| "FREE".into())
            .to_uppercase();
        
        let quota_pct = a.quota.as_ref()
            .and_then(|q| q.models.first())
            .map(|m| m.percentage)
            .unwrap_or(0) as i32;
        
        let is_forbidden = a.quota.as_ref().map(|q| q.is_forbidden).unwrap_or(false);
        
        let model_quotas: Vec<ModelQuota> = a.quota.as_ref()
            .map(|q| q.models.iter().map(|m| ModelQuota {
                model_name: m.name.clone().into(),
                percentage: m.percentage as i32,
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
                    }
                });
            } else {
                let cfg = match config::load_app_config() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to load config: {}", e);
                        return;
                    }
                };

                let data_dir = match account::get_data_dir() {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Failed to get data dir: {}", e);
                        return;
                    }
                };

                let token_manager = Arc::new(TokenManager::new(data_dir));
                token_manager.update_sticky_config(cfg.proxy.scheduling.clone()).await;

                if let Err(e) = token_manager.load_accounts().await {
                    tracing::warn!("Failed to load accounts: {}", e);
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
                        state.monitor = Some(monitor);

                        let port = cfg.proxy.port;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                app.global::<AppState>().set_proxy_running(true);
                                app.global::<AppState>().set_listen_port(port as i32);
                            }
                        });
                        tracing::info!("Proxy started on {}:{}", bind_addr, port);
                    }
                    Err(e) => {
                        tracing::error!("Failed to start proxy: {}", e);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                app.global::<AppState>().set_proxy_running(false);
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

    fn update_uptime(&self) {
        if let Some(app) = self.app.upgrade() {
            let mut stats = app.global::<AppState>().get_stats();
            stats.uptime_seconds = self.start_time.elapsed().as_secs() as i32;
            app.global::<AppState>().set_stats(stats);
        }
    }
}

fn format_relative_time(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;
    
    if diff < 0 { "just now".into() }
    else if diff < 60 { format!("{}s ago", diff) }
    else if diff < 3600 { format!("{}m ago", diff / 60) }
    else if diff < 86400 { format!("{}h ago", diff / 3600) }
    else { format!("{}d ago", diff / 86400) }
}

fn create_tray_icon(app_weak: slint::Weak<MainWindow>) -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id("show", "Show Window", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))?;
    
    let icon_rgba: Vec<u8> = (0..16*16)
        .flat_map(|i| {
            let x = i % 16;
            let y = i / 16;
            vec![(139 + x * 3).min(255) as u8, (92_i32 - y as i32 * 2).max(0) as u8, (246_i32 - x as i32 - y as i32).max(0) as u8, 255]
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
}
