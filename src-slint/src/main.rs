//! Antigravity Manager - Native Desktop UI
//!
//! This is the Slint-based native desktop application.

mod backend;

use antigravity_core::modules::{self, logger};
use backend::BackendState;
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;

slint::include_modules!();

/// Helper to build AccountData from antigravity_core::Account
fn build_account_data(account: &antigravity_core::models::Account, current_id: Option<&str>) -> AccountData {
    let is_current = current_id.map(|id| account.id == id).unwrap_or(false);
    let tier = BackendState::get_tier(account);
    let is_forbidden = account.quota.as_ref().map(|q| q.is_forbidden).unwrap_or(false);
    
    AccountData {
        id: account.id.clone().into(),
        email: account.email.clone().into(),
        name: account.name.clone().unwrap_or_default().into(),
        disabled: account.disabled,
        disabled_reason: account.disabled_reason.clone().unwrap_or_default().into(),
        proxy_disabled: account.proxy_disabled,
        subscription_tier: tier.into(),
        last_used: chrono::DateTime::from_timestamp(account.last_used, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "Unknown".to_string())
            .into(),
        gemini_pro_quota: BackendState::get_model_quota(account, "gemini-3-pro") as i32,
        gemini_flash_quota: BackendState::get_model_quota(account, "flash") as i32,
        gemini_image_quota: BackendState::get_model_quota(account, "image") as i32,
        claude_quota: BackendState::get_model_quota(account, "claude") as i32,
        is_current,
        is_forbidden,
        selected: false,
    }
}

/// Reload accounts from backend and update UI model
fn reload_accounts(
    model: &Rc<VecModel<AccountData>>,
    app_weak: &slint::Weak<AppWindow>,
) {
    if let Ok(accounts) = modules::list_accounts() {
        let current_id = modules::get_current_account_id().ok().flatten();
        
        // Clear and rebuild
        while model.row_count() > 0 {
            model.remove(0);
        }
        
        for account in &accounts {
            model.push(build_account_data(account, current_id.as_deref()));
        }
        
        // Update counts
        if let Some(app) = app_weak.upgrade() {
            let total = accounts.len() as i32;
            let pro_count = accounts.iter().filter(|a| {
                BackendState::get_tier(a) == "PRO"
            }).count() as i32;
            let ultra_count = accounts.iter().filter(|a| {
                BackendState::get_tier(a) == "ULTRA"
            }).count() as i32;
            let free_count = total - pro_count - ultra_count;
            
            app.global::<AppState>().set_all_count(total);
            app.global::<AppState>().set_pro_count(pro_count);
            app.global::<AppState>().set_ultra_count(ultra_count);
            app.global::<AppState>().set_free_count(free_count);
            app.global::<AppState>().set_total_items(total);
            app.global::<AppState>().set_selected_count(0);
        }
        
        tracing::info!("Reloaded {} accounts", accounts.len());
    }
}

/// Get selected account IDs from model
fn get_selected_ids(model: &Rc<VecModel<AccountData>>) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..model.row_count() {
        if let Some(account) = model.row_data(i) {
            if account.selected {
                ids.push(account.id.to_string());
            }
        }
    }
    ids
}

