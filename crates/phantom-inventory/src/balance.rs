//! Balance tracking across multiple chains and wallets.
//!
//! Tracks native (ETH) and ERC-20 token balances per address per chain,
//! with staleness detection and on-chain refresh capability.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::{Address, U256};
use dashmap::DashMap;
use phantom_chain::provider::DynProvider;
use phantom_common::error::{InventoryError, InventoryResult};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Sentinel address representing the native token (ETH/MATIC/etc.).
pub const NATIVE_TOKEN: Address = Address::ZERO;

/// Composite key for balance lookups: (wallet, chain, token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BalanceKey {
    address: Address,
    chain_id: u64,
    token: Address,
}

/// Internal balance record with metadata.
#[derive(Debug, Clone)]
struct BalanceEntry {
    amount: U256,
    updated_at: u64,
}

/// Configuration for the balance tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceConfig {
    /// Duration in seconds after which a balance is considered stale.
    pub stale_threshold_secs: u64,
    /// Maximum number of tracked balance entries.
    pub max_entries: usize,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            stale_threshold_secs: 30,
            max_entries: 10_000,
        }
    }
}

/// A single token balance for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    /// Wallet address.
    pub address: Address,
    /// Chain ID.
    pub chain_id: u64,
    /// Token contract address (`Address::ZERO` for native token).
    pub token: Address,
    /// Balance amount in the token's smallest unit.
    pub amount: U256,
    /// Unix timestamp of last update.
    pub updated_at: u64,
    /// Whether this balance is considered stale.
    pub is_stale: bool,
}

/// Summary of balances for a specific chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainBalanceSummary {
    /// Chain ID.
    pub chain_id: u64,
    /// Number of tracked balances on this chain.
    pub entry_count: usize,
    /// Number of unique wallet addresses on this chain.
    pub wallet_count: usize,
    /// Number of stale entries.
    pub stale_count: usize,
}

/// Tracks token and native balances across multiple chains and wallets.
///
/// Uses [`DashMap`] for concurrent access. Balances are updated either
/// manually via `update_balance` or by fetching from chain via
/// `refresh_native_balance`.
pub struct BalanceTracker {
    balances: DashMap<BalanceKey, BalanceEntry>,
    config: BalanceConfig,
}

impl BalanceTracker {
    /// Creates a new balance tracker with the given configuration.
    pub fn new(config: BalanceConfig) -> Self {
        Self {
            balances: DashMap::new(),
            config,
        }
    }

    /// Creates a balance tracker with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(BalanceConfig::default())
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &BalanceConfig {
        &self.config
    }

    /// Updates (or inserts) a balance for the given wallet/chain/token.
    pub fn update_balance(
        &self,
        address: &Address,
        chain_id: u64,
        token: &Address,
        amount: U256,
    ) -> InventoryResult<()> {
        if self.balances.len() >= self.config.max_entries
            && !self.balances.contains_key(&BalanceKey {
                address: *address,
                chain_id,
                token: *token,
            })
        {
            return Err(InventoryError::BalanceCheckFailed(format!(
                "balance entry limit reached (max {})",
                self.config.max_entries
            )));
        }

        let key = BalanceKey {
            address: *address,
            chain_id,
            token: *token,
        };

        let now = current_timestamp();

        self.balances.insert(
            key,
            BalanceEntry {
                amount,
                updated_at: now,
            },
        );

        debug!(
            address = %address,
            chain_id,
            token = %token,
            amount = %amount,
            "updated balance"
        );

        Ok(())
    }

    /// Returns the balance for a specific wallet/chain/token, or `None`
    /// if not tracked.
    pub fn get_balance(&self, address: &Address, chain_id: u64, token: &Address) -> Option<U256> {
        let key = BalanceKey {
            address: *address,
            chain_id,
            token: *token,
        };
        self.balances.get(&key).map(|entry| entry.amount)
    }

