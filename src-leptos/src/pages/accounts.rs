//! Accounts page

use leptos::prelude::*;
use leptos::task::spawn_local;
use crate::app::AppState;
use crate::components::{Button, ButtonVariant};
use crate::tauri::commands;

#[component]
pub fn Accounts() -> impl IntoView {
    let state = expect_context::<AppState>();
    
    // Selection state
    let selected_ids = RwSignal::new(std::collections::HashSet::<String>::new());
    let search_query = RwSignal::new(String::new());
    
    // Loading states
    let refresh_pending = RwSignal::new(false);
    let oauth_pending = RwSignal::new(false);
    let sync_pending = RwSignal::new(false);
    
    // Error/success messages
    let message = RwSignal::new(Option::<(String, bool)>::None);
    
    // Filtered accounts
    let filtered_accounts = Memo::new(move |_| {
        let query = search_query.get().to_lowercase();
        let accounts = state.accounts.get();
        
        if query.is_empty() {
            accounts
        } else {
            accounts.into_iter()
                .filter(|a| a.email.to_lowercase().contains(&query))
                .collect()
        }
    });
    
    // Selection counts
    let selected_count = Memo::new(move |_| selected_ids.get().len());
    let all_selected = Memo::new(move |_| {
        let accounts = filtered_accounts.get();
        let selected = selected_ids.get();
        !accounts.is_empty() && accounts.iter().all(|a| selected.contains(&a.id))
    });
    
    // Show message helper
    let show_message = move |msg: String, is_error: bool| {
        message.set(Some((msg, is_error)));
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            message.set(None);
        });
    };
    
    // Refresh accounts list
    let on_refresh = move || {
        refresh_pending.set(true);
        spawn_local(async move {
            match commands::list_accounts().await {
                Ok(accounts) => {
                    let state = expect_context::<AppState>();
                    state.accounts.set(accounts);
                }
                Err(e) => show_message(format!("Failed to refresh: {}", e), true),
            }
            refresh_pending.set(false);
        });
    };
    
    // Refresh all quotas
    let on_refresh_quotas = move || {
        refresh_pending.set(true);
        spawn_local(async move {
            match commands::refresh_all_quotas().await {
                Ok(stats) => {
                    show_message(format!("Refreshed {}/{} accounts", stats.success, stats.total), false);
                    // Reload accounts to get updated quotas
                    if let Ok(accounts) = commands::list_accounts().await {
                        let state = expect_context::<AppState>();
                        state.accounts.set(accounts);
                    }
                }
                Err(e) => show_message(format!("Failed: {}", e), true),
            }
            refresh_pending.set(false);
        });
    };
    
    // Start OAuth login
    let on_add_account = move || {
        oauth_pending.set(true);
        spawn_local(async move {
            match commands::start_oauth_login().await {
                Ok(account) => {
                    show_message(format!("Added: {}", account.email), false);
                    // Reload accounts
                    if let Ok(accounts) = commands::list_accounts().await {
                        let state = expect_context::<AppState>();
                        state.accounts.set(accounts);
                    }
                }
                Err(e) => show_message(format!("OAuth failed: {}", e), true),
            }
            oauth_pending.set(false);
        });
    };
    
    // Sync from local Antigravity DB
    let on_sync_local = move || {
        sync_pending.set(true);
        spawn_local(async move {
            match commands::sync_account_from_db().await {
                Ok(Some(account)) => {
                    show_message(format!("Synced: {}", account.email), false);
                    if let Ok(accounts) = commands::list_accounts().await {
                        let state = expect_context::<AppState>();
                        state.accounts.set(accounts);
                    }
                }
                Ok(None) => show_message("No account found in local DB".to_string(), true),
                Err(e) => show_message(format!("Sync failed: {}", e), true),
            }
            sync_pending.set(false);
        });
    };
    
    // Delete selected accounts
    let on_delete_selected = move || {
        let ids: Vec<String> = selected_ids.get().into_iter().collect();
        let count = ids.len();
        spawn_local(async move {
            match commands::delete_accounts(&ids).await {
                Ok(()) => {
                    show_message(format!("Deleted {} accounts", count), false);
                    selected_ids.set(std::collections::HashSet::new());
                    if let Ok(accounts) = commands::list_accounts().await {
                        let state = expect_context::<AppState>();
                        state.accounts.set(accounts);
                    }
                }
                Err(e) => show_message(format!("Delete failed: {}", e), true),
            }
        });
    };
    
    // Switch to account
    let on_switch_account = move |account_id: String| {
        spawn_local(async move {
            if commands::switch_account(&account_id).await.is_ok() {
                let state = expect_context::<AppState>();
                state.current_account_id.set(Some(account_id));
            }
        });
    };
    
    // Delete single account
    let on_delete_account = move |account_id: String| {
        spawn_local(async move {
            if commands::delete_account(&account_id).await.is_ok() {
                if let Ok(accounts) = commands::list_accounts().await {
                    let state = expect_context::<AppState>();
                    state.accounts.set(accounts);
                }
            }
        });
    };

    view! {
        <div class="page accounts">
            <header class="page-header">
                <div class="header-left">
                    <h1>"Accounts"</h1>
                    <p class="subtitle">
                        {move || format!("{} accounts", state.accounts.get().len())}
                    </p>
                </div>
                <div class="header-actions">
                    <Button 
                        text="📥 Sync Local".to_string()
                        variant=ButtonVariant::Secondary
                        on_click=on_sync_local
                        loading=sync_pending.get()
                    />
                    <Button 
                        text="🔄 Refresh Quotas".to_string()
                        variant=ButtonVariant::Secondary
                        on_click=on_refresh_quotas
                        loading=refresh_pending.get()
                    />
                    <Button 
                        text="➕ Add Account".to_string()
                        variant=ButtonVariant::Primary 
                        on_click=on_add_account
                        loading=oauth_pending.get()
                    />
                </div>
            </header>
            
            // Message banner
            <Show when=move || message.get().is_some()>
                {move || {
                    let (msg, is_error) = message.get().unwrap();
                    view! {
                        <div class=format!("alert {}", if is_error { "alert--error" } else { "alert--success" })>
                            <span>{msg}</span>
                        </div>
                    }
                }}
            </Show>
            
            // Toolbar
            <div class="toolbar">
                <div class="search-box">
                    <input 
                        type="text"
                        placeholder="Search accounts..."
                        prop:value=move || search_query.get()
                        on:input=move |ev| {
                            search_query.set(event_target_value(&ev));
                        }
                    />
                </div>
                
                <Show when=move || { selected_count.get() > 0 }>
                    <div class="selection-actions">
                        <span class="selection-count">
                            {move || format!("{} selected", selected_count.get())}
                        </span>
                        <Button 
                            text="🗑 Delete".to_string()
                            variant=ButtonVariant::Danger
                            on_click=on_delete_selected
                        />
                    </div>
                </Show>
                
                <div class="toolbar-right">
                    <Button 
                        text="🔄".to_string()
                        variant=ButtonVariant::Ghost
                        on_click=on_refresh
                        loading=refresh_pending.get()
                    />
                </div>
            </div>
            
            // Accounts table
            <div class="accounts-table-container">
                <table class="accounts-table">
                    <thead>
                        <tr>
                            <th class="col-checkbox">
                                <input 
                                    type="checkbox"
                                    checked=move || all_selected.get()
                                    on:change=move |_| {
                                        let accounts = filtered_accounts.get();
                                        if all_selected.get() {
                                            selected_ids.set(std::collections::HashSet::new());
                                        } else {
                                            selected_ids.set(accounts.iter().map(|a| a.id.clone()).collect());
                                        }
                                    }
                                />
                            </th>
                            <th class="col-status"></th>
                            <th class="col-email">"Email"</th>
                            <th class="col-quota">"Gemini"</th>
                            <th class="col-quota">"Claude"</th>
                            <th class="col-proxy">"Proxy"</th>
                            <th class="col-actions">"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || filtered_accounts.get()
                            key=|account| account.id.clone()
                            children=move |account| {
                            let account_id = account.id.clone();
                                let account_id2 = account.id.clone();
                                let account_id3 = account.id.clone();
                                let account_id4 = account.id.clone();
                                let account_id5 = account.id.clone();
                                let account_id_checkbox = account.id.clone();
                                let account_id_switch = account.id.clone();
                                let account_id_delete = account.id.clone();
                                let email = account.email.clone();
                                let is_disabled = account.disabled;
                                let proxy_disabled = account.proxy_disabled;
                                
                                // Compute quota
                                let quota_gemini = account.quota.as_ref().map(|q| {
                                    q.models.iter()
                                        .find(|m| m.model.contains("gemini") || m.model.contains("flash"))
                                        .map(|m| if m.limit > 0 { (m.limit - m.used) * 100 / m.limit } else { 0 })
                                        .unwrap_or(0)
                                }).unwrap_or(0);
                                
                                let quota_claude = account.quota.as_ref().map(|q| {
                                    q.models.iter()
                                        .find(|m| m.model.contains("claude"))
                                        .map(|m| if m.limit > 0 { (m.limit - m.used) * 100 / m.limit } else { 0 })
                                        .unwrap_or(0)
                                }).unwrap_or(0);
                                
                                let is_current_class = move || {
                                    if state.current_account_id.get() == Some(account_id.clone()) {
                                        "is-current"
                                    } else {
                                        ""
                                    }
                                };
                                
                                let is_selected_class = move || {
                                    if selected_ids.get().contains(&account_id2) {
                                        "is-selected"
                                    } else {
                                        ""
                                    }
                                };
                                
                                let is_selected_checked = move || {
                                    selected_ids.get().contains(&account_id3)
                                };
                                
                                let status_class = move || {
                                    if state.current_account_id.get() == Some(account_id4.clone()) {
                                        "status-dot--active"
                                    } else if is_disabled {
                                        "status-dot--disabled"
                                    } else {
                                        "status-dot--idle"
                                    }
                                };
                                
                                let is_current = move || {
                                    state.current_account_id.get() == Some(account_id5.clone())
                                };
                                
                                view! {
                                    <tr class=move || format!("account-row {} {}", is_current_class(), is_selected_class())>
                                        <td class="col-checkbox">
                                            <input 
                                                type="checkbox" 
                                                checked=is_selected_checked
                                                on:change={
                                                    let id = account_id_checkbox.clone();
                                                    move |_| {
                                                        let id = id.clone();
                                                        selected_ids.update(move |set| {
                                                            if set.contains(&id) {
                                                                set.remove(&id);
                                                            } else {
                                                                set.insert(id);
                                                            }
                                                        });
                                                    }
                                                }
                                            />
                                        </td>
                                        <td class="col-status">
                                            <span class=move || format!("status-dot {}", status_class())></span>
                                        </td>
                                        <td class="col-email">
                                            <span class="email-text">{email.clone()}</span>
                                            <Show when=is_current>
                                                <span class="current-badge">"ACTIVE"</span>
                                            </Show>
                                        </td>
                                        <td class="col-quota">
                                            <div class="quota-bar">
                                                <div 
                                                    class=format!("quota-fill {}", quota_class(quota_gemini))
                                                    style=format!("width: {}%", quota_gemini)
                                                ></div>
                                            </div>
                                            <span class="quota-text">{quota_gemini}"%"</span>
                                        </td>
                                        <td class="col-quota">
                                            <div class="quota-bar">
                                                <div 
                                                    class=format!("quota-fill {}", quota_class(quota_claude))
                                                    style=format!("width: {}%", quota_claude)
                                                ></div>
                                            </div>
                                            <span class="quota-text">{quota_claude}"%"</span>
                                        </td>
                                        <td class="col-proxy">
                                            <span class=format!("proxy-badge {}", if proxy_disabled { "off" } else { "on" })>
                                                {if proxy_disabled { "OFF" } else { "ON" }}
                                            </span>
                                        </td>
                                        <td class="col-actions">
                                            <button 
                                                class="btn btn--icon" 
                                                title="Switch to this account"
                                                on:click={
                                                    let id = account_id_switch.clone();
                                                    move |_| on_switch_account(id.clone())
                                                }
                                            >"⚡"</button>
                                            <button 
                                                class="btn btn--icon btn--danger" 
                                                title="Delete"
                                                on:click={
                                                    let id = account_id_delete.clone();
                                                    move |_| on_delete_account(id.clone())
                                                }
                                            >"🗑"</button>
                                        </td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                </table>
                
                <Show when=move || filtered_accounts.get().is_empty()>
                    <div class="empty-state">
                        <span class="empty-icon">"👥"</span>
                        <p>"No accounts found"</p>
                        <p class="hint">"Add an account using OAuth or sync from local Antigravity"</p>
                        <div class="empty-actions">
                            <Button 
                                text="➕ Add Account".to_string()
                                variant=ButtonVariant::Primary
                                on_click=on_add_account
                                loading=oauth_pending.get()
                            />
                            <Button 
                                text="📥 Sync Local".to_string()
                                variant=ButtonVariant::Secondary
                                on_click=on_sync_local
                                loading=sync_pending.get()
                            />
                        </div>
                    </div>
                </Show>
            </div>
        </div>
    }
}

fn quota_class(percent: i32) -> &'static str {
    match percent {
        0..=20 => "quota-fill--critical",
        21..=50 => "quota-fill--warning",
        _ => "quota-fill--good",
    }
}
