//! API Proxy page

use leptos::prelude::*;
use crate::app::AppState;
use crate::components::{Button, ButtonVariant};
use crate::tauri::commands;

#[component]
pub fn ApiProxy() -> impl IntoView {
    let state = expect_context::<AppState>();
    
    // Local state
    let loading = RwSignal::new(false);
    let copied = RwSignal::new(false);
    
    // Toggle proxy
    let toggle_proxy = Action::new(move |_: &()| async move {
        loading.set(true);
        let status = state.proxy_status.get();
        
        let result = if status.running {
            commands::stop_proxy_service().await
        } else {
            if let Some(config) = state.config.get() {
                commands::start_proxy_service(&config.proxy).await
            } else {
                Err("No config loaded".to_string())
            }
        };
        
        if result.is_ok() {
            // Refresh status
            if let Ok(new_status) = commands::get_proxy_status().await {
                state.proxy_status.set(new_status);
            }
        }
        loading.set(false);
    });
    
    // Generate API key
    let generate_key = Action::new(move |_: &()| async move {
        if let Ok(new_key) = commands::generate_api_key().await {
            state.config.update(|c| {
                if let Some(config) = c.as_mut() {
                    config.proxy.api_key = new_key;
                }
            });
        }
    });

    view! {
        <div class="page proxy">
            <header class="page-header">
                <div class="header-left">
                    <h1>"API Proxy"</h1>
                    <p class="subtitle">"OpenAI-compatible API endpoint"</p>
                </div>
                <div class="header-actions">
                    <a href="/monitor" class="btn btn--secondary">
                        "📡 Monitor"
                    </a>
                </div>
            </header>
            
            // Status card
            <section class="status-card">
                <div class="status-indicator">
                    <span class=move || format!("status-dot status-dot--{}", 
                        if state.proxy_status.get().running { "running" } else { "stopped" }
                    )></span>
                    <div class="status-info">
                        <h3>{move || if state.proxy_status.get().running { "Running" } else { "Stopped" }}</h3>
                        <p>{move || {
                            let status = state.proxy_status.get();
                            if status.running {
                                format!("{} • {} accounts", status.base_url, status.active_accounts)
                            } else {
                                "Proxy is not running".to_string()
                            }
                        }}</p>
                    </div>
                </div>
                <Button 
                    variant=if state.proxy_status.get().running { ButtonVariant::Danger } else { ButtonVariant::Primary }
                    loading=loading.get()
                    on_click=Callback::new(move |_| toggle_proxy.dispatch(()))
                >
                    {move || if state.proxy_status.get().running { "⏹ Stop" } else { "▶ Start" }}
                </Button>
            </section>
            
            // Configuration
            <section class="config-section">
                <h2>"Configuration"</h2>
                
                <div class="config-grid">
                    <div class="config-item">
                        <label>"Port"</label>
                        <input 
                            type="number" 
                            value=move || state.config.get().map(|c| c.proxy.port).unwrap_or(8045)
                            disabled=move || state.proxy_status.get().running
                        />
                    </div>
                    
                    <div class="config-item">
                        <label>"Request Timeout (s)"</label>
                        <input 
                            type="number" 
                            value=move || state.config.get().map(|c| c.proxy.request_timeout).unwrap_or(120)
                        />
                    </div>
                    
                    <div class="config-item config-item--toggle">
                        <label>"Auto-start"</label>
                        <input 
                            type="checkbox" 
                            checked=move || state.config.get().map(|c| c.proxy.auto_start).unwrap_or(false)
                        />
                    </div>
                    
                    <div class="config-item config-item--toggle">
                        <label>"Enable Logging"</label>
                        <input 
                            type="checkbox" 
                            checked=move || state.config.get().map(|c| c.proxy.enable_logging).unwrap_or(true)
                        />
                    </div>
                </div>
            </section>
            
            // API Key
            <section class="api-key-section">
                <h2>"API Authentication"</h2>
                
                <div class="api-key-row">
                    <input 
                        type="password" 
                        class="api-key-input"
                        value=move || state.config.get().map(|c| c.proxy.api_key.clone()).unwrap_or_default()
                        readonly=true
                    />
                    <button 
                        class="btn btn--icon"
                        on:click=move |_| {
                            if let Some(config) = state.config.get() {
                                let _ = web_sys::window()
                                    .and_then(|w| w.navigator().clipboard())
                                    .map(|c| c.write_text(&config.proxy.api_key));
                                copied.set(true);
                                set_timeout(move || copied.set(false), std::time::Duration::from_secs(2));
                            }
                        }
                    >
                        {move || if copied.get() { "✓" } else { "📋" }}
                    </button>
                    <Button 
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| generate_key.dispatch(()))
                    >
                        "🔄 Generate"
                    </Button>
                </div>
            </section>
            
            // Quick Start
            <section class="quick-start-section">
                <h2>"Quick Start"</h2>
                
                <div class="endpoint-card">
                    <label>"Base URL"</label>
                    <div class="endpoint-row">
                        <code>{move || format!("http://127.0.0.1:{}/v1", 
                            state.config.get().map(|c| c.proxy.port).unwrap_or(8045)
                        )}</code>
                        <button class="btn btn--icon">"📋"</button>
                    </div>
                </div>
                
                <div class="code-example">
                    <div class="code-header">
                        <span>"Python"</span>
                    </div>
                    <pre><code>{move || format!(r#"from openai import OpenAI

client = OpenAI(
    base_url="http://127.0.0.1:{}/v1",
    api_key="{}"
)

response = client.chat.completions.create(
    model="gemini-3-flash",
    messages=[{{"role": "user", "content": "Hello!"}}]
)

print(response.choices[0].message.content)"#, 
                        state.config.get().map(|c| c.proxy.port).unwrap_or(8045),
                        state.config.get().map(|c| c.proxy.api_key.clone()).unwrap_or_else(|| "YOUR_API_KEY".to_string())
                    )}</code></pre>
                </div>
            </section>
        </div>
    }
}
