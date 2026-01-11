//! Business logic modules for Antigravity.
//!
//! Note: This is a minimal stub. Full implementation will be migrated
//! from src-tauri/src/modules/ incrementally.

pub mod account;
pub mod config;
pub mod logger;
pub mod proxy_db;

// Re-export for convenience
pub use account::*;
pub use config::*;
pub use logger::*;
pub use proxy_db::*;
