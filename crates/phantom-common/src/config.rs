//! Configuration system with TOML parsing and environment variable overrides.

use crate::types::ChainId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Chain-specific configurations.
    #[serde(default)]
    pub chains: HashMap<String, ChainConfig>,
    /// Database configuration.
    pub database: DatabaseConfig,
    /// Redis cache configuration.
    pub redis: RedisConfig,
    /// Strategy engine configuration.
    pub strategy: StrategyConfig,
    /// Execution engine configuration.
    pub execution: ExecutionConfig,
    /// API server configuration.
    pub api: ApiConfig,
    /// Metrics and observability configuration.
    pub metrics: MetricsConfig,
}

/// Configuration for a single chain connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainConfig {
    /// Chain identifier.
    pub chain_id: ChainId,
    /// HTTP RPC endpoint URL.
    pub rpc_url: String,
    /// WebSocket RPC endpoint URL.
    pub ws_url: Option<String>,
    /// Maximum number of concurrent RPC requests.
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: usize,
    /// Request timeout in milliseconds.
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    /// Whether to enable mempool monitoring.
    #[serde(default)]
    pub mempool_enabled: bool,
}

/// PostgreSQL database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Database connection URL.
    pub url: String,
    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// Minimum number of idle connections.
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
    /// Connection timeout in seconds.
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
}

/// Redis cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedisConfig {
    /// Redis connection URL.
    pub url: String,
    /// Maximum number of connections in the pool.
    #[serde(default = "default_redis_pool_size")]
    pub pool_size: usize,
}

/// Strategy engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyConfig {
    /// Minimum profit threshold in basis points to execute a fill.
    #[serde(default = "default_min_profit_bps")]
    pub min_profit_bps: u32,
    /// Maximum gas price willing to pay (in gwei).
    #[serde(default = "default_max_gas_price_gwei")]
    pub max_gas_price_gwei: u64,
    /// Strategy evaluation timeout in milliseconds.
    #[serde(default = "default_evaluation_timeout_ms")]
    pub evaluation_timeout_ms: u64,
    /// Enabled strategy names.
    #[serde(default = "default_enabled_strategies")]
    pub enabled_strategies: Vec<String>,
}

/// Execution engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    /// Private key for signing transactions (hex-encoded, without 0x prefix).
    /// In production, use a KMS or vault.
    pub private_key: Option<String>,
    /// Whether to use Flashbots for private transaction submission.
    #[serde(default)]
    pub flashbots_enabled: bool,
    /// Flashbots relay URL.
    pub flashbots_relay_url: Option<String>,
    /// Maximum number of retry attempts for failed transactions.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Base retry delay in milliseconds (exponential backoff).
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,
}

/// REST/WebSocket API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// Host to bind the API server to.
    #[serde(default = "default_api_host")]
    pub host: String,
    /// Port to bind the API server to.
    #[serde(default = "default_api_port")]
    pub port: u16,
    /// Whether to enable WebSocket feeds.
    #[serde(default = "default_true")]
    pub websocket_enabled: bool,
}

/// Metrics and observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    /// Whether to enable Prometheus metrics endpoint.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Port for the Prometheus metrics endpoint.
    #[serde(default = "default_metrics_port")]
    pub port: u16,
    /// Log level (trace, debug, info, warn, error).
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Whether to output logs in JSON format.
    #[serde(default)]
    pub json_logs: bool,
}

// Default value functions

fn default_max_concurrent_requests() -> usize {
    64
}
fn default_request_timeout_ms() -> u64 {
    5000
}
fn default_max_connections() -> u32 {
    10
}
fn default_min_connections() -> u32 {
    2
}
fn default_connect_timeout_secs() -> u64 {
    10
}
fn default_redis_pool_size() -> usize {
    16
}
fn default_min_profit_bps() -> u32 {
    10
}
fn default_max_gas_price_gwei() -> u64 {
    100
}
fn default_evaluation_timeout_ms() -> u64 {
    50
}
fn default_enabled_strategies() -> Vec<String> {
    vec!["simple_arb".into()]
}
fn default_max_retries() -> u32 {
    3
}
fn default_retry_delay_ms() -> u64 {
    1000
}
fn default_api_host() -> String {
    "127.0.0.1".into()
}
fn default_api_port() -> u16 {
    8080
}
fn default_metrics_port() -> u16 {
    9090
}
fn default_log_level() -> String {
    "info".into()
}
fn default_true() -> bool {
    true
}

impl AppConfig {
    /// Loads configuration from a TOML file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content =
            std::fs::read_to_string(path.as_ref()).map_err(|e| ConfigError::FileRead {
                path: path.as_ref().display().to_string(),
                reason: e.to_string(),
            })?;
        Self::from_toml_str(&content)
    }

    /// Parses configuration from a TOML string.
    pub fn from_toml_str(content: &str) -> Result<Self, ConfigError> {
        let config: AppConfig =
            toml::from_str(content).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Applies environment variable overrides with the `PHANTOM_` prefix.
    ///
    /// Supported overrides:
    /// - `PHANTOM_DATABASE_URL` → `database.url`
    /// - `PHANTOM_REDIS_URL` → `redis.url`
    /// - `PHANTOM_API_HOST` → `api.host`
    /// - `PHANTOM_API_PORT` → `api.port`
    /// - `PHANTOM_LOG_LEVEL` → `metrics.log_level`
    /// - `PHANTOM_PRIVATE_KEY` → `execution.private_key`
    pub fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("PHANTOM_DATABASE_URL") {
            self.database.url = val;
        }
        if let Ok(val) = std::env::var("PHANTOM_REDIS_URL") {
            self.redis.url = val;
        }
        if let Ok(val) = std::env::var("PHANTOM_API_HOST") {
            self.api.host = val;
        }
        if let Ok(val) = std::env::var("PHANTOM_API_PORT") {
            if let Ok(port) = val.parse() {
                self.api.port = port;
            }
        }
        if let Ok(val) = std::env::var("PHANTOM_LOG_LEVEL") {
            self.metrics.log_level = val;
        }
        if let Ok(val) = std::env::var("PHANTOM_PRIVATE_KEY") {
            self.execution.private_key = Some(val);
        }
    }

    /// Validates the configuration for consistency and completeness.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.chains.is_empty() {
            return Err(ConfigError::Validation(
                "at least one chain must be configured".into(),
            ));
        }

        for (name, chain) in &self.chains {
            if chain.rpc_url.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "chain '{name}' has empty rpc_url"
                )));
            }
        }

        if self.database.url.is_empty() {
            return Err(ConfigError::Validation(
                "database.url must not be empty".into(),
            ));
        }

        if self.redis.url.is_empty() {
            return Err(ConfigError::Validation(
                "redis.url must not be empty".into(),
            ));
        }

        if self.database.max_connections == 0 {
            return Err(ConfigError::Validation(
                "database.max_connections must be > 0".into(),
            ));
        }

        Ok(())
    }
}

/// Configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Failed to read the configuration file.
    #[error("failed to read config file '{path}': {reason}")]
    FileRead { path: String, reason: String },

    /// Failed to parse TOML content.
    #[error("config parse error: {0}")]
    ParseError(String),

    /// Configuration validation failed.
    #[error("config validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CONFIG: &str = r#"
[chains.ethereum]
chain_id = "ethereum"
rpc_url = "https://eth-mainnet.example.com"
ws_url = "wss://eth-mainnet.example.com"
mempool_enabled = true

[chains.arbitrum]
chain_id = "arbitrum"
rpc_url = "https://arb-mainnet.example.com"

[database]
url = "postgres://user:pass@localhost:5432/phantom"

[redis]
url = "redis://localhost:6379"

[strategy]
min_profit_bps = 25
max_gas_price_gwei = 50

[execution]
flashbots_enabled = true
flashbots_relay_url = "https://relay.flashbots.net"

[api]
port = 3000

[metrics]
log_level = "debug"
json_logs = true
"#;

    #[test]
    fn parse_valid_config() {
        let config = AppConfig::from_toml_str(VALID_CONFIG).expect("valid config");
        assert_eq!(config.chains.len(), 2);
        assert!(config.chains.contains_key("ethereum"));
        assert!(config.chains.contains_key("arbitrum"));
        assert_eq!(config.chains["ethereum"].chain_id, ChainId::Ethereum);
        assert_eq!(config.api.port, 3000);
        assert_eq!(config.strategy.min_profit_bps, 25);
    }

    #[test]
    fn defaults_applied() {
        let config = AppConfig::from_toml_str(VALID_CONFIG).expect("valid config");
        // These should use defaults since not specified
        assert_eq!(config.database.max_connections, 10);
        assert_eq!(config.redis.pool_size, 16);
        assert_eq!(config.execution.max_retries, 3);
        assert_eq!(config.api.host, "127.0.0.1");
        assert!(config.api.websocket_enabled);
        assert!(config.metrics.enabled);
    }

    #[test]
    fn empty_chains_fails_validation() {
        let toml = r#"
[database]
url = "postgres://localhost/phantom"
[redis]
url = "redis://localhost"
[strategy]
[execution]
[api]
[metrics]
"#;
        let err = AppConfig::from_toml_str(toml).unwrap_err();
        assert!(err.to_string().contains("at least one chain"));
    }

    #[test]
    fn empty_rpc_url_fails_validation() {
        let toml = r#"
[chains.ethereum]
chain_id = "ethereum"
rpc_url = ""
[database]
url = "postgres://localhost/phantom"
[redis]
url = "redis://localhost"
[strategy]
[execution]
[api]
[metrics]
"#;
        let err = AppConfig::from_toml_str(toml).unwrap_err();
        assert!(err.to_string().contains("empty rpc_url"));
    }

    #[test]
    fn empty_database_url_fails_validation() {
        let toml = r#"
[chains.ethereum]
chain_id = "ethereum"
rpc_url = "https://eth.example.com"
[database]
url = ""
[redis]
url = "redis://localhost"
[strategy]
[execution]
[api]
[metrics]
"#;
        let err = AppConfig::from_toml_str(toml).unwrap_err();
        assert!(err.to_string().contains("database.url"));
    }

    #[test]
    fn serde_roundtrip() {
        let config = AppConfig::from_toml_str(VALID_CONFIG).expect("valid config");
        let toml_str = toml::to_string(&config).expect("serialize");
        let reparsed = AppConfig::from_toml_str(&toml_str).expect("reparse");
        assert_eq!(config.chains.len(), reparsed.chains.len());
        assert_eq!(config.api.port, reparsed.api.port);
    }

    #[test]
    fn env_overrides() {
        let mut config = AppConfig::from_toml_str(VALID_CONFIG).expect("valid config");

        std::env::set_var("PHANTOM_API_PORT", "9999");
        std::env::set_var("PHANTOM_LOG_LEVEL", "trace");
        config.apply_env_overrides();

        assert_eq!(config.api.port, 9999);
        assert_eq!(config.metrics.log_level, "trace");

        // Cleanup
        std::env::remove_var("PHANTOM_API_PORT");
        std::env::remove_var("PHANTOM_LOG_LEVEL");
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let err = AppConfig::from_toml_str("not valid toml {{{").unwrap_err();
        assert!(matches!(err, ConfigError::ParseError(_)));
    }

    #[test]
    fn config_error_display() {
        let err = ConfigError::Validation("test error".into());
        assert!(err.to_string().contains("test error"));
    }
}
