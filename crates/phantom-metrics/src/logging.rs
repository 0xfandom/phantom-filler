//! Structured logging with the `tracing` ecosystem.
//!
//! Provides a configurable logging setup using `tracing-subscriber` with
//! JSON or pretty-printed output, per-module filtering, and optional
//! file output.

use serde::{Deserialize, Serialize};
use tracing::Level;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{filter::LevelFilter, fmt, EnvFilter};

/// Log output format.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Human-readable pretty-printed output.
    #[default]
    Pretty,
    /// Machine-readable JSON output.
    Json,
    /// Compact single-line output.
    Compact,
}

/// Configuration for the logging system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Whether logging is enabled.
    pub enabled: bool,
    /// Default log level.
    pub level: String,
    /// Output format.
    pub format: LogFormat,
    /// Per-module log level overrides (e.g., "phantom_chain=debug").
    pub module_filters: Vec<String>,
    /// Whether to include source file/line in log output.
    pub include_location: bool,
    /// Whether to include the current span in log output.
    pub include_span: bool,
    /// Whether to include thread IDs.
    pub include_thread_id: bool,
    /// Whether to include the target module path.
    pub include_target: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            format: LogFormat::Pretty,
            module_filters: Vec::new(),
            include_location: false,
            include_span: true,
            include_thread_id: false,
            include_target: true,
        }
    }
}

impl LoggingConfig {
    /// Builds the `EnvFilter` from the configured level and module overrides.
    ///
    /// The `RUST_LOG` environment variable takes precedence if set.
    pub fn build_filter(&self) -> EnvFilter {
        // Start with RUST_LOG env var if present.
        if let Ok(env) = std::env::var("RUST_LOG") {
            return EnvFilter::new(env);
        }

        let mut directives = vec![self.level.clone()];
        directives.extend(self.module_filters.iter().cloned());
        EnvFilter::new(directives.join(","))
    }

    /// Returns the `LevelFilter` corresponding to the configured level string.
    pub fn level_filter(&self) -> LevelFilter {
        match self.level.to_lowercase().as_str() {
            "trace" => LevelFilter::TRACE,
            "debug" => LevelFilter::DEBUG,
            "info" => LevelFilter::INFO,
            "warn" => LevelFilter::WARN,
            "error" => LevelFilter::ERROR,
            "off" => LevelFilter::OFF,
            _ => LevelFilter::INFO,
        }
    }
}

/// Initializes the global tracing subscriber.
///
/// Must be called once at startup before any `tracing` macros are used.
/// Returns an error if the subscriber has already been set.
pub fn init_logging(config: &LoggingConfig) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let env_filter = config.build_filter();

    match config.format {
        LogFormat::Json => {
            let layer = fmt::layer()
                .json()
                .with_timer(SystemTime)
                .with_target(config.include_target)
                .with_file(config.include_location)
                .with_line_number(config.include_location)
                .with_thread_ids(config.include_thread_id)
                .with_span_list(config.include_span);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow::anyhow!("failed to initialize logging: {e}"))?;
        }
        LogFormat::Pretty => {
            let layer = fmt::layer()
                .pretty()
                .with_timer(SystemTime)
                .with_target(config.include_target)
                .with_file(config.include_location)
                .with_line_number(config.include_location)
                .with_thread_ids(config.include_thread_id);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow::anyhow!("failed to initialize logging: {e}"))?;
        }
        LogFormat::Compact => {
            let layer = fmt::layer()
                .compact()
                .with_timer(SystemTime)
                .with_target(config.include_target)
                .with_file(config.include_location)
                .with_line_number(config.include_location)
                .with_thread_ids(config.include_thread_id);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow::anyhow!("failed to initialize logging: {e}"))?;
        }
    }

    tracing::info!(
        level = %config.level,
        format = ?config.format,
        "logging initialized"
    );

    Ok(())
}

/// Standard span names used across the system for consistent tracing.
pub mod spans {
    /// Span for order processing pipeline.
    pub const ORDER_PROCESSING: &str = "order_processing";
    /// Span for fill execution.
    pub const FILL_EXECUTION: &str = "fill_execution";
    /// Span for price fetching.
    pub const PRICE_FETCH: &str = "price_fetch";
    /// Span for strategy evaluation.
    pub const STRATEGY_EVAL: &str = "strategy_eval";
    /// Span for chain RPC calls.
    pub const RPC_CALL: &str = "rpc_call";
    /// Span for settlement confirmation.
    pub const SETTLEMENT: &str = "settlement";
    /// Span for risk checking.
    pub const RISK_CHECK: &str = "risk_check";
}

/// Standard field names for structured log entries.
pub mod fields {
    /// Chain identifier.
    pub const CHAIN_ID: &str = "chain_id";
    /// Order identifier.
    pub const ORDER_ID: &str = "order_id";
    /// Transaction hash.
    pub const TX_HASH: &str = "tx_hash";
    /// Token address.
    pub const TOKEN: &str = "token";
    /// Strategy name.
    pub const STRATEGY: &str = "strategy";
    /// Duration in milliseconds.
    pub const DURATION_MS: &str = "duration_ms";
    /// Profit/loss in wei.
    pub const PNL_WEI: &str = "pnl_wei";
    /// Gas used.
    pub const GAS_USED: &str = "gas_used";
    /// Block number.
    pub const BLOCK_NUMBER: &str = "block_number";
}

