use antigravity_core::proxy::monitor::ProxyEventBus;
use antigravity_shared::models::ProxyRequestLog;
use tauri::{AppHandle, Emitter};

// Re-export shared models
pub use antigravity_core::proxy::monitor::ProxyMonitor;
pub use antigravity_shared::models::ProxyStats;

pub struct TauriEventBus {
    app_handle: AppHandle,
}

impl TauriEventBus {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

impl ProxyEventBus for TauriEventBus {
    fn emit_request_log(&self, log: &ProxyRequestLog) {
        // Emit the event to the frontend
        // Note: The event name "proxy://request" must match what the frontend listens to
        let _ = self.app_handle.emit("proxy://request", log);
    }
}
