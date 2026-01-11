use antigravity_shared::models::{ProxyRequestLog, ProxyStats};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;

/// Trait for emitting proxy events to the UI/Subscriber
pub trait ProxyEventBus: Send + Sync {
    fn emit_request_log(&self, log: &ProxyRequestLog);
}

pub struct ProxyMonitor {
    pub logs: RwLock<VecDeque<ProxyRequestLog>>,
    pub stats: RwLock<ProxyStats>,
    pub max_logs: usize,
    pub enabled: AtomicBool,
    event_bus: Option<Box<dyn ProxyEventBus>>,
}

impl ProxyMonitor {
    pub fn new(max_logs: usize, event_bus: Option<Box<dyn ProxyEventBus>>) -> Self {
        // Initialize DB
        if let Err(e) = crate::modules::proxy_db::init_db() {
            tracing::error!("Failed to initialize proxy DB: {}", e);
        }

        Self {
            logs: RwLock::new(VecDeque::with_capacity(max_logs)),
            stats: RwLock::new(ProxyStats::default()),
            max_logs,
            enabled: AtomicBool::new(false), // Default to disabled
            event_bus,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub async fn log_request(&self, log: ProxyRequestLog) {
        if !self.is_enabled() {
            return;
        }
        tracing::info!("[Monitor] Logging request: {} {}", log.method, log.url);
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.total_requests += 1;
            if log.status >= 200 && log.status < 400 {
                stats.success_count += 1;
            } else {
                stats.error_count += 1;
            }
        }

        // Add log to memory
        {
            let mut logs = self.logs.write().await;
            if logs.len() >= self.max_logs {
                logs.pop_back();
            }
            logs.push_front(log.clone());
        }

        // Save to DB
        let log_to_save = log.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::modules::proxy_db::save_log(&log_to_save) {
                tracing::error!("Failed to save proxy log to DB: {}", e);
            }
        });

        // Emit event
        if let Some(bus) = &self.event_bus {
            bus.emit_request_log(&log);
        }
    }

    pub async fn get_logs(&self, limit: usize) -> Vec<ProxyRequestLog> {
        // Try to get from DB first for true history
        match crate::modules::proxy_db::get_logs(limit) {
            Ok(logs) => logs,
            Err(e) => {
                tracing::error!("Failed to get logs from DB: {}", e);
                // Fallback to memory
                let logs = self.logs.read().await;
                logs.iter().take(limit).cloned().collect()
            }
        }
    }

    pub async fn get_stats(&self) -> ProxyStats {
        match crate::modules::proxy_db::get_stats() {
            Ok(stats) => stats,
            Err(e) => {
                tracing::error!("Failed to get stats from DB: {}", e);
                self.stats.read().await.clone()
            }
        }
    }

    pub async fn clear(&self) {
        let mut logs = self.logs.write().await;
        logs.clear();
        let mut stats = self.stats.write().await;
        *stats = ProxyStats::default();

        if let Err(e) = crate::modules::proxy_db::clear_proxy_logs() {
            tracing::error!("Failed to clear logs in DB: {}", e);
        }
    }
}
