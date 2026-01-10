//! Antigravity Manager - Native Desktop UI
//!
//! This is the Slint-based native desktop application.
//! It replaces the Tauri WebView-based frontend with pure Rust rendering.

use antigravity_core::modules::logger;

slint::include_modules!();

fn main() {
    // Initialize logging
    logger::init_logger();
    tracing::info!("Antigravity Manager starting...");

    // Create and run the main window
    let app = match AppWindow::new() {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Failed to create application window: {}", e);
            std::process::exit(1);
        }
    };

    // TODO: Connect backend logic to UI callbacks
    
    tracing::info!("Application window created, running event loop...");
    
    if let Err(e) = app.run() {
        tracing::error!("Application error: {}", e);
        std::process::exit(1);
    }
}