fn main() {
    // Initialize logging
    logger::init_logger();
    tracing::info!("Antigravity Manager starting...");

    // Create tokio runtime for async operations
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let _guard = runtime.enter();

    // Create backend
    let backend = backend::create_backend();

    // Load accounts
    let (
        total_accounts,
        avg_gemini,
        avg_gemini_image,
        avg_claude,
        low_quota_count,
        current_email,
        current_name,
        current_last_used,
        all_count,
        pro_count,
        ultra_count,
        free_count,
        accounts_data,
    ) = {
        let mut state = runtime.block_on(backend.lock());
        if let Err(e) = state.load_accounts() {
            tracing::warn!("Failed to load accounts: {}", e);
            (0, 0, 0, 0, 0, String::new(), String::new(), String::new(), 0, 0, 0, 0, Vec::new())
        } else {
            let current = state.get_current_account();
            let current_id = state.current_account_id().map(|s| s.to_string());
            let email = current.map(|a| a.email.clone()).unwrap_or_default();
            let name = current.and_then(|a| a.name.clone()).unwrap_or_default();
            let last_used = current.map(|a| {
                chrono::DateTime::from_timestamp(a.last_used, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Unknown".to_string())
            }).unwrap_or_default();
            
            // Build accounts data for UI
            let accounts_data: Vec<AccountData> = state.accounts().iter().enumerate().map(|(_i, a)| {
                build_account_data(a, current_id.as_deref())
            }).collect();
            
            (
                state.account_count() as i32,
                state.avg_gemini_quota() as i32,
                state.avg_gemini_image_quota() as i32,
                state.avg_claude_quota() as i32,
                state.low_quota_count() as i32,
                email,
                name,
                last_used,
                state.account_count() as i32,
                state.pro_count() as i32,
                state.ultra_count() as i32,
                state.free_count() as i32,
                accounts_data,
            )
        }
    };

    tracing::info!(
        "Stats: {} accounts, {}% avg Gemini, {}% avg Claude, {} low quota",
        total_accounts, avg_gemini, avg_claude, low_quota_count
    );

    // Create and run the main window
    let app = match AppWindow::new() {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Failed to create application window: {}", e);
            std::process::exit(1);
        }
    };

    // Set dashboard stats
    app.set_stats(DashboardStats {
        total_accounts,
        avg_gemini,
        avg_gemini_image,
        avg_claude,
        low_quota_count,
    });

    // Set current account info
    let has_account = !current_email.is_empty();
    app.set_current_account(CurrentAccountInfo {
        email: current_email.into(),
        name: if current_name.is_empty() { 
            "U".into() 
        } else { 
            current_name.chars().next().unwrap_or('U').to_string().into() 
        },
        last_used: current_last_used.into(),
        has_account,
    });

    // Create accounts model (Rc for sharing with callbacks)
    let accounts_vec_model = Rc::new(VecModel::from(accounts_data));
    let accounts_model = ModelRc::from(accounts_vec_model.clone());
    
    // Set AppState global data for AccountsPage
    app.global::<AppState>().set_accounts(accounts_model);
    app.global::<AppState>().set_all_count(all_count);
    app.global::<AppState>().set_pro_count(pro_count);
    app.global::<AppState>().set_ultra_count(ultra_count);
    app.global::<AppState>().set_free_count(free_count);
    app.global::<AppState>().set_total_items(total_accounts);
    app.global::<AppState>().set_total_pages(((total_accounts as f32) / 20.0).ceil().max(1.0) as i32);
    app.global::<AppState>().set_current_page(1);

    // === AppState Callbacks ===
    
    app.global::<AppState>().on_add_account(|| {
        tracing::info!("Add account requested - TODO: Open OAuth dialog");
    });

    // Refresh all accounts
    let model_for_refresh = accounts_vec_model.clone();
    let app_weak_for_refresh = app.as_weak();
    app.global::<AppState>().on_refresh_all(move || {
        tracing::info!("Refresh all accounts");
        reload_accounts(&model_for_refresh, &app_weak_for_refresh);
    });

    // Export selected accounts
    let model_for_export = accounts_vec_model.clone();
    app.global::<AppState>().on_export_selected(move || {
        let selected_ids = get_selected_ids(&model_for_export);
        if selected_ids.is_empty() {
            tracing::warn!("No accounts selected for export");
            return;
        }
        
        // Load full accounts data
        let mut accounts_to_export = Vec::new();
        for id in &selected_ids {
            if let Ok(account) = modules::load_account(id) {
                accounts_to_export.push(account);
            }
        }
        
        // Serialize to JSON
        let json = serde_json::to_string_pretty(&accounts_to_export).unwrap_or_default();
        tracing::info!("Exported {} accounts ({} bytes)", accounts_to_export.len(), json.len());
        
        // TODO: Save to file using native dialog
        // For now, just log
    });

    // Delete selected accounts
    let model_for_delete = accounts_vec_model.clone();
    let app_weak_for_delete = app.as_weak();
    app.global::<AppState>().on_delete_selected(move || {
        let selected_ids = get_selected_ids(&model_for_delete);
        if selected_ids.is_empty() {
            return;
        }
        
        tracing::info!("Deleting {} accounts", selected_ids.len());
        
        if let Err(e) = modules::delete_accounts(&selected_ids) {
            tracing::error!("Failed to delete accounts: {}", e);
        } else {
            reload_accounts(&model_for_delete, &app_weak_for_delete);
        }
    });

    app.global::<AppState>().on_toggle_proxy_batch(|enable| {
        tracing::info!("Toggle proxy batch: {}", enable);
        // TODO: Implement batch proxy toggle
    });

    app.global::<AppState>().on_search_changed(|query| {
        tracing::info!("Search: {}", query);
        // TODO: Implement search filtering
    });

    // Toggle single account selection
    let model_for_toggle = accounts_vec_model.clone();
    let app_weak_for_toggle = app.as_weak();
    app.global::<AppState>().on_toggle_select(move |id| {
        let id_str = id.to_string();
        let mut selected_count = 0;
        
        for i in 0..model_for_toggle.row_count() {
            if let Some(mut account) = model_for_toggle.row_data(i) {
                if account.id.as_str() == id_str {
                    account.selected = !account.selected;
                    model_for_toggle.set_row_data(i, account.clone());
                }
                if account.selected {
                    selected_count += 1;
                }
            }
        }
        
        if let Some(app) = app_weak_for_toggle.upgrade() {
            app.global::<AppState>().set_selected_count(selected_count);
        }
    });

    // Toggle all accounts selection
    let model_for_toggle_all = accounts_vec_model.clone();
    let app_weak_for_toggle_all = app.as_weak();
    app.global::<AppState>().on_toggle_all(move || {
        let mut all_selected = true;
        for i in 0..model_for_toggle_all.row_count() {
            if let Some(account) = model_for_toggle_all.row_data(i) {
                if !account.selected {
                    all_selected = false;
                    break;
                }
            }
        }
        
        let new_state = !all_selected;
        for i in 0..model_for_toggle_all.row_count() {
            if let Some(mut account) = model_for_toggle_all.row_data(i) {
                account.selected = new_state;
                model_for_toggle_all.set_row_data(i, account);
            }
        }
        
        if let Some(app) = app_weak_for_toggle_all.upgrade() {
            let count = if new_state { model_for_toggle_all.row_count() as i32 } else { 0 };
            app.global::<AppState>().set_selected_count(count);
        }
    });

    // Switch to account
    let model_for_switch = accounts_vec_model.clone();
    let app_weak_for_switch = app.as_weak();
    app.global::<AppState>().on_switch_account(move |id| {
        let id_str = id.to_string();
        tracing::info!("Switching to account: {}", id_str);
        
        if let Err(e) = modules::set_current_account_id(&id_str) {
            tracing::error!("Failed to switch account: {}", e);
        } else {
            // Update is_current flags in model
            let _current_id = Some(id_str.as_str());
            for i in 0..model_for_switch.row_count() {
                if let Some(mut account) = model_for_switch.row_data(i) {
                    account.is_current = account.id.as_str() == id_str;
                    model_for_switch.set_row_data(i, account);
                }
            }
            
            // Update current account info
            if let Some(app) = app_weak_for_switch.upgrade() {
                if let Ok(Some(account)) = modules::get_current_account() {
                    app.set_current_account(CurrentAccountInfo {
                        email: account.email.clone().into(),
                        name: account.name.as_ref()
                            .and_then(|n| n.chars().next())
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "U".to_string())
                            .into(),
                        last_used: chrono::DateTime::from_timestamp(account.last_used, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "Unknown".to_string())
                            .into(),
                        has_account: true,
                    });
                }
            }
        }
    });

    app.global::<AppState>().on_refresh_account(|id| {
        tracing::info!("Refresh account: {} - TODO: Implement OAuth quota refresh", id);
        // TODO: Implement quota refresh via API
    });

    app.global::<AppState>().on_view_details(|id| {
        tracing::info!("View details: {} - TODO: Open details dialog", id);
        // TODO: Implement details dialog
    });

    // Export single account
    app.global::<AppState>().on_export_account(|id| {
        let id_str = id.to_string();
        if let Ok(account) = modules::load_account(&id_str) {
            let json = serde_json::to_string_pretty(&account).unwrap_or_default();
            tracing::info!("Exported account {} ({} bytes)", id_str, json.len());
            // TODO: Save to file
        }
    });

    // Delete single account
    let model_for_delete_one = accounts_vec_model.clone();
    let app_weak_for_delete_one = app.as_weak();
    app.global::<AppState>().on_delete_account(move |id| {
        let id_str = id.to_string();
        tracing::info!("Deleting account: {}", id_str);
        
        if let Err(e) = modules::delete_account(&id_str) {
            tracing::error!("Failed to delete account: {}", e);
        } else {
            reload_accounts(&model_for_delete_one, &app_weak_for_delete_one);
        }
    });

    // Toggle proxy for single account
    let model_for_proxy = accounts_vec_model.clone();
    app.global::<AppState>().on_toggle_proxy(move |id| {
        let id_str = id.to_string();
        tracing::info!("Toggle proxy for: {}", id_str);
        
        // Find and toggle proxy_disabled
        for i in 0..model_for_proxy.row_count() {
            if let Some(mut account) = model_for_proxy.row_data(i) {
                if account.id.as_str() == id_str {
                    account.proxy_disabled = !account.proxy_disabled;
                    
                    // Save to disk
                    if let Ok(mut core_account) = modules::load_account(&id_str) {
                        core_account.proxy_disabled = account.proxy_disabled;
                        let _ = modules::save_account(&core_account);
                    }
                    
                    model_for_proxy.set_row_data(i, account);
                    break;
                }
            }
        }
    });

    // === Dashboard Callbacks ===
    
    let _backend_clone = backend.clone();
    let model_for_dash_refresh = accounts_vec_model.clone();
    let app_weak_for_dash_refresh = app.as_weak();
    let _runtime_handle = runtime.handle().clone();
    app.on_refresh_accounts(move || {
        tracing::info!("Dashboard: Refresh accounts");
        reload_accounts(&model_for_dash_refresh, &app_weak_for_dash_refresh);
    });

    app.on_add_account(|| {
        tracing::info!("Dashboard: Add account - TODO: Open OAuth dialog");
    });

    app.on_switch_account(|| {
        tracing::info!("Dashboard: Switch account - TODO: Open account picker");
    });

    tracing::info!("Application window created, running event loop...");

    if let Err(e) = app.run() {
        tracing::error!("Application error: {}", e);
        std::process::exit(1);
    }
}
