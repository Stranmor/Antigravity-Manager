pub mod account;
pub mod quota;
pub mod config;
pub mod logger;
pub mod db;
pub mod process;
pub mod oauth;
#[cfg(feature = "desktop")]
pub mod oauth_server;
pub mod migration;
#[cfg(feature = "desktop")]
pub mod tray;
#[cfg(feature = "desktop")]
pub mod i18n;
pub mod proxy_db;

use crate::models;

// Re-export commonly used functions to modules namespace top-level
pub use account::*;
#[allow(unused_imports)]
pub use quota::*;
pub use config::*;
#[allow(unused_imports)]
pub use logger::*;

pub async fn fetch_quota(access_token: &str, email: &str) -> crate::error::AppResult<(models::QuotaData, Option<String>)> {
    quota::fetch_quota(access_token, email).await
}
