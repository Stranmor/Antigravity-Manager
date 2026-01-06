use std::sync::Arc;
use slint::Model;
use antigravity_tools_lib::modules::account;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info,antigravity=debug")
        .init();

    tracing::info!("Starting Antigravity Desktop v4.0.0");

    let app = MainWindow::new()?;
    let controller = Arc::new(AppController::new(&app));

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
}
