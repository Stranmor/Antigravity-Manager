// Middleware 模块 - Axum 中间件

pub mod auth;
pub mod cors;
pub mod logging;
pub mod monitor;
pub mod request_id;

pub use auth::auth_middleware;
pub use cors::cors_layer;
pub use request_id::{request_id_middleware, RequestId};

// Re-export X_REQUEST_ID_HEADER for external use
#[allow(unused_imports)]
pub use request_id::X_REQUEST_ID_HEADER;
