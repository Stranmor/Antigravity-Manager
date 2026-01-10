//! Dashboard page

use leptos::prelude::*;
use crate::app::AppState;
use crate::components::StatsCard;
use crate::types::DashboardStats;

#[component]
pub fn Dashboard() -> impl IntoView {
    let state = expect_context::<AppState>();
    
    // Compute stats from accounts
    let stats = Memo::new(move |_| {
        DashboardStats::from_accounts(&state.accounts.get())
    });
    
    // Current account info
    let current_account = Memo::new(move |_| {
        let current_id = state.current_account_id.get();
        current_id.and_then(|id| {
            state.accounts.get().into_iter().find(|a| a.id == id)
        })
    });

    view! {
        <div class="page dashboard">
            <header class="page-header">
                <h1>"Dashboard"</h1>
                <p class="subtitle">"Overview of your Antigravity accounts"</p>
            </header>
            
            // Stats grid
            <section class="stats-grid">
                <StatsCard 
                    title="Total Accounts".to_string()
                    value=Signal::derive(move || stats.get().total_accounts.to_string())
                    icon="👥".to_string()
                    color="blue".to_string()
                />
                <StatsCard 
                    title="Avg Gemini Quota".to_string()
                    value=Signal::derive(move || format!("{}%", stats.get().avg_gemini_quota))
                    icon="✨".to_string()
                    color="purple".to_string()
                />
                <StatsCard 
                    title="Avg Image Quota".to_string()
                    value=Signal::derive(move || format!("{}%", stats.get().avg_gemini_image_quota))
                    icon="🎨".to_string()
                    color="pink".to_string()
                />
                <StatsCard 
                    title="Avg Claude Quota".to_string()
                    value=Signal::derive(move || format!("{}%", stats.get().avg_claude_quota))
                    icon="🤖".to_string()
                    color="orange".to_string()
                />
            </section>
            
            // Tier breakdown
            <section class="tier-section">
                <h2>"Account Tiers"</h2>
                <div class="tier-grid">
                    <div class="tier-card tier-card--ultra">
                        <span class="tier-count">{move || stats.get().ultra_count}</span>
                        <span class="tier-label">"Ultra"</span>
                    </div>
                    <div class="tier-card tier-card--pro">
                        <span class="tier-count">{move || stats.get().pro_count}</span>
                        <span class="tier-label">"Pro"</span>
                    </div>
                    <div class="tier-card tier-card--free">
                        <span class="tier-count">{move || stats.get().free_count}</span>
                        <span class="tier-label">"Free"</span>
                    </div>
                    <div class="tier-card tier-card--warning">
                        <span class="tier-count">{move || stats.get().low_quota_count}</span>
                        <span class="tier-label">"Low Quota"</span>
                    </div>
                </div>
            </section>
            
            // Current account
            <section class="current-account-section">
                <h2>"Current Account"</h2>
                {move || match current_account.get() {
                    Some(account) => view! {
                        <div class="current-account-card">
                            <div class="account-info">
                                <span class="email">{account.email.clone()}</span>
                            </div>
                            <a href="/accounts" class="btn btn--secondary">"Switch Account"</a>
                        </div>
                    }.into_any(),
                    None => view! {
                        <div class="no-account">
                            <p>"No account selected"</p>
                            <a href="/accounts" class="btn btn--primary">"Select Account"</a>
                        </div>
                    }.into_any()
                }}
            </section>
            
            // Quick actions
            <section class="quick-actions">
                <h2>"Quick Actions"</h2>
                <div class="action-grid">
                    <a href="/accounts" class="action-card">
                        <span class="action-icon">"➕"</span>
                        <span class="action-label">"Add Account"</span>
                    </a>
                    <a href="/proxy" class="action-card">
                        <span class="action-icon">"🔌"</span>
                        <span class="action-label">"Start Proxy"</span>
                    </a>
                    <a href="/monitor" class="action-card">
                        <span class="action-icon">"📡"</span>
                        <span class="action-label">"View Logs"</span>
                    </a>
                    <a href="/settings" class="action-card">
                        <span class="action-icon">"⚙️"</span>
                        <span class="action-label">"Settings"</span>
                    </a>
                </div>
            </section>
        </div>
    }
}
