//! # Antigravity Core
//!
//! Core business logic for Antigravity Manager.
//! This crate contains all shared functionality extracted from the Tauri app:
//!
//! - **Models**: Data structures (Account, Token, Quota, Config)
//! - **Modules**: Business logic (account management, OAuth, quota fetching)
//! - **Proxy**: API proxy server (handlers, mappers, rate limiting)
//! - **Error**: Unified error types
//!
//! This crate is UI-agnostic and can be used with:
//! - Slint (native desktop)
//! - Tauri (webview desktop)
//! - CLI applications
//! - Headless servers

pub mod error;
pub mod models;
pub mod modules;
pub mod proxy;
pub mod utils;

// Re-export commonly used types
pub use error::{AppError, AppResult};
pub use models::{Account, AppConfig, QuotaData, TokenData};