    /// Returns all tracked balances for a given wallet on a specific chain.
    pub fn get_wallet_balances(&self, address: &Address, chain_id: u64) -> Vec<TokenBalance> {
        let now = current_timestamp();
        let stale_threshold = self.config.stale_threshold_secs;

        self.balances
            .iter()
            .filter(|entry| {
                let k = entry.key();
                k.address == *address && k.chain_id == chain_id
            })
            .map(|entry| {
                let k = entry.key();
                let v = entry.value();
                TokenBalance {
                    address: k.address,
                    chain_id: k.chain_id,
                    token: k.token,
                    amount: v.amount,
                    updated_at: v.updated_at,
                    is_stale: now.saturating_sub(v.updated_at) > stale_threshold,
                }
            })
            .collect()
    }

    /// Returns true if the balance for the given key is stale (older than
    /// the configured threshold).
    pub fn is_stale(&self, address: &Address, chain_id: u64, token: &Address) -> bool {
        let key = BalanceKey {
            address: *address,
            chain_id,
            token: *token,
        };

        match self.balances.get(&key) {
            Some(entry) => {
                let now = current_timestamp();
                now.saturating_sub(entry.updated_at) > self.config.stale_threshold_secs
            }
            None => true, // no entry = stale
        }
    }

    /// Fetches and updates the native token balance from chain.
    pub async fn refresh_native_balance(
        &self,
        address: &Address,
        chain_id: u64,
        provider: &Arc<DynProvider>,
    ) -> InventoryResult<U256> {
        let balance = provider.get_balance(*address).await.map_err(|e| {
            InventoryError::BalanceCheckFailed(format!("failed to fetch balance: {e}"))
        })?;

        self.update_balance(address, chain_id, &NATIVE_TOKEN, balance)?;

        info!(
            address = %address,
            chain_id,
            balance = %balance,
            "refreshed native balance"
        );

        Ok(balance)
    }

    /// Removes all balance entries for a specific wallet on a chain.
    pub fn clear_wallet(&self, address: &Address, chain_id: u64) -> usize {
        let keys_to_remove: Vec<BalanceKey> = self
            .balances
            .iter()
            .filter(|entry| {
                let k = entry.key();
                k.address == *address && k.chain_id == chain_id
            })
            .map(|entry| *entry.key())
            .collect();

        let count = keys_to_remove.len();
        for key in keys_to_remove {
            self.balances.remove(&key);
        }

        if count > 0 {
            info!(
                address = %address,
                chain_id,
                removed = count,
                "cleared wallet balances"
            );
        }

        count
    }

    /// Returns a summary of balances for a specific chain.
    pub fn chain_summary(&self, chain_id: u64) -> ChainBalanceSummary {
        let now = current_timestamp();
        let stale_threshold = self.config.stale_threshold_secs;

        let mut entry_count = 0;
        let mut stale_count = 0;
        let mut wallets = std::collections::HashSet::new();

        for entry in self.balances.iter() {
            let k = entry.key();
            if k.chain_id == chain_id {
                entry_count += 1;
                wallets.insert(k.address);
                if now.saturating_sub(entry.value().updated_at) > stale_threshold {
                    stale_count += 1;
                }
            }
        }

        ChainBalanceSummary {
            chain_id,
            entry_count,
            wallet_count: wallets.len(),
            stale_count,
        }
    }

    /// Returns the total number of tracked balance entries.
    pub fn entry_count(&self) -> usize {
        self.balances.len()
    }

    /// Returns true if no balances are tracked.
    pub fn is_empty(&self) -> bool {
        self.balances.is_empty()
    }

    /// Removes all tracked balances.
    pub fn clear_all(&self) {
        self.balances.clear();
        info!("cleared all balance entries");
    }

    /// Checks whether a wallet has sufficient balance of a token.
    pub fn has_sufficient_balance(
        &self,
        address: &Address,
        chain_id: u64,
        token: &Address,
        required: U256,
    ) -> bool {
        self.get_balance(address, chain_id, token)
            .map(|balance| balance >= required)
            .unwrap_or(false)
    }
}

impl Default for BalanceTracker {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Returns the current unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const WALLET: Address = address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
    const WALLET_2: Address = address!("0x6000da47483062A0D734Ba3dc7576Ce6A0B645C4");
    const USDC: Address = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    const WETH: Address = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

    #[test]
    fn balance_config_default() {
        let config = BalanceConfig::default();
        assert_eq!(config.stale_threshold_secs, 30);
        assert_eq!(config.max_entries, 10_000);
    }

