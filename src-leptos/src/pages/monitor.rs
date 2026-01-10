//! Monitor page - Real-time request logging

use leptos::prelude::*;
use crate::components::{Button, ButtonVariant};

/// Request log entry
#[derive(Clone, Debug, PartialEq)]
pub struct RequestLog {
    pub id: String,
    pub timestamp: i64,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub duration_ms: u32,
    pub model: Option<String>,
    pub mapped_model: Option<String>,
    pub account_email: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

#[component]
pub fn Monitor() -> impl IntoView {
    // Local state
    let logs = RwSignal::new(Vec::<RequestLog>::new());
    let filter = RwSignal::new(String::new());
    let logging_enabled = RwSignal::new(true);
    
    // Stats
    let stats = Memo::new(move |_| {
        let logs = logs.get();
        let total = logs.len();
        let success = logs.iter().filter(|l| l.status >= 200 && l.status < 400).count();
        let error = total - success;
        (total, success, error)
    });
    
    // Filtered logs
    let filtered_logs = Memo::new(move |_| {
        let query = filter.get().to_lowercase();
        let all_logs = logs.get();
        
        if query.is_empty() {
            all_logs
        } else {
            all_logs.into_iter()
                .filter(|l| {
                    l.url.to_lowercase().contains(&query) ||
                    l.method.to_lowercase().contains(&query) ||
                    l.model.as_ref().map(|m| m.to_lowercase().contains(&query)).unwrap_or(false) ||
                    l.status.to_string().contains(&query)
                })
                .collect()
        }
    });
    
    let on_clear = move || {
        logs.set(vec![]);
    };

    view! {
        <div class="page monitor">
            <header class="page-header">
                <div class="header-left">
                    <a href="/proxy" class="back-button">"← Back"</a>
                    <div>
                        <h1>"Request Monitor"</h1>
                        <p class="subtitle">"Real-time API request logging"</p>
                    </div>
                </div>
                
                <div class="header-stats">
                    <span class="stat stat--total">{move || stats.get().0}" REQS"</span>
                    <span class="stat stat--success">{move || stats.get().1}" OK"</span>
                    <span class="stat stat--error">{move || stats.get().2}" ERR"</span>
                </div>
            </header>
            
            // Controls
            <div class="monitor-controls">
                <button 
                    class=move || format!("recording-btn {}", if logging_enabled.get() { "recording" } else { "paused" })
                    on:click=move |_| logging_enabled.update(|v| *v = !*v)
                >
                    <span class="dot"></span>
                    {move || if logging_enabled.get() { "Recording" } else { "Paused" }}
                </button>
                
                <div class="search-box">
                    <input 
                        type="text"
                        placeholder="Filter by URL, model, status..."
                        prop:value=move || filter.get()
                        on:input=move |ev| filter.set(event_target_value(&ev))
                    />
                </div>
                
                <div class="quick-filters">
                    <button 
                        class=move || if filter.get().is_empty() { "active" } else { "" }
                        on:click=move |_| filter.set(String::new())
                    >"All"</button>
                    <button 
                        class=move || if filter.get() == "40" { "active" } else { "" }
                        on:click=move |_| filter.set("40".to_string())
                    >"Errors"</button>
                    <button 
                        class=move || if filter.get() == "gemini" { "active" } else { "" }
                        on:click=move |_| filter.set("gemini".to_string())
                    >"Gemini"</button>
                    <button 
                        class=move || if filter.get() == "claude" { "active" } else { "" }
                        on:click=move |_| filter.set("claude".to_string())
                    >"Claude"</button>
                </div>
                
                <Button 
                    text="🗑".to_string()
                    variant=ButtonVariant::Ghost
                    on_click=on_clear
                />
            </div>
            
            // Logs table
            <div class="logs-table-container">
                <table class="logs-table">
                    <thead>
                        <tr>
                            <th class="col-status">"Status"</th>
                            <th class="col-method">"Method"</th>
                            <th class="col-model">"Model"</th>
                            <th class="col-account">"Account"</th>
                            <th class="col-path">"Path"</th>
                            <th class="col-tokens">"Tokens"</th>
                            <th class="col-duration">"Duration"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || filtered_logs.get()
                            key=|log| log.id.clone()
                            children=|log| {
                                let status_class = if log.status >= 200 && log.status < 400 { "success" } else { "error" };
                                let model_display = log.model.clone().unwrap_or_else(|| "-".to_string());
                                let mapped = log.mapped_model.clone().filter(|m| Some(m) != log.model.as_ref());
                                let account = log.account_email.clone().unwrap_or_else(|| "-".to_string());
                                let tokens_in = log.input_tokens.map(|t| format!("I:{}", t)).unwrap_or_default();
                                let tokens_out = log.output_tokens.map(|t| format!("O:{}", t)).unwrap_or_default();
                                
                                view! {
                                    <tr class="log-row">
                                        <td class="col-status">
                                            <span class=format!("status-badge status-badge--{}", status_class)>
                                                {log.status}
                                            </span>
                                        </td>
                                        <td class="col-method">{log.method}</td>
                                        <td class="col-model">
                                            {model_display}
                                            {mapped.map(|m| format!(" → {}", m))}
                                        </td>
                                        <td class="col-account">{account}</td>
                                        <td class="col-path">{log.url}</td>
                                        <td class="col-tokens">{tokens_in}" "{tokens_out}</td>
                                        <td class="col-duration">{log.duration_ms}"ms"</td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                </table>
                
                <Show when=move || filtered_logs.get().is_empty()>
                    <div class="empty-state">
                        <span class="empty-icon">"📡"</span>
                        <p>"No requests yet"</p>
                        <p class="hint">"Requests will appear here when the proxy is running"</p>
                    </div>
                </Show>
            </div>
        </div>
    }
}
