//! Antigravity Manager - Native Desktop UI
//!
//! This is the Slint-based native desktop application.
//! It replaces the Tauri WebView-based frontend with pure Rust rendering.

mod backend;

use antigravity_core::modules::logger;

slint::include_modules!();

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
    {
        let mut state = runtime.block_on(backend.lock());
        if let Err(e) = state.load_accounts() {
            tracing::warn!("Failed to load accounts: {}", e);
        } else {
            tracing::info!("Accounts loaded: {}", state.account_count());
        }
    }

    // Create and run the main window
    let app = match AppWindow::new() {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Failed to create application window: {}", e);
            std::process::exit(1);
        }
    };

    // Set initial stats from backend
    {
        let state = runtime.block_on(backend.lock());
        // Note: We'll need to update Dashboard component to accept dynamic data
        // For now, the UI shows placeholder values
        tracing::info!(
            "Stats: {} accounts, {:.0}% avg Gemini, {:.0}% avg Claude, {} low quota",
            state.account_count(),
            state.avg_gemini_quota(),
            state.avg_claude_quota(),
            state.low_quota_count()
        );
    }

    tracing::info!("Application window created, running event loop...");

    if let Err(e) = app.run() {
        tracing::error!("Application error: {}", e);
        std::process::exit(1);
    }
}
