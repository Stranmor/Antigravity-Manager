//! Server Logger with Log Rotation
//!
//! This module provides advanced logging for the headless server with:
//! - Daily/hourly/size-based log rotation
//! - Automatic retention cleanup (max 7 days by default)
//! - Optional gzip compression for rotated files
//! - Separate log files for proxy requests, errors, and debug
//! - Non-blocking async writes
//! - UTC timezone for consistent file naming
//!
//! Uses `logroller` crate for SOTA log rotation (2026).

use logroller::{Compression, LogRoller, LogRollerBuilder, Rotation, RotationAge, RotationSize, TimeZone};
use std::path::Path;
use tracing::{info, warn, Level};
use tracing_subscriber::{
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

use super::config::LogRotationConfig;

/// Guards for non-blocking log writers that must be kept alive
pub struct LogGuards {
    /// Main log file guard
    pub main_guard: tracing_appender::non_blocking::WorkerGuard,
    /// Error log file guard (optional, only when separate_by_level is enabled)
    pub error_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    /// Debug log file guard (optional, only when separate_by_level is enabled)
    pub debug_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Build a `LogRoller` from configuration
fn build_log_roller(
    log_dir: &Path,
    file_prefix: &str,
    config: &LogRotationConfig,
) -> Result<LogRoller, String> {
    let mut builder = LogRollerBuilder::new(log_dir, Path::new(file_prefix));

    // Set rotation strategy
    match config.strategy.as_str() {
        "daily" => {
            builder = builder.rotation(Rotation::AgeBased(RotationAge::Daily));
        }
        "hourly" => {
            builder = builder.rotation(Rotation::AgeBased(RotationAge::Hourly));
        }
        "size" => {
            // Size-based rotation in MB
            builder = builder.rotation(Rotation::SizeBased(RotationSize::MB(config.max_size_mb)));
        }
        _ => {
            // Default to daily
            builder = builder.rotation(Rotation::AgeBased(RotationAge::Daily));
        }
    }

    // Set retention policy (max files to keep)
    builder = builder.max_keep_files(config.max_files as u64);

    // Set timezone
    if config.use_utc {
        builder = builder.time_zone(TimeZone::UTC);
    } else {
        builder = builder.time_zone(TimeZone::Local);
    }

    // Enable compression if configured
    if config.compress {
        builder = builder.compression(Compression::Gzip);
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build LogRoller: {e}"))
}

/// Wrapper to make LogRoller work with tracing's MakeWriter trait
struct LogRollerWriter {
    roller: LogRoller,
}

impl LogRollerWriter {
    fn new(roller: LogRoller) -> Self {
        Self { roller }
    }
}

impl std::io::Write for LogRollerWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.roller.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.roller.flush()
    }
}

/// Initialize the server logging system with log rotation
///
/// # Arguments
/// * `log_dir` - Directory to store log files
/// * `config` - Log rotation configuration
///
/// # Returns
/// * `LogGuards` - Guards that must be kept alive for the duration of the server
pub fn init_server_logger(log_dir: &Path, config: &LogRotationConfig) -> Result<LogGuards, String> {
    // Ensure log directory exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)
            .map_err(|e| format!("Failed to create log directory: {e}"))?;
    }

    // Build filter from environment or default
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,antigravity_tools_lib=debug"));

    // Console layer - human-readable output
    let console_layer = fmt::Layer::new()
        .with_target(true)
        .with_thread_ids(false)
        .with_level(true)
        .with_ansi(true);

    if config.enabled {
        // Main log roller with rotation
        let main_roller = build_log_roller(log_dir, "server.log", config)?;
        let (non_blocking_main, main_guard) =
            tracing_appender::non_blocking(LogRollerWriter::new(main_roller));

        // Main file layer - JSON structured logging
        let main_file_layer = fmt::Layer::new()
            .json()
            .with_writer(non_blocking_main)
            .with_target(true)
            .with_current_span(true)
            .with_span_list(true)
            .flatten_event(false);

        if config.separate_by_level {
            // Create separate log files for different levels
            let error_roller = build_log_roller(log_dir, "errors.log", config)?;
            let (non_blocking_error, error_guard) =
                tracing_appender::non_blocking(LogRollerWriter::new(error_roller));

            let debug_roller = build_log_roller(log_dir, "debug.log", config)?;
            let (non_blocking_debug, debug_guard) =
                tracing_appender::non_blocking(LogRollerWriter::new(debug_roller));

            // Error-only layer (ERROR and WARN)
            let error_layer = fmt::Layer::new()
                .json()
                .with_writer(non_blocking_error)
                .with_target(true)
                .with_current_span(true)
                .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
                    Level::WARN,
                ));

            // Debug layer (DEBUG and TRACE only)
            let debug_layer = fmt::Layer::new()
                .json()
                .with_writer(non_blocking_debug)
                .with_target(true)
                .with_current_span(true)
                .with_filter(tracing_subscriber::filter::filter_fn(|meta| {
                    *meta.level() >= Level::DEBUG
                }));

            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(console_layer)
                .with(main_file_layer)
                .with(error_layer)
                .with(debug_layer)
                .try_init();

            Ok(LogGuards {
                main_guard,
                error_guard: Some(error_guard),
                debug_guard: Some(debug_guard),
            })
        } else {
            // Single log file for all levels
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(console_layer)
                .with(main_file_layer)
                .try_init();

            Ok(LogGuards {
                main_guard,
                error_guard: None,
                debug_guard: None,
            })
        }
    } else {
        // Rotation disabled - use basic tracing-appender rolling
        let file_appender = tracing_appender::rolling::daily(log_dir, "server.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        let file_layer = fmt::Layer::new()
            .json()
            .with_writer(non_blocking)
            .with_target(true)
            .with_current_span(true);

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(console_layer)
            .with(file_layer)
            .try_init();

        Ok(LogGuards {
            main_guard: guard,
            error_guard: None,
            debug_guard: None,
        })
    }
}

