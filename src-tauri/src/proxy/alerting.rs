//! Alerting Module - Webhook-based health monitoring for Telegram/Slack
//!
//! Periodically checks system health and sends alerts to configured webhooks
//! when degradation is detected. Implements alert throttling and state management
//! to prevent notification floods.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Alerting severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// Alert notification payload
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    /// Unique alert identifier (e.g., "accounts_low")
    pub id: String,
    /// Human-readable alert title
    pub title: String,
    /// Alert description with details
    pub description: String,
    /// Severity level
    pub severity: AlertSeverity,
    /// Unix timestamp (seconds)
    pub timestamp: i64,
    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// Alerting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertingConfig {
    /// Enable alerting system
    pub enabled: bool,
    /// Health check interval (seconds)
    pub check_interval_secs: u64,
    /// Minimum time between duplicate alerts (seconds)
    pub cooldown_secs: u64,
    /// Telegram bot token (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram_bot_token: Option<String>,
    /// Telegram chat ID (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram_chat_id: Option<String>,
    /// Slack webhook URL (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slack_webhook_url: Option<String>,
    /// Custom webhook URL (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_webhook_url: Option<String>,
}

impl Default for AlertingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval_secs: 60, // Check every 1 minute
            cooldown_secs: 300,      // 5 minutes cooldown
            telegram_bot_token: None,
            telegram_chat_id: None,
            slack_webhook_url: None,
            custom_webhook_url: None,
        }
    }
}

/// Alert state tracker for deduplication
#[derive(Debug, Clone)]
struct AlertState {
    /// Last time this alert was sent
    last_sent: Instant,
    /// Number of times alert has fired
    fire_count: u64,
}

/// Alerting manager - orchestrates health checks and webhook dispatch
pub struct AlertManager {
    config: Arc<RwLock<AlertingConfig>>,
    client: Client,
    /// Tracks last alert send times for deduplication
    alert_states: Arc<RwLock<HashMap<String, AlertState>>>,
    /// Base URL for health check endpoint
    base_url: String,
}

