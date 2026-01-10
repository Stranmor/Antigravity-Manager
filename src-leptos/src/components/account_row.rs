//! Account row component for accounts table

use leptos::prelude::*;
use crate::types::Account;

#[component]
pub fn AccountRow(
    account: Account,
    is_current: Signal<bool>,
    is_selected: RwSignal<bool>,
    #[prop(into)] on_select: Callback<()>,
    #[prop(into)] on_switch: Callback<()>,
    #[prop(into)] on_delete: Callback<()>,
    #[prop(into)] on_toggle_proxy: Callback<()>,
) -> impl IntoView {
    let email = account.email.clone();
    let name = account.name.clone().unwrap_or_default();
    let id = account.id.clone();
    
    // Compute quota percentages
    let quota_gemini = account.quota.as_ref().map(|q| {
        q.models.iter()
            .find(|m| m.model.contains("gemini-3"))
            .map(|m| if m.limit > 0 { (m.limit - m.used) * 100 / m.limit } else { 0 })
            .unwrap_or(0)
    }).unwrap_or(0);
    
    let quota_claude = account.quota.as_ref().map(|q| {
        q.models.iter()
            .find(|m| m.model.contains("claude"))
            .map(|m| if m.limit > 0 { (m.limit - m.used) * 100 / m.limit } else { 0 })
            .unwrap_or(0)
    }).unwrap_or(0);

    view! {
        <tr 
            class:account-row=true
            class:is-current=move || is_current.get()
            class:is-selected=move || is_selected.get()
        >
            // Checkbox
            <td class="col-checkbox">
                <input 
                    type="checkbox" 
                    checked=move || is_selected.get()
                    on:change=move |_| on_select.call(())
                />
            </td>
            
            // Status indicator
            <td class="col-status">
                <span class=move || format!("status-dot {}", 
                    if is_current.get() { "status-dot--active" } 
                    else if account.disabled { "status-dot--disabled" }
                    else { "status-dot--idle" }
                )></span>
            </td>
            
            // Email & Name
            <td class="col-email">
                <div class="email-cell">
                    <span class="email">{email}</span>
                    {(!name.is_empty()).then(|| view! { <span class="name">{name}</span> })}
                </div>
            </td>
            
            // Gemini Quota
            <td class="col-quota">
                <div class="quota-bar">
                    <div 
                        class=format!("quota-fill {}", quota_class(quota_gemini))
                        style=format!("width: {}%", quota_gemini)
                    ></div>
                </div>
                <span class="quota-text">{quota_gemini}"%"</span>
            </td>
            
            // Claude Quota
            <td class="col-quota">
                <div class="quota-bar">
                    <div 
                        class=format!("quota-fill {}", quota_class(quota_claude))
                        style=format!("width: {}%", quota_claude)
                    ></div>
                </div>
                <span class="quota-text">{quota_claude}"%"</span>
            </td>
            
            // Proxy toggle
            <td class="col-proxy">
                <button 
                    class=format!("proxy-toggle {}", if account.proxy_disabled { "disabled" } else { "enabled" })
                    on:click=move |_| on_toggle_proxy.call(())
                >
                    {if account.proxy_disabled { "OFF" } else { "ON" }}
                </button>
            </td>
            
            // Actions
            <td class="col-actions">
                <div class="action-buttons">
                    <button 
                        class="btn btn--icon" 
                        title="Switch to this account"
                        on:click=move |_| on_switch.call(())
                    >
                        "⚡"
                    </button>
                    <button 
                        class="btn btn--icon btn--danger" 
                        title="Delete account"
                        on:click=move |_| on_delete.call(())
                    >
                        "🗑"
                    </button>
                </div>
            </td>
        </tr>
    }
}

fn quota_class(percent: i32) -> &'static str {
    match percent {
        0..=20 => "quota-fill--critical",
        21..=50 => "quota-fill--warning",
        _ => "quota-fill--good",
    }
}