/// Clean up old log files based on retention policy
///
/// This is called periodically to ensure old logs are removed even if
/// the server hasn't rotated files recently (e.g., low traffic periods).
pub async fn cleanup_old_logs(log_dir: &Path, max_age_days: u64) -> Result<usize, String> {
    use std::time::{Duration, SystemTime};

    let max_age = Duration::from_secs(max_age_days * 24 * 60 * 60);
    let now = SystemTime::now();
    let mut removed = 0;

    let entries = std::fs::read_dir(log_dir)
        .map_err(|e| format!("Failed to read log directory: {e}"))?;

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process log files
        if !path.is_file() {
            continue;
        }

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(extension, "log" | "gz") {
            continue;
        }

        // Check file age
        if let Ok(metadata) = path.metadata() {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age > max_age {
                        if let Err(e) = std::fs::remove_file(&path) {
                            warn!("Failed to remove old log file {:?}: {}", path, e);
                        } else {
                            info!("Removed old log file: {:?}", path);
                            removed += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(removed)
}

/// Start a background task to periodically clean up old logs
pub fn start_log_cleanup_task(
    log_dir: std::path::PathBuf,
    max_age_days: u64,
    interval_hours: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = tokio::time::Duration::from_secs(interval_hours * 60 * 60);

        loop {
            tokio::time::sleep(interval).await;

            match cleanup_old_logs(&log_dir, max_age_days).await {
                Ok(count) if count > 0 => {
                    info!("Log cleanup: removed {} old log files", count);
                }
                Ok(_) => {
                    // No files removed, nothing to log
                }
                Err(e) => {
                    warn!("Log cleanup failed: {}", e);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = LogRotationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.strategy, "daily");
        assert_eq!(config.max_files, 7);
        assert!(!config.compress);
        assert!(config.use_utc);
    }

    #[test]
    fn test_build_log_roller_daily() {
        let temp_dir = TempDir::new().unwrap();
        let config = LogRotationConfig::default();

        let result = build_log_roller(temp_dir.path(), "test.log", &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_log_roller_size_based() {
        let temp_dir = TempDir::new().unwrap();
        let config = LogRotationConfig {
            enabled: true,
            strategy: "size".to_string(),
            max_files: 5,
            compress: true,
            max_size_mb: 50,
            use_utc: true,
            separate_by_level: false,
        };

        let result = build_log_roller(temp_dir.path(), "test.log", &config);
        assert!(result.is_ok());
    }
}