    #[test]
    fn balance_config_serde_roundtrip() {
        let config = BalanceConfig {
            stale_threshold_secs: 60,
            max_entries: 5_000,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: BalanceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.stale_threshold_secs, 60);
        assert_eq!(deserialized.max_entries, 5_000);
    }

    #[test]
    fn default_tracker_is_empty() {
        let tracker = BalanceTracker::with_defaults();
        assert!(tracker.is_empty());
        assert_eq!(tracker.entry_count(), 0);
    }

    #[test]
    fn update_and_get_balance() {
        let tracker = BalanceTracker::with_defaults();
        let amount = U256::from(1_000_000u64); // 1 USDC

        tracker
            .update_balance(&WALLET, 1, &USDC, amount)
            .expect("update");

        assert_eq!(tracker.get_balance(&WALLET, 1, &USDC), Some(amount));
        assert_eq!(tracker.entry_count(), 1);
    }

    #[test]
    fn get_nonexistent_balance() {
        let tracker = BalanceTracker::with_defaults();
        assert_eq!(tracker.get_balance(&WALLET, 1, &USDC), None);
    }

    #[test]
    fn update_overwrites_existing() {
        let tracker = BalanceTracker::with_defaults();

        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(200u64))
            .expect("update");

        assert_eq!(
            tracker.get_balance(&WALLET, 1, &USDC),
            Some(U256::from(200u64))
        );
        assert_eq!(tracker.entry_count(), 1); // not duplicated
    }

    #[test]
    fn native_token_balance() {
        let tracker = BalanceTracker::with_defaults();
        let eth_balance = U256::from(1_000_000_000_000_000_000u64); // 1 ETH

        tracker
            .update_balance(&WALLET, 1, &NATIVE_TOKEN, eth_balance)
            .expect("update");

        assert_eq!(
            tracker.get_balance(&WALLET, 1, &NATIVE_TOKEN),
            Some(eth_balance)
        );
    }

    #[test]
    fn separate_chains_tracked_independently() {
        let tracker = BalanceTracker::with_defaults();

        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 42161, &USDC, U256::from(200u64))
            .expect("update");

        assert_eq!(
            tracker.get_balance(&WALLET, 1, &USDC),
            Some(U256::from(100u64))
        );
        assert_eq!(
            tracker.get_balance(&WALLET, 42161, &USDC),
            Some(U256::from(200u64))
        );
        assert_eq!(tracker.entry_count(), 2);
    }

    #[test]
    fn separate_tokens_tracked_independently() {
        let tracker = BalanceTracker::with_defaults();

        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &WETH, U256::from(200u64))
            .expect("update");

        assert_eq!(
            tracker.get_balance(&WALLET, 1, &USDC),
            Some(U256::from(100u64))
        );
        assert_eq!(
            tracker.get_balance(&WALLET, 1, &WETH),
            Some(U256::from(200u64))
        );
    }

    #[test]
    fn max_entries_enforced() {
        let config = BalanceConfig {
            max_entries: 2,
            ..BalanceConfig::default()
        };
        let tracker = BalanceTracker::new(config);

        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &WETH, U256::from(200u64))
            .expect("update");

        // Third entry should fail.
        let result = tracker.update_balance(&WALLET, 42161, &USDC, U256::from(300u64));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("limit reached"));
    }

    #[test]
    fn max_entries_allows_update_of_existing() {
        let config = BalanceConfig {
            max_entries: 1,
            ..BalanceConfig::default()
        };
        let tracker = BalanceTracker::new(config);

        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("first insert");

        // Updating existing entry should succeed even at limit.
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(200u64))
            .expect("update existing");

        assert_eq!(
            tracker.get_balance(&WALLET, 1, &USDC),
            Some(U256::from(200u64))
        );
    }

    #[test]
    fn is_stale_no_entry() {
        let tracker = BalanceTracker::with_defaults();
        assert!(tracker.is_stale(&WALLET, 1, &USDC));
    }

    #[test]
    fn is_stale_fresh_entry() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");

        // Just inserted — should not be stale.
        assert!(!tracker.is_stale(&WALLET, 1, &USDC));
    }

    #[test]
    fn has_sufficient_balance_true() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(1000u64))
            .expect("update");

        assert!(tracker.has_sufficient_balance(&WALLET, 1, &USDC, U256::from(500u64)));
        assert!(tracker.has_sufficient_balance(&WALLET, 1, &USDC, U256::from(1000u64)));
    }

    #[test]
    fn has_sufficient_balance_false() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");

        assert!(!tracker.has_sufficient_balance(&WALLET, 1, &USDC, U256::from(200u64)));
    }

    #[test]
    fn has_sufficient_balance_no_entry() {
        let tracker = BalanceTracker::with_defaults();
        assert!(!tracker.has_sufficient_balance(&WALLET, 1, &USDC, U256::from(1u64)));
    }

    #[test]
    fn get_wallet_balances() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &WETH, U256::from(200u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 42161, &USDC, U256::from(300u64))
            .expect("update");

        let balances = tracker.get_wallet_balances(&WALLET, 1);
        assert_eq!(balances.len(), 2);

        let tokens: Vec<Address> = balances.iter().map(|b| b.token).collect();
        assert!(tokens.contains(&USDC));
        assert!(tokens.contains(&WETH));
    }

    #[test]
    fn clear_wallet() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &WETH, U256::from(200u64))
            .expect("update");
        tracker
            .update_balance(&WALLET_2, 1, &USDC, U256::from(300u64))
            .expect("update");

        let removed = tracker.clear_wallet(&WALLET, 1);
        assert_eq!(removed, 2);
        assert_eq!(tracker.entry_count(), 1); // WALLET_2 remains
        assert!(tracker.get_balance(&WALLET, 1, &USDC).is_none());
    }

    #[test]
    fn clear_all() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET_2, 42161, &WETH, U256::from(200u64))
            .expect("update");

        tracker.clear_all();
        assert!(tracker.is_empty());
    }

    #[test]
    fn chain_summary() {
        let tracker = BalanceTracker::with_defaults();
        tracker
            .update_balance(&WALLET, 1, &USDC, U256::from(100u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 1, &WETH, U256::from(200u64))
            .expect("update");
        tracker
            .update_balance(&WALLET_2, 1, &USDC, U256::from(300u64))
            .expect("update");
        tracker
            .update_balance(&WALLET, 42161, &USDC, U256::from(400u64))
            .expect("update");

        let summary = tracker.chain_summary(1);
        assert_eq!(summary.chain_id, 1);
        assert_eq!(summary.entry_count, 3);
        assert_eq!(summary.wallet_count, 2);
        assert_eq!(summary.stale_count, 0); // all fresh
    }

    #[test]
    fn chain_summary_empty_chain() {
        let tracker = BalanceTracker::with_defaults();
        let summary = tracker.chain_summary(999);
        assert_eq!(summary.entry_count, 0);
        assert_eq!(summary.wallet_count, 0);
    }

    #[test]
    fn token_balance_serde_roundtrip() {
        let tb = TokenBalance {
            address: WALLET,
            chain_id: 1,
            token: USDC,
            amount: U256::from(1_000_000u64),
            updated_at: 1700000000,
            is_stale: false,
        };
        let json = serde_json::to_string(&tb).expect("serialize");
        let deserialized: TokenBalance = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.chain_id, 1);
        assert_eq!(deserialized.amount, U256::from(1_000_000u64));
        assert!(!deserialized.is_stale);
    }

    #[test]
    fn chain_balance_summary_serde_roundtrip() {
        let summary = ChainBalanceSummary {
            chain_id: 42161,
            entry_count: 5,
            wallet_count: 2,
            stale_count: 1,
        };
        let json = serde_json::to_string(&summary).expect("serialize");
        let deserialized: ChainBalanceSummary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.chain_id, 42161);
        assert_eq!(deserialized.stale_count, 1);
    }

    #[tokio::test]
    async fn refresh_native_balance_with_anvil() {
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

        let tracker = BalanceTracker::with_defaults();
        let address = anvil.addresses()[0];

        let balance = tracker
            .refresh_native_balance(&address, 1, &provider)
            .await
            .expect("refresh");

        // Anvil default accounts have 10000 ETH.
        assert!(balance > U256::ZERO);
        assert_eq!(
            tracker.get_balance(&address, 1, &NATIVE_TOKEN),
            Some(balance)
        );
        assert!(!tracker.is_stale(&address, 1, &NATIVE_TOKEN));
    }
}
