//! Health checks and circuit breaker pattern.
//!
//! Provides a component health registry for liveness/readiness probes and
//! a circuit breaker that trips after consecutive failures and auto-resets
//! after a cooldown period.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

// ─── Health Check Registry ───────────────────────────────────────────

/// Status of an individual component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    /// Component is operating normally.
    Healthy,
    /// Component is degraded but partially functional.
    Degraded,
    /// Component is not functional.
    Unhealthy,
}

/// Health report for a single component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Component name.
    pub name: String,
    /// Current status.
    pub status: ComponentStatus,
    /// Human-readable status message.
    pub message: String,
    /// Unix timestamp of last status update.
    pub last_updated: u64,
}

/// Overall system health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemStatus {
    /// All components healthy.
    Healthy,
    /// Some components degraded but system is functional.
    Degraded,
    /// Critical components unhealthy.
    Unhealthy,
}

/// Full system health report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Overall system status.
    pub status: SystemStatus,
    /// Per-component health details.
    pub components: Vec<ComponentHealth>,
    /// System uptime in seconds.
    pub uptime_seconds: u64,
}

/// Internal entry stored in the health registry.
struct HealthEntry {
    status: ComponentStatus,
    message: String,
    last_updated: Instant,
    /// Whether this component is critical (affects overall system health).
    critical: bool,
}

/// Registry tracking health of all system components.
///
/// Thread-safe via `DashMap`; components register and update their
/// status, and the registry produces aggregate health reports.
pub struct HealthRegistry {
    components: DashMap<String, HealthEntry>,
    start_time: Instant,
}

impl HealthRegistry {
    /// Creates a new empty health registry.
    pub fn new() -> Self {
        Self {
            components: DashMap::new(),
            start_time: Instant::now(),
        }
    }

    /// Registers a component as healthy.
    pub fn register(&self, name: &str, critical: bool) {
        self.components.insert(
            name.to_string(),
            HealthEntry {
                status: ComponentStatus::Healthy,
                message: "registered".to_string(),
                last_updated: Instant::now(),
                critical,
            },
        );
        debug!(component = name, critical, "registered health component");
    }

    /// Updates a component's health status.
    pub fn update(&self, name: &str, status: ComponentStatus, message: &str) {
        if let Some(mut entry) = self.components.get_mut(name) {
            let old_status = entry.status;
            entry.status = status;
            entry.message = message.to_string();
            entry.last_updated = Instant::now();

            if old_status != status {
                match status {
                    ComponentStatus::Unhealthy => {
                        warn!(component = name, %message, "component unhealthy");
                    }
                    ComponentStatus::Degraded => {
                        warn!(component = name, %message, "component degraded");
                    }
                    ComponentStatus::Healthy => {
                        info!(component = name, "component recovered");
                    }
                }
            }
        }
    }

    /// Removes a component from the registry.
    pub fn deregister(&self, name: &str) {
        self.components.remove(name);
    }

    /// Returns true if the system is ready (all critical components healthy or degraded).
    pub fn is_ready(&self) -> bool {
        self.components
            .iter()
            .all(|entry| !entry.critical || entry.status != ComponentStatus::Unhealthy)
    }

    /// Returns true if the system is alive (registry exists and has components).
    pub fn is_alive(&self) -> bool {
        !self.components.is_empty()
    }

    /// Produces a full health report.
    pub fn report(&self) -> HealthReport {
        let components: Vec<ComponentHealth> = self
            .components
            .iter()
            .map(|entry| ComponentHealth {
                name: entry.key().clone(),
                status: entry.status,
                message: entry.message.clone(),
                last_updated: entry.last_updated.elapsed().as_secs(),
            })
            .collect();

        let status = self.aggregate_status(&components);

        HealthReport {
            status,
            components,
            uptime_seconds: self.start_time.elapsed().as_secs(),
        }
    }

