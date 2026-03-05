//! Nonce management for concurrent transaction execution.
//!
//! Tracks per-address, per-chain nonces to prevent collisions when
//! multiple fill transactions are submitted simultaneously.

use alloy::primitives::Address;
use dashmap::DashMap;
use std::sync::Arc;

use phantom_chain::provider::DynProvider;
use phantom_common::error::{ExecutionError, ExecutionResult};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Composite key for nonce tracking: (address, chain_id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NonceKey {
    address: Address,
    chain_id: u64,
}

/// Configuration for the nonce manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceConfig {
    /// Maximum number of pending (unconfirmed) nonces per address before
    /// refusing to issue more. Prevents runaway nonce inflation.
    pub max_pending_per_address: u64,
}

impl Default for NonceConfig {
    fn default() -> Self {
        Self {
            max_pending_per_address: 16,
        }
    }
}

/// Manages transaction nonces across multiple addresses and chains.
///
/// Uses a [`DashMap`] for concurrent, per-shard locking. The `get_mut`
/// method holds a shard lock during read-increment, ensuring no two
/// callers receive the same nonce for the same address/chain pair.
pub struct NonceManager {
    nonces: DashMap<NonceKey, u64>,
    /// Tracks the on-chain confirmed nonce so we can compute pending count.
    confirmed: DashMap<NonceKey, u64>,
    config: NonceConfig,
}

impl NonceManager {
    /// Creates a new nonce manager with the given configuration.
    pub fn new(config: NonceConfig) -> Self {
        Self {
            nonces: DashMap::new(),
            confirmed: DashMap::new(),
            config,
        }
    }

    /// Creates a nonce manager with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(NonceConfig::default())
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &NonceConfig {
        &self.config
    }

    /// Returns the next nonce for the given address and chain.
    ///
    /// If the address/chain pair has not been seen before, fetches the
    /// current on-chain nonce via `eth_getTransactionCount` and
    /// initializes local tracking from that value.
    ///
    /// Each call atomically increments the tracked nonce, ensuring
    /// concurrent callers never receive the same value.
    pub async fn get_next_nonce(
        &self,
        address: &Address,
        chain_id: u64,
        provider: &Arc<DynProvider>,
    ) -> ExecutionResult<u64> {
        let key = NonceKey {
            address: *address,
            chain_id,
        };

        // Fast path: entry already exists — atomically read and increment.
        if let Some(mut entry) = self.nonces.get_mut(&key) {
            let nonce = *entry;

            // Check pending limit.
            let confirmed = self.confirmed.get(&key).map(|v| *v).unwrap_or(0);
            let pending = nonce.saturating_sub(confirmed);
            if pending >= self.config.max_pending_per_address {
                return Err(ExecutionError::NonceError(format!(
                    "pending nonce limit reached ({pending}/{}) for {address} on chain {chain_id}",
                    self.config.max_pending_per_address
                )));
            }

            *entry += 1;
            debug!(address = %address, chain_id, nonce, "assigned nonce (cached)");
            return Ok(nonce);
        }

        // Slow path: first time seeing this address/chain — sync from chain.
        let on_chain = self.fetch_nonce(address, provider).await?;

        // Use entry API to handle race: another task may have inserted while
        // we were fetching.
        let _initial = *self
            .nonces
            .entry(key)
            .and_modify(|existing| {
                // Another task initialized first — just use their value.
                debug!(
                    address = %address,
                    chain_id,
                    existing = *existing,
                    "nonce already initialized by concurrent task"
                );
            })
            .or_insert(on_chain);

        // Now atomically increment.
        if let Some(mut entry) = self.nonces.get_mut(&key) {
            let assigned = *entry;
            *entry += 1;

            // Store confirmed baseline.
            self.confirmed.entry(key).or_insert(on_chain);

            info!(
                address = %address,
                chain_id,
                nonce = assigned,
                on_chain_nonce = on_chain,
                "initialized and assigned nonce"
            );
            return Ok(assigned);
        }

        // Should never happen since we just inserted.
        Err(ExecutionError::NonceError(
            "unexpected nonce state after initialization".into(),
        ))
    }

    /// Forces a re-sync of the nonce from on-chain state.
    ///
    /// Overwrites the local nonce with the current `eth_getTransactionCount`.
    /// Use after detecting stuck transactions or nonce gaps.
    pub async fn sync_nonce(
        &self,
        address: &Address,
        chain_id: u64,
        provider: &Arc<DynProvider>,
    ) -> ExecutionResult<u64> {
        let on_chain = self.fetch_nonce(address, provider).await?;

        let key = NonceKey {
            address: *address,
            chain_id,
        };

        self.nonces.insert(key, on_chain);
        self.confirmed.insert(key, on_chain);

        info!(
            address = %address,
            chain_id,
            nonce = on_chain,
            "synced nonce from chain"
        );

        Ok(on_chain)
    }

    /// Returns the current local nonce without incrementing.
    ///
    /// Returns `None` if the address/chain pair has never been tracked.
    pub fn current_nonce(&self, address: &Address, chain_id: u64) -> Option<u64> {
        let key = NonceKey {
            address: *address,
            chain_id,
        };
        self.nonces.get(&key).map(|v| *v)
    }

