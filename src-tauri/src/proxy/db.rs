use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::proxy::monitor::ProxyRequestLog;

pub fn get_proxy_db_path() -> Result<PathBuf, String> {
    let data_dir = crate::modules::account::get_data_dir()?;
    Ok(data_dir.join("proxy_logs.db"))
}

/// Schema version for migrations
/// Used for documentation and debugging - actual migration logic uses version table
#[allow(dead_code)]
const SCHEMA_VERSION: i32 = 3;

pub fn init_db() -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // Enable foreign keys and WAL mode for better performance
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;"
    ).map_err(|e| e.to_string())?;

    // Create schema_version table if not exists
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
        [],
    ).map_err(|e| e.to_string())?;

    // Get current schema version
    let current_version: i32 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |row| row.get(0))
        .unwrap_or(0);

    // Run migrations
    if current_version < 1 {
        migrate_v1(&conn)?;
    }
    if current_version < 2 {
        migrate_v2(&conn)?;
    }
    if current_version < 3 {
        migrate_v3(&conn)?;
    }

    Ok(())
}

/// Migration v1: Original request_logs table with added columns
fn migrate_v1(conn: &Connection) -> Result<(), String> {
    tracing::info!("Running database migration v1: request_logs table");

    conn.execute(
        "CREATE TABLE IF NOT EXISTS request_logs (
            id TEXT PRIMARY KEY,
            timestamp INTEGER,
            method TEXT,
            url TEXT,
            status INTEGER,
            duration INTEGER,
            model TEXT,
            error TEXT
        )",
        [],
    ).map_err(|e| e.to_string())?;

    // Try to add new columns (ignore errors if they exist)
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN request_body TEXT", []);
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN response_body TEXT", []);
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN input_tokens INTEGER", []);
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN output_tokens INTEGER", []);
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN resolved_model TEXT", []);

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_timestamp ON request_logs (timestamp DESC)",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (1)", [])
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Migration v2: Analytics persistence tables
fn migrate_v2(conn: &Connection) -> Result<(), String> {
    tracing::info!("Running database migration v2: analytics persistence tables");

    // Add account_id column to request_logs if not exists
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN account_id TEXT", []);
    let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN provider TEXT", []);

    // Create index for account_id lookups
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_account ON request_logs (account_id, timestamp DESC)",
        [],
    );

    // Daily aggregated analytics per account
    // Stores pre-computed daily stats to avoid scanning request_logs
    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_account_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL,
            date TEXT NOT NULL,  -- YYYY-MM-DD format
            request_count INTEGER DEFAULT 0,
            success_count INTEGER DEFAULT 0,
            error_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            total_duration_ms INTEGER DEFAULT 0,
            rate_limit_hits INTEGER DEFAULT 0,
            updated_at INTEGER NOT NULL,
            UNIQUE(account_id, date)
        )",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_daily_stats_date ON daily_account_stats (date DESC)",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_daily_stats_account ON daily_account_stats (account_id, date DESC)",
        [],
    ).map_err(|e| e.to_string())?;

    // Circuit breaker state change events (audit trail)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS circuit_breaker_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            previous_state TEXT NOT NULL,  -- 'closed', 'open', 'half_open'
            new_state TEXT NOT NULL,
            reason TEXT,
            failure_count INTEGER
        )",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_cb_events_account ON circuit_breaker_events (account_id, timestamp DESC)",
        [],
    ).map_err(|e| e.to_string())?;

    // Rate limit events (track when accounts get rate limited)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rate_limit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            reset_at INTEGER,  -- Unix timestamp when rate limit resets
            quota_group TEXT,  -- e.g., 'gemini', 'claude', etc.
            retry_after_seconds INTEGER
        )",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_rate_limit_account ON rate_limit_events (account_id, timestamp DESC)",
        [],
    ).map_err(|e| e.to_string())?;

    // Global summary stats (for fast dashboard queries)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS global_stats (
            key TEXT PRIMARY KEY,
            value INTEGER DEFAULT 0,
            updated_at INTEGER
        )",
        [],
    ).map_err(|e| e.to_string())?;

    // Initialize global stats if not exist
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO global_stats (key, value, updated_at) VALUES ('total_requests', 0, ?1)",
        [now],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO global_stats (key, value, updated_at) VALUES ('total_tokens', 0, ?1)",
        [now],
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO global_stats (key, value, updated_at) VALUES ('total_circuit_trips', 0, ?1)",
        [now],
    ).map_err(|e| e.to_string())?;

    conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (2)", [])
        .map_err(|e| e.to_string())?;

    tracing::info!("Database migration v2 completed successfully");
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<(), String> {
    tracing::info!("Running database migration v3: adaptive rate limit persistence");

    conn.execute(
        "CREATE TABLE IF NOT EXISTS adaptive_limits (
            account_id TEXT PRIMARY KEY,
            confirmed_limit INTEGER NOT NULL,
            ceiling INTEGER NOT NULL,
            last_calibration INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        [],
    ).map_err(|e| e.to_string())?;

    conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (3)", [])
        .map_err(|e| e.to_string())?;

    tracing::info!("Database migration v3 completed successfully");
    Ok(())
}

