use tracing::{info, warn, error};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use std::fs;
use std::path::PathBuf;
use crate::modules::account::get_data_dir;

// Custom local timezone time formatter for console output
struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let format = time::format_description::well_known::Rfc3339;
        if let Ok(formatted) = now.format(&format) {
            write!(w, "{formatted}")
        } else {
            write!(w, "{now}")
        }
    }
}

pub fn get_log_dir() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    let log_dir = data_dir.join("logs");

    if !log_dir.exists() {
        fs::create_dir_all(&log_dir).map_err(|e| format!("Failed to create log directory: {e}"))?;
    }

    Ok(log_dir)
}

/// Initialize logging system with human-readable console and JSON file output
pub fn init_logger() {
    // Capture log macro logs
    let _ = tracing_log::LogTracer::init();

    let log_dir = match get_log_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("Failed to initialize log directory: {e}");
            return;
        }
    };

    // 1. Setup file appender with daily rotation
    let file_appender = tracing_appender::rolling::daily(&log_dir, "app.log");
    let (non_blocking_file, file_guard) = tracing_appender::non_blocking(file_appender);

    // 2. Console layer - human-readable format with local timezone
    let console_layer = fmt::Layer::new()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true)
        .with_timer(LocalTimer);

    // 3. File layer - structured JSON format for log analysis
    // Includes: timestamp (ISO8601), level, target, message, span fields (request_id)
    let file_layer = fmt::Layer::new()
        .json()
        .with_writer(non_blocking_file)
        .with_target(true)
        .with_current_span(true)
        .with_span_list(true)
        .flatten_event(false);

    // 4. Setup filter layer (default INFO level to reduce log volume)
    let filter_layer = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // 5. Initialize global subscriber (use try_init to avoid panic on duplicate init)
    let _ = tracing_subscriber::registry()
        .with(filter_layer)
        .with(console_layer)
        .with(file_layer)
        .try_init();

    // Leak guards to ensure their lifetime extends until program exit
    // This is the recommended approach when using tracing_appender::non_blocking
    let _ = Box::leak(Box::new(file_guard));

    info!("Logging system initialized (console: human-readable, file: JSON)");
}

/// Clear log cache (uses truncate mode to keep file handles valid)
pub fn clear_logs() -> Result<(), String> {
    let log_dir = get_log_dir()?;
    if log_dir.exists() {
        // Iterate through all files and truncate instead of deleting directory
        let entries = fs::read_dir(&log_dir).map_err(|e| format!("Failed to read log directory: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                // Open file in truncate mode to set size to 0
                let _ = fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(path);
            }
        }
    }
    Ok(())
}

/// Log info message (backward compatible interface)
pub fn log_info(message: &str) {
    info!("{}", message);
}

/// Log warning message (backward compatible interface)
pub fn log_warn(message: &str) {
    warn!("{}", message);
}

/// Log error message (backward compatible interface)
pub fn log_error(message: &str) {
    error!("{}", message);
}