impl AlertManager {
    /// Create new alert manager
    pub fn new(config: AlertingConfig, base_url: String) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to create HTTP client for alerting"),
            alert_states: Arc::new(RwLock::new(HashMap::new())),
            base_url,
        }
    }

    /// Update alerting configuration (hot-reload support)
    pub fn update_config(&self, config: AlertingConfig) {
        *self.config.write() = config;
        info!("Alerting configuration updated");
    }

    /// Start background monitoring task
    pub fn start_monitoring(self: Arc<Self>) {
        tokio::spawn(async move {
            info!("Starting alerting background task");
            loop {
                let config = self.config.read().clone();

                if !config.enabled {
                    debug!("Alerting disabled, sleeping for 60s");
                    sleep(Duration::from_secs(60)).await;
                    continue;
                }

                // Perform health check
                if let Err(e) = self.check_health().await {
                    error!("Health check failed: {}", e);
                }

                sleep(Duration::from_secs(config.check_interval_secs)).await;
            }
        });
    }

    /// Perform health check and trigger alerts if needed
    async fn check_health(&self) -> Result<(), Box<dyn std::error::Error>> {
        let health_url = format!("{}/api/health/detailed", self.base_url);

        debug!("Fetching health from {}", health_url);

        let response = self.client.get(&health_url).send().await?;

        if !response.status().is_success() {
            let alert = Alert {
                id: "health_check_failed".to_string(),
                title: "❌ Health Check Failed".to_string(),
                description: format!(
                    "Health endpoint returned status: {}",
                    response.status()
                ),
                severity: AlertSeverity::Critical,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: None,
            };

            self.send_alert(alert).await;
            return Ok(());
        }

        let health: serde_json::Value = response.json().await?;

        // Extract key metrics
        let status = health["status"].as_str().unwrap_or("unknown");
        let accounts_total = health["components"]["token_manager"]["accounts_total"]
            .as_u64()
            .unwrap_or(0);
        let accounts_available = health["components"]["token_manager"]["accounts_available"]
            .as_u64()
            .unwrap_or(0);
        let circuit_open = health["components"]["circuit_breaker"]["open"]
            .as_u64()
            .unwrap_or(0);

        // Evaluate alerting rules
        self.evaluate_rules(status, accounts_total, accounts_available, circuit_open)
            .await;

        Ok(())
    }

    /// Evaluate alerting rules based on health metrics
    async fn evaluate_rules(
        &self,
        status: &str,
        accounts_total: u64,
        accounts_available: u64,
        circuit_open: u64,
    ) {
        // Rule 1: System unhealthy
        if status == "unhealthy" {
            let alert = Alert {
                id: "system_unhealthy".to_string(),
                title: "🔴 System Unhealthy".to_string(),
                description: "Overall system status is unhealthy. Check component health."
                    .to_string(),
                severity: AlertSeverity::Critical,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: None,
            };
            self.send_alert(alert).await;
        }

        // Rule 2: System degraded
        if status == "degraded" {
            let alert = Alert {
                id: "system_degraded".to_string(),
                title: "⚠️ System Degraded".to_string(),
                description: "System is running in degraded mode. Some components may be impaired."
                    .to_string(),
                severity: AlertSeverity::Warning,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: None,
            };
            self.send_alert(alert).await;
        }

        // Rule 3: Low available accounts
        if accounts_available < 2 && accounts_total > 0 {
            let alert = Alert {
                id: "accounts_low".to_string(),
                title: "⚠️ Low Available Accounts".to_string(),
                description: format!(
                    "Only {} out of {} accounts are available. Service may become unavailable soon.",
                    accounts_available, accounts_total
                ),
                severity: AlertSeverity::Warning,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("accounts_total".to_string(), accounts_total.to_string());
                    map.insert(
                        "accounts_available".to_string(),
                        accounts_available.to_string(),
                    );
                    map
                }),
            };
            self.send_alert(alert).await;
        }

        // Rule 4: High circuit breaker open count
        if circuit_open > accounts_total / 2 && accounts_total > 0 {
            let alert = Alert {
                id: "circuit_breaker_high".to_string(),
                title: "🔌 High Circuit Breaker Failures".to_string(),
                description: format!(
                    "{} out of {} accounts have open circuit breakers. Check for upstream issues.",
                    circuit_open, accounts_total
                ),
                severity: AlertSeverity::Warning,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("circuit_open".to_string(), circuit_open.to_string());
                    map.insert("accounts_total".to_string(), accounts_total.to_string());
                    map
                }),
            };
            self.send_alert(alert).await;
        }

        // Rule 5: All accounts unavailable (critical)
        if accounts_available == 0 && accounts_total > 0 {
            let alert = Alert {
                id: "all_accounts_unavailable".to_string(),
                title: "🚨 ALL ACCOUNTS UNAVAILABLE".to_string(),
                description: "Service is down. All accounts are rate-limited or unavailable."
                    .to_string(),
                severity: AlertSeverity::Critical,
                timestamp: OffsetDateTime::now_utc().unix_timestamp(),
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("accounts_total".to_string(), accounts_total.to_string());
                    map
                }),
            };
            self.send_alert(alert).await;
        }
    }

    /// Send alert with deduplication
    async fn send_alert(&self, alert: Alert) {
        let cooldown = {
            let config = self.config.read();
            Duration::from_secs(config.cooldown_secs)
        };

        {
            let mut states = self.alert_states.write();
            let now = Instant::now();

            if let Some(state) = states.get_mut(&alert.id) {
                if now.duration_since(state.last_sent) < cooldown {
                    debug!(
                        "Alert {} is in cooldown, skipping (last sent {} seconds ago)",
                        alert.id,
                        now.duration_since(state.last_sent).as_secs()
                    );
                    return;
                }
                state.last_sent = now;
                state.fire_count += 1;
            } else {
                states.insert(
                    alert.id.clone(),
                    AlertState {
                        last_sent: now,
                        fire_count: 1,
                    },
                );
            }
        }

        info!(
            "Sending alert: {} ({})",
            alert.title,
            match alert.severity {
                AlertSeverity::Info => "INFO",
                AlertSeverity::Warning => "WARNING",
                AlertSeverity::Critical => "CRITICAL",
            }
        );

        let mut tasks = vec![];

        {
            let config = self.config.read();

            if let (Some(token), Some(chat_id)) =
                (&config.telegram_bot_token, &config.telegram_chat_id)
            {
                let client = self.client.clone();
                let alert = alert.clone();
                let token = token.clone();
                let chat_id = chat_id.clone();

                tasks.push(tokio::spawn(async move {
                    if let Err(e) = send_telegram_alert(&client, &token, &chat_id, &alert).await {
                        error!("Failed to send Telegram alert: {}", e);
                    }
                }));
            }

            if let Some(webhook_url) = &config.slack_webhook_url {
                let client = self.client.clone();
                let alert = alert.clone();
                let webhook_url = webhook_url.clone();

                tasks.push(tokio::spawn(async move {
                    if let Err(e) = send_slack_alert(&client, &webhook_url, &alert).await {
                        error!("Failed to send Slack alert: {}", e);
                    }
                }));
            }

            if let Some(webhook_url) = &config.custom_webhook_url {
                let client = self.client.clone();
                let alert = alert.clone();
                let webhook_url = webhook_url.clone();

                tasks.push(tokio::spawn(async move {
                    if let Err(e) = send_custom_webhook_alert(&client, &webhook_url, &alert).await
                    {
                        error!("Failed to send custom webhook alert: {}", e);
                    }
                }));
            }
        }

        for task in tasks {
            let _ = task.await;
        }
    }
}