/// Converts a `tracing::Level` to its string representation.
pub fn level_to_string(level: Level) -> &'static str {
    match level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default() {
        let config = LoggingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.level, "info");
        assert_eq!(config.format, LogFormat::Pretty);
        assert!(!config.include_location);
        assert!(config.include_span);
        assert!(!config.include_thread_id);
        assert!(config.include_target);
        assert!(config.module_filters.is_empty());
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = LoggingConfig {
            enabled: true,
            level: "debug".to_string(),
            format: LogFormat::Json,
            module_filters: vec!["phantom_chain=trace".to_string()],
            include_location: true,
            include_span: true,
            include_thread_id: true,
            include_target: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: LoggingConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.level, "debug");
        assert_eq!(deserialized.format, LogFormat::Json);
        assert_eq!(deserialized.module_filters.len(), 1);
        assert!(deserialized.include_location);
    }

    #[test]
    fn log_format_serde() {
        for format in [LogFormat::Pretty, LogFormat::Json, LogFormat::Compact] {
            let json = serde_json::to_string(&format).expect("serialize");
            let deserialized: LogFormat = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(format, deserialized);
        }
    }

    #[test]
    fn log_format_default() {
        assert_eq!(LogFormat::default(), LogFormat::Pretty);
    }

    #[test]
    fn level_filter_mapping() {
        let cases = [
            ("trace", LevelFilter::TRACE),
            ("debug", LevelFilter::DEBUG),
            ("info", LevelFilter::INFO),
            ("warn", LevelFilter::WARN),
            ("error", LevelFilter::ERROR),
            ("off", LevelFilter::OFF),
            ("INFO", LevelFilter::INFO),
            ("DEBUG", LevelFilter::DEBUG),
            ("unknown", LevelFilter::INFO), // fallback
        ];
        for (input, expected) in cases {
            let config = LoggingConfig {
                level: input.to_string(),
                ..Default::default()
            };
            assert_eq!(config.level_filter(), expected, "failed for: {input}");
        }
    }

    #[test]
    fn build_filter_default() {
        // Temporarily remove RUST_LOG to test default behavior.
        let original = std::env::var("RUST_LOG").ok();
        std::env::remove_var("RUST_LOG");

        let config = LoggingConfig::default();
        let filter = config.build_filter();
        // Should parse successfully.
        let _ = format!("{filter:?}");

        // Restore.
        if let Some(val) = original {
            std::env::set_var("RUST_LOG", val);
        }
    }

    #[test]
    fn build_filter_with_modules() {
        let original = std::env::var("RUST_LOG").ok();
        std::env::remove_var("RUST_LOG");

        let config = LoggingConfig {
            level: "warn".to_string(),
            module_filters: vec![
                "phantom_chain=debug".to_string(),
                "phantom_execution=trace".to_string(),
            ],
            ..Default::default()
        };
        let filter = config.build_filter();
        let debug_str = format!("{filter:?}");
        assert!(!debug_str.is_empty());

        if let Some(val) = original {
            std::env::set_var("RUST_LOG", val);
        }
    }

    #[test]
    fn init_logging_disabled() {
        let config = LoggingConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(init_logging(&config).is_ok());
    }

    #[test]
    fn span_name_constants() {
        assert!(!spans::ORDER_PROCESSING.is_empty());
        assert!(!spans::FILL_EXECUTION.is_empty());
        assert!(!spans::PRICE_FETCH.is_empty());
        assert!(!spans::STRATEGY_EVAL.is_empty());
        assert!(!spans::RPC_CALL.is_empty());
        assert!(!spans::SETTLEMENT.is_empty());
        assert!(!spans::RISK_CHECK.is_empty());
    }

    #[test]
    fn field_name_constants() {
        assert!(!fields::CHAIN_ID.is_empty());
        assert!(!fields::ORDER_ID.is_empty());
        assert!(!fields::TX_HASH.is_empty());
        assert!(!fields::TOKEN.is_empty());
        assert!(!fields::STRATEGY.is_empty());
        assert!(!fields::DURATION_MS.is_empty());
        assert!(!fields::PNL_WEI.is_empty());
        assert!(!fields::GAS_USED.is_empty());
        assert!(!fields::BLOCK_NUMBER.is_empty());
    }

    #[test]
    fn level_to_string_mapping() {
        assert_eq!(level_to_string(Level::TRACE), "trace");
        assert_eq!(level_to_string(Level::DEBUG), "debug");
        assert_eq!(level_to_string(Level::INFO), "info");
        assert_eq!(level_to_string(Level::WARN), "warn");
        assert_eq!(level_to_string(Level::ERROR), "error");
    }

    #[test]
    fn field_names_unique() {
        let names = [
            fields::CHAIN_ID,
            fields::ORDER_ID,
            fields::TX_HASH,
            fields::TOKEN,
            fields::STRATEGY,
            fields::DURATION_MS,
            fields::PNL_WEI,
            fields::GAS_USED,
            fields::BLOCK_NUMBER,
        ];
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(name), "duplicate field name: {name}");
        }
    }

    #[test]
    fn span_names_unique() {
        let names = [
            spans::ORDER_PROCESSING,
            spans::FILL_EXECUTION,
            spans::PRICE_FETCH,
            spans::STRATEGY_EVAL,
            spans::RPC_CALL,
            spans::SETTLEMENT,
            spans::RISK_CHECK,
        ];
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(name), "duplicate span name: {name}");
        }
    }
}