pub fn save_log(log: &ProxyRequestLog) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO request_logs (id, timestamp, method, url, status, duration, model, resolved_model, error, request_body, response_body, input_tokens, output_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            log.id,
            log.timestamp,
            log.method,
            log.url,
            log.status,
            log.duration,
            log.model,
            log.resolved_model,
            log.error,
            log.request_body,
            log.response_body,
            log.input_tokens,
            log.output_tokens,
        ],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn get_logs(limit: usize) -> Result<Vec<ProxyRequestLog>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT id, timestamp, method, url, status, duration, model, resolved_model, error, request_body, response_body, input_tokens, output_tokens
         FROM request_logs
         ORDER BY timestamp DESC
         LIMIT ?1"
    ).map_err(|e| e.to_string())?;

    let logs_iter = stmt.query_map([limit], |row| {
        Ok(ProxyRequestLog {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            method: row.get(2)?,
            url: row.get(3)?,
            status: row.get(4)?,
            duration: row.get(5)?,
            model: row.get(6)?,
            resolved_model: row.get(7).unwrap_or(None),
            error: row.get(8)?,
            request_body: row.get(9).unwrap_or(None),
            response_body: row.get(10).unwrap_or(None),
            input_tokens: row.get(11).unwrap_or(None),
            output_tokens: row.get(12).unwrap_or(None),
        })
    }).map_err(|e| e.to_string())?;

    let mut logs = Vec::new();
    for log in logs_iter {
        logs.push(log.map_err(|e| e.to_string())?);
    }
    Ok(logs)
}

pub fn get_stats() -> Result<crate::proxy::monitor::ProxyStats, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let total_requests: u64 = conn.query_row(
        "SELECT COUNT(*) FROM request_logs",
        [],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    let success_count: u64 = conn.query_row(
        "SELECT COUNT(*) FROM request_logs WHERE status >= 200 AND status < 400",
        [],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    let error_count: u64 = conn.query_row(
        "SELECT COUNT(*) FROM request_logs WHERE status < 200 OR status >= 400",
        [],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    Ok(crate::proxy::monitor::ProxyStats {
        total_requests,
        success_count,
        error_count,
    })
}

pub fn clear_logs() -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM request_logs", []).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// Analytics Data Structures
// ============================================================================

/// Daily aggregated statistics for a single account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyAccountStats {
    pub account_id: String,
    pub date: String,  // YYYY-MM-DD
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_duration_ms: i64,
    pub rate_limit_hits: i64,
}

impl DailyAccountStats {
    /// Calculate success rate (0.0 - 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.request_count > 0 {
            self.success_count as f64 / self.request_count as f64
        } else {
            1.0
        }
    }

    /// Calculate average latency in ms
    pub fn avg_latency_ms(&self) -> f64 {
        if self.request_count > 0 {
            self.total_duration_ms as f64 / self.request_count as f64
        } else {
            0.0
        }
    }

    /// Total tokens (input + output)
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }
}

/// Circuit breaker state change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerEvent {
    pub id: i64,
    pub account_id: String,
    pub timestamp: i64,
    pub previous_state: String,
    pub new_state: String,
    pub reason: Option<String>,
    pub failure_count: Option<i32>,
}