    /// Returns the number of registered components.
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    fn aggregate_status(&self, _components: &[ComponentHealth]) -> SystemStatus {
        let mut has_degraded = false;

        for entry in self.components.iter() {
            if entry.status == ComponentStatus::Unhealthy && entry.critical {
                return SystemStatus::Unhealthy;
            }
            if entry.status == ComponentStatus::Degraded
                || (entry.status == ComponentStatus::Unhealthy && !entry.critical)
            {
                has_degraded = true;
            }
        }

        if has_degraded {
            SystemStatus::Degraded
        } else {
            SystemStatus::Healthy
        }
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Circuit Breaker ─────────────────────────────────────────────────

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation; requests pass through.
    Closed,
    /// Circuit tripped; requests are rejected.
    Open,
    /// Testing recovery; limited requests allowed.
    HalfOpen,
}

/// Configuration for a circuit breaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before tripping.
    pub failure_threshold: u32,
    /// Duration the circuit stays open before transitioning to half-open.
    pub reset_timeout_secs: u64,
    /// Number of successes in half-open required to close the circuit.
    pub success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout_secs: 30,
            success_threshold: 2,
        }
    }
}

/// A thread-safe circuit breaker.
///
/// Tracks consecutive failures and transitions through Closed → Open →
/// HalfOpen → Closed states. Uses atomic counters for lock-free operation.
pub struct CircuitBreaker {
    name: String,
    config: CircuitBreakerConfig,
    /// Current consecutive failure count.
    failures: AtomicU32,
    /// Current consecutive success count (used in HalfOpen).
    successes: AtomicU32,
    /// Timestamp (as unix epoch secs) when the circuit was tripped open.
    tripped_at: AtomicU64,
    /// 0 = Closed, 1 = Open, 2 = HalfOpen.
    state: AtomicU32,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with the given name and config.
    pub fn new(name: impl Into<String>, config: CircuitBreakerConfig) -> Self {
        Self {
            name: name.into(),
            config,
            failures: AtomicU32::new(0),
            successes: AtomicU32::new(0),
            tripped_at: AtomicU64::new(0),
            state: AtomicU32::new(0), // Closed
        }
    }

    /// Creates a circuit breaker with default configuration.
    pub fn with_defaults(name: impl Into<String>) -> Self {
        Self::new(name, CircuitBreakerConfig::default())
    }

    /// Returns the circuit breaker name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current state.
    pub fn state(&self) -> CircuitState {
        let raw = self.state.load(Ordering::SeqCst);
        self.check_half_open_transition(raw)
    }

    /// Returns true if requests should be allowed through.
    pub fn is_allowed(&self) -> bool {
        match self.state() {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true, // limited requests in half-open
            CircuitState::Open => false,
        }
    }

