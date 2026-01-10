//! Monitor page - Real-time request logging

use leptos::prelude::*;
use crate::components::{Button, ButtonVariant};

/// Request log entry
#[derive(Clone, Debug)]
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
                        class:active=move || filter.get().is_empty()
                        on:click=move |_| filter.set(String::new())
                    >"All"</button>
                    <button 
                        class:active=move || filter.get() == "40"
                        on:click=move |_| filter.set("40".to_string())
                    >"Errors"</button>
                    <button 
                        class:active=move || filter.get() == "gemini"
                        on:click=move |_| filter.set("gemini".to_string())
                    >"Gemini"</button>
                    <button 
                        class:active=move || filter.get() == "claude"
                        on:click=move |_| filter.set("claude".to_string())
                    >"Claude"</button>
                </div>
                
                <Button 
                    variant=ButtonVariant::Ghost
                    on_click=Callback::new(move |_| logs.set(vec![]))
                >
                    "🗑"
                </Button>
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
                                
                                view! {
                                    <tr class="log-row">
                                        <td class="col-status">
                                            <span class=format!("status-badge status-badge--{}", status_class)>
                                                {log.status}
                                            </span>
                                        </td>
                                        <td class="col-method">{log.method}</td>
                                        <td class="col-model">
                                            {log.model.clone().unwrap_or_else(|| "-".to_string())}
                                            {log.mapped_model.as_ref().filter(|m| Some(*m) != log.model.as_ref()).map(|m| {
                                                view! { <span class="mapped">" → "{m}</span> }
                                            })}
                                        </td>
                                        <td class="col-account">{log.account_email.clone().unwrap_or_else(|| "-".to_string())}</td>
                                        <td class="col-path">{log.url}</td>
                                        <td class="col-tokens">
                                            {log.input_tokens.map(|t| format!("I:{}", t))}
                                            " "
                                            {log.output_tokens.map(|t| format!("O:{}", t))}
                                        </td>
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
