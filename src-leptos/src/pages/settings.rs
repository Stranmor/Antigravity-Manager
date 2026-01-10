//! Settings page

use leptos::prelude::*;
use leptos::task::spawn_local;
use crate::app::AppState;
use crate::components::{Button, ButtonVariant};
use crate::tauri::commands;

#[component]
pub fn Settings() -> impl IntoView {
    let state = expect_context::<AppState>();
    
    // Saving state
    let saving = RwSignal::new(false);
    
    // Save settings
    let on_save = move || {
        saving.set(true);
        spawn_local(async move {
            let state = expect_context::<AppState>();
            if let Some(config) = state.config.get() {
                let _ = commands::save_config(&config).await;
            }
            saving.set(false);
        });
    };

    view! {
        <div class="page settings">
            <header class="page-header">
                <h1>"Settings"</h1>
                <Button 
                    text="💾 Save".to_string()
                    variant=ButtonVariant::Primary
                    loading=saving.get()
                    on_click=on_save
                />
            </header>
            
            // General
            <section class="settings-section">
                <h2>"General"</h2>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Language"</label>
                        <p class="setting-desc">"Interface language"</p>
                    </div>
                    <select>
                        <option value="en">"English"</option>
                        <option value="zh">"中文"</option>
                        <option value="ru">"Русский"</option>
                    </select>
                </div>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Theme"</label>
                        <p class="setting-desc">"Application color scheme"</p>
                    </div>
                    <select>
                        <option value="dark">"Dark"</option>
                        <option value="light">"Light"</option>
                        <option value="system">"System"</option>
                    </select>
                </div>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Auto-launch"</label>
                        <p class="setting-desc">"Start with system"</p>
                    </div>
                    <input 
                        type="checkbox" 
                        class="toggle"
                        checked=move || state.config.get().map(|c| c.auto_launch).unwrap_or(false)
                    />
                </div>
            </section>
            
            // Quota Refresh
            <section class="settings-section">
                <h2>"Quota Refresh"</h2>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Auto-refresh quotas"</label>
                        <p class="setting-desc">"Automatically update account quotas"</p>
                    </div>
                    <input 
                        type="checkbox" 
                        class="toggle"
                        checked=move || state.config.get().map(|c| c.auto_refresh).unwrap_or(true)
                    />
                </div>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Refresh interval"</label>
                        <p class="setting-desc">"Minutes between quota updates"</p>
                    </div>
                    <input 
                        type="number" 
                        min="1" 
                        max="1440"
                        prop:value=move || state.config.get().map(|c| c.refresh_interval.to_string()).unwrap_or_default()
                    />
                </div>
            </section>
            
            // Paths
            <section class="settings-section">
                <h2>"Paths"</h2>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Export directory"</label>
                        <p class="setting-desc">"Default location for exported data"</p>
                    </div>
                    <div class="path-input">
                        <input 
                            type="text" 
                            readonly=true
                            prop:value=move || state.config.get()
                                .and_then(|c| c.default_export_path)
                                .unwrap_or_else(|| "Not set".to_string())
                        />
                        <button class="btn btn--icon">"📁"</button>
                    </div>
                </div>
            </section>
            
            // About
            <section class="settings-section settings-section--about">
                <h2>"About"</h2>
                
                <div class="about-info">
                    <div class="app-icon">"🚀"</div>
                    <div class="app-details">
                        <h3>"Antigravity Manager"</h3>
                        <p>"Version 3.3.20"</p>
                        <p class="links">
                            <a href="https://github.com/nicepkg/gpt-runner" target="_blank">"GitHub"</a>
                        </p>
                    </div>
                </div>
            </section>
            
            // Danger zone
            <section class="settings-section settings-section--danger">
                <h2>"Maintenance"</h2>
                
                <div class="setting-row">
                    <div class="setting-info">
                        <label>"Clear logs"</label>
                        <p class="setting-desc">"Remove all request logs"</p>
                    </div>
                    <Button 
                        text="Clear Logs".to_string()
                        variant=ButtonVariant::Danger
                        on_click=|| {}
                    />
                </div>
            </section>
        </div>
    }
}
