//! Antigravity Server - Headless Daemon
//!
//! A pure Rust HTTP server that:
//! - Runs the proxy logic (account rotation, API forwarding) on /v1/*
//! - Serves the Leptos WebUI as static files
//! - Provides a REST API for CLI and UI control on /api/*
//!
//! Access via: http://localhost:8045

use anyhow::Result;
use axum::{
    extract::DefaultBodyLimit, http::StatusCode, response::IntoResponse, routing::get, Router,
};
use std::net::SocketAddr;
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod api;
mod state;

use state::AppState;

const DEFAULT_PORT: u16 = 8045;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("🚀 Antigravity Server starting...");

    // Initialize application state
    let state = AppState::new().await?;
    info!("✅ Application state initialized");
    info!("📊 {} accounts loaded", state.get_account_count());

    // Build the router with proxy integrated
    let app = build_router(state);

    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    info!("🌐 Server listening on http://{}", addr);
    info!("📊 WebUI available at http://localhost:{}/", DEFAULT_PORT);
    info!("🔌 API available at http://localhost:{}/api/", DEFAULT_PORT);
    info!(
        "🔀 Proxy endpoints at http://localhost:{}/v1/",
        DEFAULT_PORT
    );

    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(state: AppState) -> Router {
    // Get proxy router from core (has its own state already applied)
    let proxy_router = state.build_proxy_router();

    // Static files for WebUI (Leptos dist)
    let static_dir =
        std::env::var("ANTIGRAVITY_STATIC_DIR").unwrap_or_else(|_| "./src-leptos/dist".to_string());

    // API router with AppState
    let api_routes = Router::new()
        .nest("/api", api::router())
        .route("/health", get(health_check))
        .route("/healthz", get(health_check))
        .with_state(state);

    // Combine: API routes + Proxy routes + Static files fallback
    // Both api_routes and proxy_router are Router<()> after with_state()
    api_routes
        .merge(proxy_router)
        .fallback_service(ServeDir::new(&static_dir).append_index_html_on_directories(true))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
}

async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({"status": "ok"})),
    )
}