    /// Returns the number of pending (unconfirmed) nonces for an address/chain.
    pub fn pending_count(&self, address: &Address, chain_id: u64) -> u64 {
        let key = NonceKey {
            address: *address,
            chain_id,
        };
        let current = self.nonces.get(&key).map(|v| *v).unwrap_or(0);
        let confirmed = self.confirmed.get(&key).map(|v| *v).unwrap_or(0);
        current.saturating_sub(confirmed)
    }

    /// Updates the confirmed nonce after a transaction is mined.
    ///
    /// This reduces the pending count and allows more nonces to be issued.
    pub fn confirm_nonce(&self, address: &Address, chain_id: u64, nonce: u64) {
        let key = NonceKey {
            address: *address,
            chain_id,
        };

        self.confirmed
            .entry(key)
            .and_modify(|confirmed| {
                // Only advance the confirmed nonce forward.
                if nonce + 1 > *confirmed {
                    *confirmed = nonce + 1;
                }
            })
            .or_insert(nonce + 1);

        debug!(
            address = %address,
            chain_id,
            confirmed_nonce = nonce,
            "confirmed nonce"
        );
    }

    /// Resets tracking for a specific address/chain pair.
    ///
    /// The next `get_next_nonce` call will re-fetch from chain.
    pub fn reset(&self, address: &Address, chain_id: u64) {
        let key = NonceKey {
            address: *address,
            chain_id,
        };
        self.nonces.remove(&key);
        self.confirmed.remove(&key);

        info!(address = %address, chain_id, "reset nonce tracking");
    }

    /// Returns the number of tracked address/chain pairs.
    pub fn tracked_count(&self) -> usize {
        self.nonces.len()
    }

    /// Returns true if no nonces are being tracked.
    pub fn is_empty(&self) -> bool {
        self.nonces.is_empty()
    }

    /// Fetches the on-chain transaction count for an address.
    async fn fetch_nonce(
        &self,
        address: &Address,
        provider: &Arc<DynProvider>,
    ) -> ExecutionResult<u64> {
        let count = provider
            .get_transaction_count(*address)
            .await
            .map_err(|e| ExecutionError::NonceError(format!("failed to fetch nonce: {e}")))?;

        Ok(count)
    }
}