/// Send alert to Telegram via Bot API
async fn send_telegram_alert(
    client: &Client,
    bot_token: &str,
    chat_id: &str,
    alert: &Alert,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);

    let icon = match alert.severity {
        AlertSeverity::Info => "ℹ️",
        AlertSeverity::Warning => "⚠️",
        AlertSeverity::Critical => "🚨",
    };

    let timestamp_str = OffsetDateTime::from_unix_timestamp(alert.timestamp)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let text = format!(
        "{} <b>{}</b>\n\n{}\n\n<i>Time: {}</i>",
        icon, alert.title, alert.description, timestamp_str
    );

    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
    });

    let response = client.post(&url).json(&payload).send().await?;

    if !response.status().is_success() {
        warn!(
            "Telegram API returned non-success status: {}",
            response.status()
        );
        let body = response.text().await?;
        warn!("Telegram API response: {}", body);
    } else {
        debug!("Telegram alert sent successfully");
    }

    Ok(())
}

/// Send alert to Slack via webhook
async fn send_slack_alert(
    client: &Client,
    webhook_url: &str,
    alert: &Alert,
) -> Result<(), Box<dyn std::error::Error>> {
    let color = match alert.severity {
        AlertSeverity::Info => "#36a64f",     // Green
        AlertSeverity::Warning => "#ff9900",  // Orange
        AlertSeverity::Critical => "#ff0000", // Red
    };

    let payload = serde_json::json!({
        "attachments": [{
            "color": color,
            "title": alert.title,
            "text": alert.description,
            "footer": "Antigravity Manager",
            "ts": alert.timestamp,
        }]
    });

    let response = client.post(webhook_url).json(&payload).send().await?;

    if !response.status().is_success() {
        warn!(
            "Slack webhook returned non-success status: {}",
            response.status()
        );
    } else {
        debug!("Slack alert sent successfully");
    }

    Ok(())
}

/// Send alert to custom webhook (JSON POST)
async fn send_custom_webhook_alert(
    client: &Client,
    webhook_url: &str,
    alert: &Alert,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client.post(webhook_url).json(&alert).send().await?;

    if !response.status().is_success() {
        warn!(
            "Custom webhook returned non-success status: {}",
            response.status()
        );
    } else {
        debug!("Custom webhook alert sent successfully");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alerting_config_default() {
        let config = AlertingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.check_interval_secs, 60);
        assert_eq!(config.cooldown_secs, 300);
    }

    #[test]
    fn test_alert_serialization() {
        let alert = Alert {
            id: "test_alert".to_string(),
            title: "Test Alert".to_string(),
            description: "This is a test".to_string(),
            severity: AlertSeverity::Warning,
            timestamp: 1234567890,
            metadata: None,
        };

        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("test_alert"));
        assert!(json.contains("warning"));
    }

    #[tokio::test]
    async fn test_alert_manager_creation() {
        let config = AlertingConfig::default();
        let manager = AlertManager::new(config, "http://localhost:9101".to_string());

        // Should not panic
        assert_eq!(manager.base_url, "http://localhost:9101");
    }
}
