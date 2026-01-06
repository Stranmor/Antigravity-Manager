use std::sync::Arc;
use slint::Model;
use antigravity_tools_lib::modules::account;
use tray_icon::{TrayIconBuilder, menu::{Menu, MenuItem, PredefinedMenuItem}};

slint::include_modules!();

struct AppController {
    app: slint::Weak<MainWindow>,
}

impl AppController {
    fn new(app: &MainWindow) -> Self {
        Self {
            app: app.as_weak(),
        }
    }

    fn load_accounts(&self) {
        match account::list_accounts() {
            Ok(accounts) => {
                let accounts_model: Vec<Account> = accounts
                    .iter()
                    .map(|a| {
                        let tier = a.quota.as_ref()
                            .and_then(|q| q.subscription_tier.clone())
                            .unwrap_or_else(|| "unknown".into());
                        
                        let quota_pct = a.quota.as_ref()
                            .and_then(|q| q.models.first())
                            .map(|m| m.percentage)
                            .unwrap_or(0);
                        
                        Account {
                            email: a.email.clone().into(),
                            tier: tier.into(),
                            quota_used: (100 - quota_pct) as i32,
                            quota_total: 100,
                            enabled: !a.disabled,
                            last_refresh: a.quota.as_ref()
                                .map(|q| format_relative_time(q.last_updated))
                                .unwrap_or_else(|| "never".into())
                                .into(),
                        }
                    })
                    .collect();

                if let Some(app) = self.app.upgrade() {
                    let active_count = accounts.iter().filter(|a| !a.disabled).count() as i32;
                    let model = std::rc::Rc::new(slint::VecModel::from(accounts_model));
                    app.global::<AppState>().set_accounts(model.into());
                    
                    let mut stats = app.global::<AppState>().get_stats();
                    stats.active_accounts = active_count;
                    app.global::<AppState>().set_stats(stats);
                }
            }
            Err(e) => {
                tracing::error!("Failed to load accounts: {}", e);
            }
        }
    }

    fn toggle_account(&self, index: i32) {
        if let Some(app) = self.app.upgrade() {
            let accounts = app.global::<AppState>().get_accounts();
            if let Some(account_row) = accounts.row_data(index as usize) {
                let email = account_row.email.to_string();
                
                if let Ok(mut acc) = account::load_account(&email) {
                    acc.disabled = !acc.disabled;
                    if let Err(e) = account::save_account(&acc) {
                        tracing::error!("Failed to save account {}: {}", email, e);
                    }
                    self.load_accounts();
                }
            }
        }
    }

    fn delete_account(&self, index: i32) {
        if let Some(app) = self.app.upgrade() {
            let accounts = app.global::<AppState>().get_accounts();
            if let Some(account_row) = accounts.row_data(index as usize) {
                let email = account_row.email.to_string();
                tracing::info!("Deleting account: {}", email);
                
                if let Err(e) = account::delete_account(&email) {
                    tracing::error!("Failed to delete account: {}", e);
                }
                self.load_accounts();
            }
        }
    }
}

fn format_relative_time(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;
    
    if diff < 0 {
        "just now".into()
    } else if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn create_tray_icon() -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();
    
    let show_item = MenuItem::with_id("show", "Show Window", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit_item = MenuItem::with_id("quit", "Quit", true, None);
    
    menu.append(&show_item)?;
    menu.append(&separator)?;
    menu.append(&quit_item)?;
    
    let icon_rgba: Vec<u8> = vec![
        0x8B, 0x5C, 0xF6, 0xFF, 0x8B, 0x5C, 0xF6, 0xFF, 0x7C, 0x3A, 0xED, 0xFF, 0x7C, 0x3A, 0xED, 0xFF,
        0x6D, 0x28, 0xD9, 0xFF, 0x6D, 0x28, 0xD9, 0xFF, 0x5B, 0x21, 0xB6, 0xFF, 0x5B, 0x21, 0xB6, 0xFF,
        0x4C, 0x1D, 0x95, 0xFF, 0x4C, 0x1D, 0x95, 0xFF, 0x3B, 0x0A, 0x82, 0xFF, 0x3B, 0x0A, 0x82, 0xFF,
        0x1E, 0x3A, 0x8A, 0xFF, 0x1E, 0x3A, 0x8A, 0xFF, 0x1E, 0x40, 0xAF, 0xFF, 0x1E, 0x40, 0xAF, 0xFF,
    ].into_iter().cycle().take(16 * 16 * 4).collect();
    
    let icon = tray_icon::Icon::from_rgba(icon_rgba, 16, 16)?;
    
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Antigravity Tools")
        .with_icon(icon)
        .build()?;
    
    Ok(tray)
}

fn setup_tray_event_handler(app_weak: slint::Weak<MainWindow>) {
    use tray_icon::menu::MenuEvent;
    
    let menu_channel = MenuEvent::receiver();
    
    std::thread::spawn(move || {
        loop {
            if let Ok(event) = menu_channel.recv() {
                let app_weak = app_weak.clone();
                match event.id.0.as_str() {
                    "show" => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak.upgrade() {
                                app.window().show().ok();
                            }
                        });
                    }
                    "quit" => {
                        let _ = slint::invoke_from_event_loop(move || {
                            slint::quit_event_loop().ok();
                        });
                        break;
                    }
                    _ => {}
                }
            }
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info,antigravity=debug")
        .init();

    tracing::info!("Starting Antigravity Desktop v4.0.0");

    let _tray = create_tray_icon().map_err(|e| {
        tracing::warn!("Failed to create tray icon: {}. Continuing without tray.", e);
    }).ok();

    let app = MainWindow::new()?;
    let controller = Arc::new(AppController::new(&app));

    if _tray.is_some() {
        setup_tray_event_handler(app.as_weak());
        tracing::info!("System tray initialized");
    }

    setup_callbacks(&app, controller.clone());
    controller.load_accounts();

    app.global::<AppState>().set_stats(ProxyStats {
        total_requests: 0,
        success_rate: 0.0,
        active_accounts: 0,
        uptime_seconds: 0,
    });
    
    app.global::<AppState>().set_proxy_running(false);
    app.global::<AppState>().set_dark_mode(true);

    app.run()?;
    Ok(())
}

fn setup_callbacks(app: &MainWindow, controller: Arc<AppController>) {
    app.global::<AppState>().on_refresh_accounts({
        let controller = controller.clone();
        move || {
            tracing::info!("Refreshing accounts...");
            controller.load_accounts();
        }
    });

    app.global::<AppState>().on_toggle_proxy({
        let app_weak = app.as_weak();
        move || {
            if let Some(app) = app_weak.upgrade() {
                let current = app.global::<AppState>().get_proxy_running();
                app.global::<AppState>().set_proxy_running(!current);
                tracing::info!("Proxy toggled: {}", !current);
            }
        }
    });

    app.global::<AppState>().on_toggle_account({
        let controller = controller.clone();
        move |index| {
            controller.toggle_account(index);
        }
    });

    app.global::<AppState>().on_delete_account({
        let controller = controller.clone();
        move |index| {
            controller.delete_account(index);
        }
    });

    app.global::<AppState>().on_open_settings({
        move || {
            tracing::info!("Open settings");
        }
    });

    app.global::<AppState>().on_toggle_dark_mode({
        let app_weak = app.as_weak();
        move || {
            if let Some(app) = app_weak.upgrade() {
                let dark_mode = app.global::<AppState>().get_dark_mode();
                tracing::info!("Theme switched to: {}", if dark_mode { "dark" } else { "light" });
            }
        }
    });
}