/// Rate limit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitEvent {
    pub id: i64,
    pub account_id: String,
    pub timestamp: i64,
    pub reset_at: Option<i64>,
    pub quota_group: Option<String>,
    pub retry_after_seconds: Option<i32>,
}

/// Summary of account analytics across all time
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountAnalyticsSummary {
    pub account_id: String,
    pub total_requests: i64,
    pub total_success: i64,
    pub total_errors: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_rate_limit_hits: i64,
    pub requests_today: i64,
    pub success_rate: f64,
    pub circuit_breaker_trips: i64,
}

/// Historical analytics for the last N days
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalAnalytics {
    pub daily_stats: Vec<DailyAccountStats>,
    pub total_requests: i64,
    pub total_success: i64,
    pub total_errors: i64,
    pub total_tokens: i64,
    pub avg_success_rate: f64,
}

// ============================================================================
// Analytics Write Operations
// ============================================================================

/// Save log with account_id and provider information
pub fn save_log_with_account(
    log: &ProxyRequestLog,
    account_id: Option<&str>,
    provider: Option<&str>,
) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO request_logs (id, timestamp, method, url, status, duration, model, resolved_model, error, request_body, response_body, input_tokens, output_tokens, account_id, provider)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            log.id,
            log.timestamp,
            log.method,
            log.url,
            log.status,
            log.duration,
            log.model,
            log.resolved_model,
            log.error,
            log.request_body,
            log.response_body,
            log.input_tokens,
            log.output_tokens,
            account_id,
            provider,
        ],
    ).map_err(|e| e.to_string())?;

    // Update daily stats if account_id is provided
    if let Some(acc_id) = account_id {
        update_daily_stats(&conn, acc_id, log)?;
    }

    Ok(())
}