impl Default for NonceManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const FILLER: Address = address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045");

    #[test]
    fn nonce_config_default() {
        let config = NonceConfig::default();
        assert_eq!(config.max_pending_per_address, 16);
    }

    #[test]
    fn nonce_config_serde_roundtrip() {
        let config = NonceConfig {
            max_pending_per_address: 32,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: NonceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.max_pending_per_address, 32);
    }

    #[test]
    fn default_manager_is_empty() {
        let manager = NonceManager::with_defaults();
        assert!(manager.is_empty());
        assert_eq!(manager.tracked_count(), 0);
    }

    #[test]
    fn current_nonce_untracked() {
        let manager = NonceManager::with_defaults();
        assert_eq!(manager.current_nonce(&FILLER, 1), None);
    }

    #[test]
    fn pending_count_untracked() {
        let manager = NonceManager::with_defaults();
        assert_eq!(manager.pending_count(&FILLER, 1), 0);
    }

    #[test]
    fn confirm_nonce_advances_confirmed() {
        let manager = NonceManager::with_defaults();
        let key = NonceKey {
            address: FILLER,
            chain_id: 1,
        };

        // Manually insert a tracked nonce.
        manager.nonces.insert(key, 5);
        manager.confirmed.insert(key, 0);

        assert_eq!(manager.pending_count(&FILLER, 1), 5);

        manager.confirm_nonce(&FILLER, 1, 2);
        // confirmed should now be 3 (nonce 2 + 1).
        assert_eq!(manager.pending_count(&FILLER, 1), 2); // 5 - 3

        // Confirming an older nonce should not move confirmed backward.
        manager.confirm_nonce(&FILLER, 1, 0);
        assert_eq!(manager.pending_count(&FILLER, 1), 2); // unchanged
    }

    #[test]
    fn reset_clears_tracking() {
        let manager = NonceManager::with_defaults();
        let key = NonceKey {
            address: FILLER,
            chain_id: 1,
        };
        manager.nonces.insert(key, 10);
        manager.confirmed.insert(key, 5);

        assert_eq!(manager.tracked_count(), 1);

        manager.reset(&FILLER, 1);

        assert_eq!(manager.tracked_count(), 0);
        assert_eq!(manager.current_nonce(&FILLER, 1), None);
    }

    #[test]
    fn separate_chains_tracked_independently() {
        let manager = NonceManager::with_defaults();
        let key1 = NonceKey {
            address: FILLER,
            chain_id: 1,
        };
        let key2 = NonceKey {
            address: FILLER,
            chain_id: 42161,
        };

        manager.nonces.insert(key1, 10);
        manager.nonces.insert(key2, 50);

        assert_eq!(manager.current_nonce(&FILLER, 1), Some(10));
        assert_eq!(manager.current_nonce(&FILLER, 42161), Some(50));
        assert_eq!(manager.tracked_count(), 2);
    }

    #[tokio::test]
    async fn get_next_nonce_with_anvil() {
        let anvil = match alloy::node_bindings::Anvil::new().try_spawn() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Anvil not available, skipping");
                return;
            }
        };

        let provider: Arc<DynProvider> = Arc::new(
            alloy::providers::ProviderBuilder::new()
                .connect_http(anvil.endpoint().parse().expect("valid url")),
        );

        let manager = NonceManager::with_defaults();
        let address = anvil.addresses()[0];

        // First call should sync from chain (nonce 0 for fresh account).
        let nonce = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("get nonce");
        assert_eq!(nonce, 0);
        assert_eq!(manager.current_nonce(&address, 1), Some(1)); // incremented

        // Second call should use cached value.
        let nonce2 = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("get nonce 2");
        assert_eq!(nonce2, 1);
        assert_eq!(manager.current_nonce(&address, 1), Some(2));
    }

    #[tokio::test]
    async fn sync_nonce_resets_to_chain() {
        let anvil = match alloy::node_bindings::Anvil::new().try_spawn() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Anvil not available, skipping");
                return;
            }
        };

        let provider: Arc<DynProvider> = Arc::new(
            alloy::providers::ProviderBuilder::new()
                .connect_http(anvil.endpoint().parse().expect("valid url")),
        );

        let manager = NonceManager::with_defaults();
        let address = anvil.addresses()[0];

        // Advance local nonce.
        let _ = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("nonce");
        let _ = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("nonce");
        assert_eq!(manager.current_nonce(&address, 1), Some(2));

        // Sync should reset to on-chain value (still 0, no txs sent).
        let synced = manager
            .sync_nonce(&address, 1, &provider)
            .await
            .expect("sync");
        assert_eq!(synced, 0);
        assert_eq!(manager.current_nonce(&address, 1), Some(0));
    }

    #[tokio::test]
    async fn pending_limit_enforced() {
        let anvil = match alloy::node_bindings::Anvil::new().try_spawn() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Anvil not available, skipping");
                return;
            }
        };

        let provider: Arc<DynProvider> = Arc::new(
            alloy::providers::ProviderBuilder::new()
                .connect_http(anvil.endpoint().parse().expect("valid url")),
        );

        let config = NonceConfig {
            max_pending_per_address: 3,
        };
        let manager = NonceManager::new(config);
        let address = anvil.addresses()[0];

        // Should succeed for first 3.
        for i in 0..3 {
            let nonce = manager
                .get_next_nonce(&address, 1, &provider)
                .await
                .expect("get nonce");
            assert_eq!(nonce, i);
        }

        // 4th should fail.
        let result = manager.get_next_nonce(&address, 1, &provider).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("pending nonce limit"));
    }

    #[tokio::test]
    async fn confirm_frees_pending_slots() {
        let anvil = match alloy::node_bindings::Anvil::new().try_spawn() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Anvil not available, skipping");
                return;
            }
        };

        let provider: Arc<DynProvider> = Arc::new(
            alloy::providers::ProviderBuilder::new()
                .connect_http(anvil.endpoint().parse().expect("valid url")),
        );

        let config = NonceConfig {
            max_pending_per_address: 2,
        };
        let manager = NonceManager::new(config);
        let address = anvil.addresses()[0];

        // Use up both slots.
        let n0 = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("nonce 0");
        let _n1 = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("nonce 1");

        // Should fail — at limit.
        assert!(manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .is_err());

        // Confirm nonce 0 — frees a slot.
        manager.confirm_nonce(&address, 1, n0);

        // Should succeed now.
        let n2 = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("nonce 2");
        assert_eq!(n2, 2);
    }

    #[tokio::test]
    async fn concurrent_nonces_are_unique() {
        let anvil = match alloy::node_bindings::Anvil::new().try_spawn() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("Anvil not available, skipping");
                return;
            }
        };

        let provider: Arc<DynProvider> = Arc::new(
            alloy::providers::ProviderBuilder::new()
                .connect_http(anvil.endpoint().parse().expect("valid url")),
        );

        let manager = Arc::new(NonceManager::with_defaults());
        let address = anvil.addresses()[0];

        // Initialize nonce first.
        let _ = manager
            .get_next_nonce(&address, 1, &provider)
            .await
            .expect("init");

        // Spawn multiple concurrent nonce requests.
        let mut handles = Vec::new();
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            let prov = Arc::clone(&provider);
            let addr = address;
            handles.push(tokio::spawn(async move {
                mgr.get_next_nonce(&addr, 1, &prov).await.expect("nonce")
            }));
        }

        let mut nonces = Vec::new();
        for handle in handles {
            nonces.push(handle.await.expect("task"));
        }

        // All nonces should be unique.
        nonces.sort();
        nonces.dedup();
        assert_eq!(nonces.len(), 10, "all nonces must be unique");
    }
}