    /// Records a successful operation.
    pub fn record_success(&self) {
        let state = self.state();

        match state {
            CircuitState::Closed => {
                // Reset failure count on success.
                self.failures.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let successes = self.successes.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    // Enough successes — close the circuit.
                    self.state.store(0, Ordering::SeqCst); // Closed
                    self.failures.store(0, Ordering::SeqCst);
                    self.successes.store(0, Ordering::SeqCst);
                    info!(
                        breaker = %self.name,
                        "circuit breaker closed (recovered)"
                    );
                }
            }
            CircuitState::Open => {} // shouldn't happen, but ignore
        }
    }

    /// Records a failed operation.
    pub fn record_failure(&self) {
        let state = self.state();

        match state {
            CircuitState::Closed => {
                let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
                if failures >= self.config.failure_threshold {
                    self.trip();
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open immediately re-opens.
                self.successes.store(0, Ordering::SeqCst);
                self.trip();
            }
            CircuitState::Open => {
                // Already open, just count.
                self.failures.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// Returns the current failure count.
    pub fn failure_count(&self) -> u32 {
        self.failures.load(Ordering::SeqCst)
    }

    /// Returns the current success count (meaningful in HalfOpen).
    pub fn success_count(&self) -> u32 {
        self.successes.load(Ordering::SeqCst)
    }

    /// Manually resets the circuit breaker to closed state.
    pub fn reset(&self) {
        self.state.store(0, Ordering::SeqCst);
        self.failures.store(0, Ordering::SeqCst);
        self.successes.store(0, Ordering::SeqCst);
        self.tripped_at.store(0, Ordering::SeqCst);
        info!(breaker = %self.name, "circuit breaker manually reset");
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Trips the circuit to open state.
    fn trip(&self) {
        self.state.store(1, Ordering::SeqCst); // Open
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        self.tripped_at.store(now, Ordering::SeqCst);
        warn!(
            breaker = %self.name,
            failures = self.failures.load(Ordering::SeqCst),
            "circuit breaker tripped open"
        );
    }

    /// Checks if an Open circuit should transition to HalfOpen.
    fn check_half_open_transition(&self, raw_state: u32) -> CircuitState {
        if raw_state != 1 {
            return match raw_state {
                0 => CircuitState::Closed,
                2 => CircuitState::HalfOpen,
                _ => CircuitState::Closed,
            };
        }

        // State is Open — check if reset timeout has elapsed.
        let tripped = self.tripped_at.load(Ordering::SeqCst);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        if now.saturating_sub(tripped) >= self.config.reset_timeout_secs {
            // Transition to HalfOpen.
            self.state.store(2, Ordering::SeqCst);
            self.successes.store(0, Ordering::SeqCst);
            debug!(
                breaker = %self.name,
                "circuit breaker transitioning to half-open"
            );
            CircuitState::HalfOpen
        } else {
            CircuitState::Open
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Health Registry Tests ───────────────────────────────────────

    #[test]
    fn registry_new_empty() {
        let registry = HealthRegistry::new();
        assert_eq!(registry.component_count(), 0);
        assert!(!registry.is_alive());
    }

    #[test]
    fn register_component() {
        let registry = HealthRegistry::new();
        registry.register("chain_connector", true);
        assert_eq!(registry.component_count(), 1);
        assert!(registry.is_alive());
        assert!(registry.is_ready());
    }

    #[test]
    fn update_component_status() {
        let registry = HealthRegistry::new();
        registry.register("db", true);

        registry.update("db", ComponentStatus::Degraded, "high latency");
        let report = registry.report();
        assert_eq!(report.status, SystemStatus::Degraded);

        registry.update("db", ComponentStatus::Healthy, "recovered");
        let report = registry.report();
        assert_eq!(report.status, SystemStatus::Healthy);
    }

    #[test]
    fn critical_unhealthy_makes_system_unhealthy() {
        let registry = HealthRegistry::new();
        registry.register("chain", true);
        registry.register("metrics", false);

        registry.update("chain", ComponentStatus::Unhealthy, "disconnected");
        assert!(!registry.is_ready());

        let report = registry.report();
        assert_eq!(report.status, SystemStatus::Unhealthy);
    }

    #[test]
    fn non_critical_unhealthy_makes_system_degraded() {
        let registry = HealthRegistry::new();
        registry.register("chain", true);
        registry.register("metrics", false);

        registry.update("metrics", ComponentStatus::Unhealthy, "exporter down");
        assert!(registry.is_ready()); // non-critical, still ready

        let report = registry.report();
        assert_eq!(report.status, SystemStatus::Degraded);
    }

    #[test]
    fn deregister_component() {
        let registry = HealthRegistry::new();
        registry.register("temp", false);
        assert_eq!(registry.component_count(), 1);

        registry.deregister("temp");
        assert_eq!(registry.component_count(), 0);
    }

    #[test]
    fn health_report_contains_all_components() {
        let registry = HealthRegistry::new();
        registry.register("a", true);
        registry.register("b", false);
        registry.register("c", true);

        let report = registry.report();
        assert_eq!(report.components.len(), 3);
    }

    #[test]
    fn health_report_uptime() {
        let registry = HealthRegistry::new();
        registry.register("test", false);
        let report = registry.report();
        // Should be 0 or very close to 0 seconds.
        assert!(report.uptime_seconds < 2);
    }

    #[test]
    fn component_status_serde() {
        for status in [
            ComponentStatus::Healthy,
            ComponentStatus::Degraded,
            ComponentStatus::Unhealthy,
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            let deserialized: ComponentStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn system_status_serde() {
        for status in [
            SystemStatus::Healthy,
            SystemStatus::Degraded,
            SystemStatus::Unhealthy,
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            let deserialized: SystemStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn health_report_serde_roundtrip() {
        let report = HealthReport {
            status: SystemStatus::Healthy,
            components: vec![ComponentHealth {
                name: "test".to_string(),
                status: ComponentStatus::Healthy,
                message: "ok".to_string(),
                last_updated: 0,
            }],
            uptime_seconds: 100,
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let deserialized: HealthReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.status, SystemStatus::Healthy);
        assert_eq!(deserialized.components.len(), 1);
    }

    #[test]
    fn default_registry() {
        let registry = HealthRegistry::default();
        assert_eq!(registry.component_count(), 0);
    }

    // ─── Circuit Breaker Tests ───────────────────────────────────────

    #[test]
    fn circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::with_defaults("test");
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.is_allowed());
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn circuit_trips_after_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout_secs: 60,
            success_threshold: 1,
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure(); // threshold reached
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.is_allowed());
    }

    #[test]
    fn success_resets_failure_count() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn circuit_transitions_to_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout_secs: 0, // immediate transition for testing
            success_threshold: 1,
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // trips to Open

        // With reset_timeout_secs=0, state() immediately sees the timeout
        // has elapsed and transitions to HalfOpen.
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(cb.is_allowed());
    }

    #[test]
    fn half_open_success_closes_circuit() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout_secs: 0,
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // Open
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen); // need 2

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn half_open_failure_reopens_circuit() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout_secs: 3600, // long timeout so Open stays Open
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // trips to Open
        assert_eq!(cb.state(), CircuitState::Open);

        // Manually transition to HalfOpen for testing.
        cb.state.store(2, Ordering::SeqCst); // HalfOpen
        cb.successes.store(0, Ordering::SeqCst);
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_failure(); // Should re-trip to Open
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn manual_reset() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout_secs: 3600,
            success_threshold: 1,
        };
        let cb = CircuitBreaker::new("test", config);

        cb.record_failure(); // Open
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.success_count(), 0);
    }

    #[test]
    fn circuit_breaker_config_default() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert_eq!(config.reset_timeout_secs, 30);
        assert_eq!(config.success_threshold, 2);
    }

    #[test]
    fn circuit_breaker_config_serde_roundtrip() {
        let config = CircuitBreakerConfig {
            failure_threshold: 10,
            reset_timeout_secs: 60,
            success_threshold: 3,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: CircuitBreakerConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.failure_threshold, 10);
        assert_eq!(deserialized.reset_timeout_secs, 60);
    }

    #[test]
    fn circuit_state_serde() {
        for state in [
            CircuitState::Closed,
            CircuitState::Open,
            CircuitState::HalfOpen,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let deserialized: CircuitState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, deserialized);
        }
    }

    #[test]
    fn circuit_breaker_name() {
        let cb = CircuitBreaker::with_defaults("rpc_ethereum");
        assert_eq!(cb.name(), "rpc_ethereum");
    }

    #[test]
    fn concurrent_circuit_breaker() {
        use std::sync::Arc;
        use std::thread;

        let cb = Arc::new(CircuitBreaker::new(
            "concurrent",
            CircuitBreakerConfig {
                failure_threshold: 100,
                reset_timeout_secs: 3600,
                success_threshold: 1,
            },
        ));

        let mut handles = vec![];
        for _ in 0..10 {
            let cb = Arc::clone(&cb);
            handles.push(thread::spawn(move || {
                for _ in 0..10 {
                    cb.record_failure();
                }
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }

        // 100 failures total, should trip (threshold = 100).
        assert!(cb.failure_count() >= 100);
    }
}
