//! Accounts page

use leptos::prelude::*;
use crate::app::AppState;
use crate::components::{AccountRow, Button, ButtonVariant};
use crate::tauri::commands;

#[component]
pub fn Accounts() -> impl IntoView {
    let state = expect_context::<AppState>();
    
    // Selection state
    let selected_ids = RwSignal::new(std::collections::HashSet::<String>::new());
    let search_query = RwSignal::new(String::new());
    
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
    
    // Actions
    let refresh_action = Action::new(move |_: &()| async move {
        if let Ok(accounts) = commands::list_accounts().await {
            state.accounts.set(accounts);
        }
    });
    
    let delete_selected_action = Action::new(move |_: &()| async move {
        let ids: Vec<String> = selected_ids.get().into_iter().collect();
        for id in &ids {
            let _ = commands::delete_account(id).await;
        }
        selected_ids.set(std::collections::HashSet::new());
        // Refresh
        if let Ok(accounts) = commands::list_accounts().await {
            state.accounts.set(accounts);
        }
    });

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
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| refresh_action.dispatch(()))
                        loading=refresh_action.pending().get()
                    >
                        "🔄 Refresh"
                    </Button>
                    <Button variant=ButtonVariant::Primary on_click=Callback::new(|_| {})>
                        "➕ Add Account"
                    </Button>
                </div>
            </header>
            
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
                
                <Show when=move || selected_count.get() > 0>
                    <div class="selection-actions">
                        <span class="selection-count">
                            {move || format!("{} selected", selected_count.get())}
                        </span>
                        <Button 
                            variant=ButtonVariant::Danger
                            on_click=Callback::new(move |_| delete_selected_action.dispatch(()))
                        >
                            "🗑 Delete"
                        </Button>
                        <Button 
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(|_| {})
                        >
                            "📤 Export"
                        </Button>
                    </div>
                </Show>
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
                                let is_current = Signal::derive(move || {
                                    state.current_account_id.get() == Some(account_id.clone())
                                });
                                let is_selected = RwSignal::new(false);
                                
                                // Sync selection state
                                Effect::new(move |_| {
                                    is_selected.set(selected_ids.get().contains(&account_id));
                                });
                                
                                let account_id_for_select = account.id.clone();
                                let account_id_for_switch = account.id.clone();
                                let account_id_for_delete = account.id.clone();
                                let account_id_for_proxy = account.id.clone();
                                
                                view! {
                                    <AccountRow 
                                        account=account
                                        is_current=is_current
                                        is_selected=is_selected
                                        on_select=Callback::new(move |_| {
                                            selected_ids.update(|set| {
                                                if set.contains(&account_id_for_select) {
                                                    set.remove(&account_id_for_select);
                                                } else {
                                                    set.insert(account_id_for_select.clone());
                                                }
                                            });
                                        })
                                        on_switch=Callback::new(move |_| {
                                            let id = account_id_for_switch.clone();
                                            spawn_local(async move {
                                                let _ = commands::set_current_account_id(&id).await;
                                                let state = expect_context::<AppState>();
                                                state.current_account_id.set(Some(id));
                                            });
                                        })
                                        on_delete=Callback::new(move |_| {
                                            let id = account_id_for_delete.clone();
                                            spawn_local(async move {
                                                let _ = commands::delete_account(&id).await;
                                                if let Ok(accounts) = commands::list_accounts().await {
                                                    let state = expect_context::<AppState>();
                                                    state.accounts.set(accounts);
                                                }
                                            });
                                        })
                                        on_toggle_proxy=Callback::new(move |_| {
                                            log::info!("Toggle proxy for {}", account_id_for_proxy);
                                        })
                                    />
                                }
                            }
                        />
                    </tbody>
                </table>
                
                <Show when=move || filtered_accounts.get().is_empty()>
                    <div class="empty-state">
                        <span class="empty-icon">"👥"</span>
                        <p>"No accounts found"</p>
                        <Button variant=ButtonVariant::Primary on_click=Callback::new(|_| {})>
                            "Add your first account"
                        </Button>
                    </div>
                </Show>
            </div>
        </div>
    }
}