/// Update daily aggregated stats for an account
fn update_daily_stats(conn: &Connection, account_id: &str, log: &ProxyRequestLog) -> Result<(), String> {
    let now = time::OffsetDateTime::now_utc();
    let date = now.date();
    let date_str = format!("{:04}-{:02}-{:02}", date.year(), date.month() as u8, date.day());
    let timestamp = now.unix_timestamp();

    let is_success = log.status >= 200 && log.status < 400;
    let is_rate_limit = log.status == 429;
    let input_tokens = i64::from(log.input_tokens.unwrap_or(0));
    let output_tokens = i64::from(log.output_tokens.unwrap_or(0));

    conn.execute(
        "INSERT INTO daily_account_stats
            (account_id, date, request_count, success_count, error_count,
             input_tokens, output_tokens, total_duration_ms, rate_limit_hits, updated_at)
         VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(account_id, date) DO UPDATE SET
            request_count = request_count + 1,
            success_count = success_count + ?3,
            error_count = error_count + ?4,
            input_tokens = input_tokens + ?5,
            output_tokens = output_tokens + ?6,
            total_duration_ms = total_duration_ms + ?7,
            rate_limit_hits = rate_limit_hits + ?8,
            updated_at = ?9",
        params![
            account_id,
            date_str,
            i32::from(is_success),
            i32::from(!is_success),
            input_tokens,
            output_tokens,
            i64::try_from(log.duration).unwrap_or(i64::MAX),
            i32::from(is_rate_limit),
            timestamp,
        ],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Record a circuit breaker state change
pub fn record_circuit_breaker_event(
    account_id: &str,
    previous_state: &str,
    new_state: &str,
    reason: Option<&str>,
    failure_count: Option<i32>,
) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();

    conn.execute(
        "INSERT INTO circuit_breaker_events (account_id, timestamp, previous_state, new_state, reason, failure_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![account_id, timestamp, previous_state, new_state, reason, failure_count],
    ).map_err(|e| e.to_string())?;

    // Update global stats if this is a trip (transition to open)
    if new_state == "open" {
        conn.execute(
            "UPDATE global_stats SET value = value + 1, updated_at = ?1 WHERE key = 'total_circuit_trips'",
            [timestamp],
        ).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Record a rate limit event
pub fn record_rate_limit_event(
    account_id: &str,
    reset_at: Option<i64>,
    quota_group: Option<&str>,
    retry_after_seconds: Option<i32>,
) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();

    conn.execute(
        "INSERT INTO rate_limit_events (account_id, timestamp, reset_at, quota_group, retry_after_seconds)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![account_id, timestamp, reset_at, quota_group, retry_after_seconds],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Analytics Query Operations
// ============================================================================

/// Get daily stats for a specific account
pub fn get_account_daily_stats(account_id: &str, days: i32) -> Result<Vec<DailyAccountStats>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT account_id, date, request_count, success_count, error_count,
                input_tokens, output_tokens, total_duration_ms, rate_limit_hits
         FROM daily_account_stats
         WHERE account_id = ?1
         ORDER BY date DESC
         LIMIT ?2"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map(params![account_id, days], |row| {
        Ok(DailyAccountStats {
            account_id: row.get(0)?,
            date: row.get(1)?,
            request_count: row.get(2)?,
            success_count: row.get(3)?,
            error_count: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            total_duration_ms: row.get(7)?,
            rate_limit_hits: row.get(8)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row.map_err(|e| e.to_string())?);
    }
    Ok(stats)
}

/// Get today's stats for all accounts
pub fn get_today_stats_all_accounts() -> Result<Vec<DailyAccountStats>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let now = time::OffsetDateTime::now_utc();
    let date = now.date();
    let date_str = format!("{:04}-{:02}-{:02}", date.year(), date.month() as u8, date.day());

    let mut stmt = conn.prepare(
        "SELECT account_id, date, request_count, success_count, error_count,
                input_tokens, output_tokens, total_duration_ms, rate_limit_hits
         FROM daily_account_stats
         WHERE date = ?1
         ORDER BY request_count DESC"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map([&date_str], |row| {
        Ok(DailyAccountStats {
            account_id: row.get(0)?,
            date: row.get(1)?,
            request_count: row.get(2)?,
            success_count: row.get(3)?,
            error_count: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            total_duration_ms: row.get(7)?,
            rate_limit_hits: row.get(8)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row.map_err(|e| e.to_string())?);
    }
    Ok(stats)
}

/// Get aggregated summary for a specific account
pub fn get_account_summary(account_id: &str) -> Result<AccountAnalyticsSummary, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let now = time::OffsetDateTime::now_utc();
    let date = now.date();
    let today_str = format!("{:04}-{:02}-{:02}", date.year(), date.month() as u8, date.day());

    // All-time stats
    let (total_requests, total_success, total_errors, total_input, total_output, total_rate_limits):
        (i64, i64, i64, i64, i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(request_count), 0), COALESCE(SUM(success_count), 0),
                COALESCE(SUM(error_count), 0), COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0), COALESCE(SUM(rate_limit_hits), 0)
         FROM daily_account_stats WHERE account_id = ?1",
        [account_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap_or((0, 0, 0, 0, 0, 0));

    // Today's requests
    let requests_today: i64 = conn.query_row(
        "SELECT COALESCE(request_count, 0) FROM daily_account_stats
         WHERE account_id = ?1 AND date = ?2",
        params![account_id, today_str],
        |row| row.get(0),
    ).unwrap_or(0);

    // Circuit breaker trips for this account
    let circuit_trips: i64 = conn.query_row(
        "SELECT COUNT(*) FROM circuit_breaker_events
         WHERE account_id = ?1 AND new_state = 'open'",
        [account_id],
        |row| row.get(0),
    ).unwrap_or(0);

    let success_rate = if total_requests > 0 {
        total_success as f64 / total_requests as f64
    } else {
        1.0
    };

    Ok(AccountAnalyticsSummary {
        account_id: account_id.to_string(),
        total_requests,
        total_success,
        total_errors,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_rate_limit_hits: total_rate_limits,
        requests_today,
        success_rate,
        circuit_breaker_trips: circuit_trips,
    })
}

/// Get historical analytics for all accounts over the last N days
pub fn get_historical_analytics(days: i32) -> Result<HistoricalAnalytics, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // Calculate date threshold
    let now = time::OffsetDateTime::now_utc();
    let threshold_date = now - time::Duration::days(i64::from(days));
    let threshold_str = format!(
        "{:04}-{:02}-{:02}",
        threshold_date.date().year(),
        threshold_date.date().month() as u8,
        threshold_date.date().day()
    );

    let mut stmt = conn.prepare(
        "SELECT account_id, date, request_count, success_count, error_count,
                input_tokens, output_tokens, total_duration_ms, rate_limit_hits
         FROM daily_account_stats
         WHERE date >= ?1
         ORDER BY date DESC, request_count DESC"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map([&threshold_str], |row| {
        Ok(DailyAccountStats {
            account_id: row.get(0)?,
            date: row.get(1)?,
            request_count: row.get(2)?,
            success_count: row.get(3)?,
            error_count: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            total_duration_ms: row.get(7)?,
            rate_limit_hits: row.get(8)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut daily_stats = Vec::new();
    let mut total_requests = 0i64;
    let mut total_success = 0i64;
    let mut total_errors = 0i64;
    let mut total_tokens = 0i64;

    for row in rows {
        let stat = row.map_err(|e| e.to_string())?;
        total_requests += stat.request_count;
        total_success += stat.success_count;
        total_errors += stat.error_count;
        total_tokens += stat.total_tokens();
        daily_stats.push(stat);
    }

    let avg_success_rate = if total_requests > 0 {
        total_success as f64 / total_requests as f64
    } else {
        1.0
    };

    Ok(HistoricalAnalytics {
        daily_stats,
        total_requests,
        total_success,
        total_errors,
        total_tokens,
        avg_success_rate,
    })
}

/// Get circuit breaker events for an account
pub fn get_circuit_breaker_events(account_id: &str, limit: i32) -> Result<Vec<CircuitBreakerEvent>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT id, account_id, timestamp, previous_state, new_state, reason, failure_count
         FROM circuit_breaker_events
         WHERE account_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map(params![account_id, limit], |row| {
        Ok(CircuitBreakerEvent {
            id: row.get(0)?,
            account_id: row.get(1)?,
            timestamp: row.get(2)?,
            previous_state: row.get(3)?,
            new_state: row.get(4)?,
            reason: row.get(5)?,
            failure_count: row.get(6)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|e| e.to_string())?);
    }
    Ok(events)
}

/// Get rate limit events for an account
pub fn get_rate_limit_events(account_id: &str, limit: i32) -> Result<Vec<RateLimitEvent>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT id, account_id, timestamp, reset_at, quota_group, retry_after_seconds
         FROM rate_limit_events
         WHERE account_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map(params![account_id, limit], |row| {
        Ok(RateLimitEvent {
            id: row.get(0)?,
            account_id: row.get(1)?,
            timestamp: row.get(2)?,
            reset_at: row.get(3)?,
            quota_group: row.get(4)?,
            retry_after_seconds: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|e| e.to_string())?);
    }
    Ok(events)
}

/// Get global summary statistics
pub fn get_global_stats() -> Result<std::collections::HashMap<String, i64>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare("SELECT key, value FROM global_stats")
        .map_err(|e| e.to_string())?;

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }).map_err(|e| e.to_string())?;

    let mut stats = std::collections::HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        stats.insert(key, value);
    }
    Ok(stats)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAdaptiveLimit {
    pub account_id: String,
    pub confirmed_limit: u64,
    pub ceiling: u64,
    pub last_calibration: i64,
}

pub fn save_adaptive_limit(
    account_id: &str,
    confirmed_limit: u64,
    ceiling: u64,
) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    conn.execute(
        "INSERT INTO adaptive_limits (account_id, confirmed_limit, ceiling, last_calibration, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(account_id) DO UPDATE SET
             confirmed_limit = ?2,
             ceiling = ?3,
             last_calibration = ?4,
             updated_at = ?4",
        params![account_id, i64::try_from(confirmed_limit).unwrap_or(i64::MAX), i64::try_from(ceiling).unwrap_or(i64::MAX), now],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn save_adaptive_limits_batch(
    limits: &[(String, u64, u64)],
) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    for (account_id, confirmed_limit, ceiling) in limits {
        conn.execute(
            "INSERT INTO adaptive_limits (account_id, confirmed_limit, ceiling, last_calibration, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(account_id) DO UPDATE SET
                 confirmed_limit = ?2,
                 ceiling = ?3,
                 last_calibration = ?4,
                 updated_at = ?4",
            params![account_id, i64::try_from(*confirmed_limit).unwrap_or(i64::MAX), i64::try_from(*ceiling).unwrap_or(i64::MAX), now],
        ).map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn load_adaptive_limits() -> Result<Vec<PersistedAdaptiveLimit>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT account_id, confirmed_limit, ceiling, last_calibration FROM adaptive_limits"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map([], |row| {
        Ok(PersistedAdaptiveLimit {
            account_id: row.get(0)?,
            confirmed_limit: row.get::<_, i64>(1)? as u64,
            ceiling: row.get::<_, i64>(2)? as u64,
            last_calibration: row.get(3)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut limits = Vec::new();
    for row in rows {
        limits.push(row.map_err(|e| e.to_string())?);
    }
    Ok(limits)
}

pub fn load_adaptive_limit(account_id: &str) -> Result<Option<PersistedAdaptiveLimit>, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let result = conn.query_row(
        "SELECT account_id, confirmed_limit, ceiling, last_calibration
         FROM adaptive_limits WHERE account_id = ?1",
        [account_id],
        |row| {
            Ok(PersistedAdaptiveLimit {
                account_id: row.get(0)?,
                confirmed_limit: row.get::<_, i64>(1)? as u64,
                ceiling: row.get::<_, i64>(2)? as u64,
                last_calibration: row.get(3)?,
            })
        },
    );

    match result {
        Ok(limit) => Ok(Some(limit)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete_adaptive_limit(account_id: &str) -> Result<(), String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    conn.execute(
        "DELETE FROM adaptive_limits WHERE account_id = ?1",
        [account_id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Get aggregated stats from request_logs for backwards compatibility
/// This computes stats from individual logs when daily_account_stats hasn't been populated
pub fn compute_account_stats_from_logs(account_id: &str) -> Result<AccountAnalyticsSummary, String> {
    let db_path = get_proxy_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let now = time::OffsetDateTime::now_utc();
    let today_start = now.date();
    let today_start_ts = time::PrimitiveDateTime::new(
        today_start,
        time::Time::MIDNIGHT,
    ).assume_utc().unix_timestamp() * 1000; // Convert to millis

    // Total stats
    let (total_requests, total_success, total_input, total_output): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*),
                SUM(CASE WHEN status >= 200 AND status < 400 THEN 1 ELSE 0 END),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0)
         FROM request_logs WHERE account_id = ?1",
        [account_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap_or((0, 0, 0, 0));

    // Today's requests
    let requests_today: i64 = conn.query_row(
        "SELECT COUNT(*) FROM request_logs WHERE account_id = ?1 AND timestamp >= ?2",
        params![account_id, today_start_ts],
        |row| row.get(0),
    ).unwrap_or(0);

    // Rate limit hits
    let rate_limits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM request_logs WHERE account_id = ?1 AND status = 429",
        [account_id],
        |row| row.get(0),
    ).unwrap_or(0);

    let total_errors = total_requests - total_success;
    let success_rate = if total_requests > 0 {
        total_success as f64 / total_requests as f64
    } else {
        1.0
    };

    Ok(AccountAnalyticsSummary {
        account_id: account_id.to_string(),
        total_requests,
        total_success,
        total_errors,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_rate_limit_hits: rate_limits,
        requests_today,
        success_rate,
        circuit_breaker_trips: 0, // Not tracked in request_logs
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily_account_stats_success_rate() {
        let stats = DailyAccountStats {
            account_id: "test".to_string(),
            date: "2026-01-07".to_string(),
            request_count: 100,
            success_count: 95,
            error_count: 5,
            input_tokens: 10000,
            output_tokens: 5000,
            total_duration_ms: 50000,
            rate_limit_hits: 2,
        };

        assert!((stats.success_rate() - 0.95).abs() < 0.001);
        assert!((stats.avg_latency_ms() - 500.0).abs() < 0.001);
        assert_eq!(stats.total_tokens(), 15000);
    }

    #[test]
    fn test_daily_account_stats_empty() {
        let stats = DailyAccountStats {
            account_id: "test".to_string(),
            date: "2026-01-07".to_string(),
            request_count: 0,
            success_count: 0,
            error_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_duration_ms: 0,
            rate_limit_hits: 0,
        };

        assert!((stats.success_rate() - 1.0).abs() < 0.001); // No requests = 100% success
        assert!((stats.avg_latency_ms() - 0.0).abs() < 0.001);
    }
}
