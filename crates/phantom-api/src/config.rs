//! API server configuration.

use serde::{Deserialize, Serialize};

/// Configuration for the REST API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Whether the API server is enabled.
    pub enabled: bool,
    /// Listen address (e.g., "0.0.0.0").
    pub listen_addr: String,
    /// Listen port.
    pub port: u16,
    /// Allowed CORS origins. Empty = allow all.
    pub cors_origins: Vec<String>,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_addr: "0.0.0.0".to_string(),
            port: 8080,
            cors_origins: Vec::new(),
            request_timeout_secs: 30,
        }
    }
}

impl ApiConfig {
    /// Returns the socket address string for binding.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.listen_addr, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = ApiConfig::default();
        assert!(config.enabled);
        assert_eq!(config.listen_addr, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert!(config.cors_origins.is_empty());
        assert_eq!(config.request_timeout_secs, 30);
    }

    #[test]
    fn bind_addr() {
        let config = ApiConfig::default();
        assert_eq!(config.bind_addr(), "0.0.0.0:8080");
    }

    #[test]
    fn bind_addr_custom() {
        let config = ApiConfig {
            listen_addr: "127.0.0.1".to_string(),
            port: 3000,
            ..Default::default()
        };
        assert_eq!(config.bind_addr(), "127.0.0.1:3000");
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = ApiConfig {
            enabled: false,
            listen_addr: "127.0.0.1".to_string(),
            port: 9090,
            cors_origins: vec!["http://localhost:3000".to_string()],
            request_timeout_secs: 60,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ApiConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.port, 9090);
        assert_eq!(deserialized.cors_origins.len(), 1);
        assert!(!deserialized.enabled);
    }
}
