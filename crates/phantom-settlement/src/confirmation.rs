//! Settlement confirmation monitoring for fill transactions.
//!
//! Tracks submitted transactions through confirmation stages and
//! detects reverts or dropped transactions.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::{Address, B256};
use dashmap::DashMap;
use phantom_chain::provider::DynProvider;
use phantom_common::error::{SettlementError, SettlementResult};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Configuration for the confirmation monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationConfig {
    /// Number of block confirmations required for finality.
    pub required_confirmations: u64,
    /// Timeout in milliseconds before a transaction is considered dropped.
    pub timeout_ms: u64,
    /// Maximum number of transactions to track simultaneously.
    pub max_tracked: usize,
}

impl Default for ConfirmationConfig {
    fn default() -> Self {
        Self {
            required_confirmations: 2,
            timeout_ms: 120_000, // 2 minutes
            max_tracked: 1_000,
        }
    }
}

/// Status of a tracked transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxStatus {
    /// Transaction submitted but not yet seen in a block.
    Pending,
    /// Transaction included in a block but below confirmation threshold.
    Included,
    /// Transaction reached required confirmations.
    Confirmed,
    /// Transaction reverted on chain.
    Reverted,
    /// Transaction not included within the timeout window.
    Dropped,
}

/// Record of a tracked transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxRecord {
    /// Transaction hash.
    pub tx_hash: B256,
    /// Chain ID where the transaction was submitted.
    pub chain_id: u64,
    /// Sender address.
    pub from: Address,
    /// Current status.
    pub status: TxStatus,
    /// Block number where the tx was included (if any).
    pub block_number: Option<u64>,
    /// Number of confirmations received.
    pub confirmations: u64,
    /// Unix timestamp when the transaction was submitted.
    pub submitted_at: u64,
    /// Unix timestamp of the last status update.
    pub updated_at: u64,
    /// Gas used (populated after inclusion).
    pub gas_used: Option<u64>,
    /// Whether the on-chain execution succeeded.
    pub execution_success: Option<bool>,
}

impl TxRecord {
    /// Returns true if the transaction has reached finality.
    pub fn is_final(&self) -> bool {
        matches!(
            self.status,
            TxStatus::Confirmed | TxStatus::Reverted | TxStatus::Dropped
        )
    }

    /// Returns true if the transaction is still pending confirmation.
    pub fn is_pending(&self) -> bool {
        matches!(self.status, TxStatus::Pending | TxStatus::Included)
    }
}

/// Monitors fill transactions through confirmation stages.
///
/// Tracks transaction lifecycle from submission through inclusion,
/// confirmation, or timeout/revert detection.
pub struct ConfirmationMonitor {
    records: DashMap<B256, TxRecord>,
    config: ConfirmationConfig,
}

impl ConfirmationMonitor {
    /// Creates a new confirmation monitor.
    pub fn new(config: ConfirmationConfig) -> Self {
        Self {
            records: DashMap::new(),
            config,
        }
    }

    /// Creates a monitor with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ConfirmationConfig::default())
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &ConfirmationConfig {
        &self.config
    }

    /// Registers a newly submitted transaction for tracking.
    pub fn track_transaction(
        &self,
        tx_hash: B256,
        chain_id: u64,
        from: Address,
    ) -> SettlementResult<()> {
        if self.records.len() >= self.config.max_tracked {
            return Err(SettlementError::ConfirmationFailed(format!(
                "tracking limit reached (max {})",
                self.config.max_tracked
            )));
        }

        let now = current_timestamp();

        self.records.insert(
            tx_hash,
            TxRecord {
                tx_hash,
                chain_id,
                from,
                status: TxStatus::Pending,
                block_number: None,
                confirmations: 0,
                submitted_at: now,
                updated_at: now,
                gas_used: None,
                execution_success: None,
            },
        );

        info!(
            tx_hash = %tx_hash,
            chain_id,
            from = %from,
            "tracking transaction"
        );

        Ok(())
    }

    /// Checks the on-chain status of a tracked transaction.
    ///
    /// Queries the provider for the transaction receipt and current
    /// block number to determine confirmation count.
    pub async fn check_status(
        &self,
        tx_hash: &B256,
        provider: &Arc<DynProvider>,
    ) -> SettlementResult<TxRecord> {
        let mut record = self.records.get_mut(tx_hash).ok_or_else(|| {
            SettlementError::ConfirmationFailed(format!("transaction not tracked: {tx_hash}"))
        })?;

        // Already final — return current state.
        if record.is_final() {
            return Ok(record.clone());
        }

        let now = current_timestamp();

        // Check for timeout.
        let elapsed_ms = now.saturating_sub(record.submitted_at) * 1000;
        if elapsed_ms > self.config.timeout_ms {
            record.status = TxStatus::Dropped;
            record.updated_at = now;
            warn!(
                tx_hash = %tx_hash,
                elapsed_ms,
                "transaction timed out"
            );
            return Ok(record.clone());
        }

        // Fetch receipt.
        let receipt = provider
            .get_transaction_receipt(*tx_hash)
            .await
            .map_err(|e| {
                SettlementError::ConfirmationFailed(format!("receipt fetch failed: {e}"))
            })?;

        let receipt = match receipt {
            Some(r) => r,
            None => {
                // Still pending — no receipt yet.
                debug!(tx_hash = %tx_hash, "transaction still pending");
                record.updated_at = now;
                return Ok(record.clone());
            }
        };

        // Transaction is included — update record.
        let tx_block = receipt.block_number.unwrap_or(0);
        let success = receipt.status();
        let gas = receipt.gas_used;

        record.block_number = Some(tx_block);
        record.gas_used = Some(gas);
        record.execution_success = Some(success);
        record.updated_at = now;

        if !success {
            record.status = TxStatus::Reverted;
            warn!(
                tx_hash = %tx_hash,
                block = tx_block,
                gas_used = gas,
                "transaction reverted"
            );
            return Ok(record.clone());
        }

        // Check confirmations.
        let current_block = provider
            .get_block_number()
            .await
            .map_err(|e| SettlementError::ConfirmationFailed(format!("block number fetch: {e}")))?;

        let confirmations = current_block.saturating_sub(tx_block);
        record.confirmations = confirmations;

        if confirmations >= self.config.required_confirmations {
            record.status = TxStatus::Confirmed;
            info!(
                tx_hash = %tx_hash,
                block = tx_block,
                confirmations,
                gas_used = gas,
                "transaction confirmed"
            );
        } else {
            record.status = TxStatus::Included;
            debug!(
                tx_hash = %tx_hash,
                block = tx_block,
                confirmations,
                required = self.config.required_confirmations,
                "transaction included, awaiting confirmations"
            );
        }

        Ok(record.clone())
    }

    /// Returns the current record for a transaction, if tracked.
    pub fn get_record(&self, tx_hash: &B256) -> Option<TxRecord> {
        self.records.get(tx_hash).map(|r| r.clone())
    }

    /// Returns all pending (non-final) transaction records.
    pub fn pending_transactions(&self) -> Vec<TxRecord> {
        self.records
            .iter()
            .filter(|entry| entry.value().is_pending())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Returns all final (confirmed, reverted, dropped) records.
    pub fn final_transactions(&self) -> Vec<TxRecord> {
        self.records
            .iter()
            .filter(|entry| entry.value().is_final())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Removes a finalized transaction from tracking.
    ///
    /// Returns the record if it was removed, `None` if not found
    /// or still pending.
    pub fn remove_finalized(&self, tx_hash: &B256) -> Option<TxRecord> {
        if let Some(entry) = self.records.get(tx_hash) {
            if entry.is_final() {
                drop(entry);
                return self.records.remove(tx_hash).map(|(_, record)| record);
            }
        }
        None
    }

    /// Removes all finalized transactions. Returns the count removed.
    pub fn prune_finalized(&self) -> usize {
        let to_remove: Vec<B256> = self
            .records
            .iter()
            .filter(|entry| entry.value().is_final())
            .map(|entry| *entry.key())
            .collect();

        let count = to_remove.len();
        for hash in to_remove {
            self.records.remove(&hash);
        }

        if count > 0 {
            info!(removed = count, "pruned finalized transactions");
        }

        count
    }

    /// Returns the total number of tracked transactions.
    pub fn tracked_count(&self) -> usize {
        self.records.len()
    }

    /// Returns the number of pending transactions.
    pub fn pending_count(&self) -> usize {
        self.records
            .iter()
            .filter(|entry| entry.value().is_pending())
            .count()
    }

    /// Returns true if no transactions are tracked.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for ConfirmationMonitor {
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
    use alloy::primitives::{address, b256};

    const TX_HASH: B256 =
        b256!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    const TX_HASH_2: B256 =
        b256!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    const FILLER: Address = address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045");

    #[test]
    fn confirmation_config_default() {
        let config = ConfirmationConfig::default();
        assert_eq!(config.required_confirmations, 2);
        assert_eq!(config.timeout_ms, 120_000);
        assert_eq!(config.max_tracked, 1_000);
    }

    #[test]
    fn confirmation_config_serde_roundtrip() {
        let config = ConfirmationConfig {
            required_confirmations: 5,
            timeout_ms: 60_000,
            max_tracked: 500,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ConfirmationConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.required_confirmations, 5);
        assert_eq!(deserialized.max_tracked, 500);
    }

    #[test]
    fn tx_status_serde() {
        let statuses = [
            TxStatus::Pending,
            TxStatus::Included,
            TxStatus::Confirmed,
            TxStatus::Reverted,
            TxStatus::Dropped,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).expect("serialize");
            let deserialized: TxStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*status, deserialized);
        }
    }

    #[test]
    fn tx_record_is_final() {
        let mut record = make_record(TxStatus::Pending);
        assert!(!record.is_final());
        assert!(record.is_pending());

        record.status = TxStatus::Included;
        assert!(!record.is_final());
        assert!(record.is_pending());

        record.status = TxStatus::Confirmed;
        assert!(record.is_final());
        assert!(!record.is_pending());

        record.status = TxStatus::Reverted;
        assert!(record.is_final());

        record.status = TxStatus::Dropped;
        assert!(record.is_final());
    }

    #[test]
    fn track_transaction() {
        let monitor = ConfirmationMonitor::with_defaults();
        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("track");

        assert_eq!(monitor.tracked_count(), 1);
        assert_eq!(monitor.pending_count(), 1);
        assert!(!monitor.is_empty());

        let record = monitor.get_record(&TX_HASH).expect("record exists");
        assert_eq!(record.status, TxStatus::Pending);
        assert_eq!(record.chain_id, 1);
        assert_eq!(record.from, FILLER);
        assert!(record.block_number.is_none());
    }

    #[test]
    fn track_limit_enforced() {
        let config = ConfirmationConfig {
            max_tracked: 1,
            ..ConfirmationConfig::default()
        };
        let monitor = ConfirmationMonitor::new(config);

        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("first");

        let result = monitor.track_transaction(TX_HASH_2, 1, FILLER);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tracking limit"));
    }

    #[test]
    fn pending_transactions() {
        let monitor = ConfirmationMonitor::with_defaults();
        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("track");
        monitor
            .track_transaction(TX_HASH_2, 1, FILLER)
            .expect("track");

        let pending = monitor.pending_transactions();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn remove_finalized() {
        let monitor = ConfirmationMonitor::with_defaults();
        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("track");

        // Cannot remove while pending.
        assert!(monitor.remove_finalized(&TX_HASH).is_none());

        // Mark as confirmed manually.
        if let Some(mut record) = monitor.records.get_mut(&TX_HASH) {
            record.status = TxStatus::Confirmed;
        }

        let removed = monitor.remove_finalized(&TX_HASH);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().status, TxStatus::Confirmed);
        assert!(monitor.is_empty());
    }

    #[test]
    fn prune_finalized() {
        let monitor = ConfirmationMonitor::with_defaults();
        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("track");
        monitor
            .track_transaction(TX_HASH_2, 1, FILLER)
            .expect("track");

        // Mark first as confirmed.
        if let Some(mut record) = monitor.records.get_mut(&TX_HASH) {
            record.status = TxStatus::Confirmed;
        }

        let pruned = monitor.prune_finalized();
        assert_eq!(pruned, 1);
        assert_eq!(monitor.tracked_count(), 1); // TX_HASH_2 remains
    }

    #[test]
    fn get_nonexistent_record() {
        let monitor = ConfirmationMonitor::with_defaults();
        assert!(monitor.get_record(&TX_HASH).is_none());
    }

    #[test]
    fn tx_record_serde_roundtrip() {
        let record = TxRecord {
            tx_hash: TX_HASH,
            chain_id: 1,
            from: FILLER,
            status: TxStatus::Confirmed,
            block_number: Some(19_000_000),
            confirmations: 5,
            submitted_at: 1700000000,
            updated_at: 1700000060,
            gas_used: Some(150_000),
            execution_success: Some(true),
        };
        let json = serde_json::to_string(&record).expect("serialize");
        let deserialized: TxRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.status, TxStatus::Confirmed);
        assert_eq!(deserialized.gas_used, Some(150_000));
        assert_eq!(deserialized.block_number, Some(19_000_000));
    }

    #[test]
    fn default_monitor() {
        let monitor = ConfirmationMonitor::default();
        assert!(monitor.is_empty());
        assert_eq!(monitor.tracked_count(), 0);
    }

    #[tokio::test]
    async fn check_status_with_anvil() {
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

        let monitor = ConfirmationMonitor::with_defaults();

        // Track a fake transaction hash that doesn't exist on-chain.
        monitor
            .track_transaction(TX_HASH, 1, FILLER)
            .expect("track");

        // Check status — should remain pending (no receipt).
        let record = monitor
            .check_status(&TX_HASH, &provider)
            .await
            .expect("check");
        assert_eq!(record.status, TxStatus::Pending);
        assert!(record.block_number.is_none());
    }

    #[tokio::test]
    async fn check_status_untracked() {
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

        let monitor = ConfirmationMonitor::with_defaults();

        let result = monitor.check_status(&TX_HASH, &provider).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    fn make_record(status: TxStatus) -> TxRecord {
        TxRecord {
            tx_hash: TX_HASH,
            chain_id: 1,
            from: FILLER,
            status,
            block_number: None,
            confirmations: 0,
            submitted_at: current_timestamp(),
            updated_at: current_timestamp(),
            gas_used: None,
            execution_success: None,
        }
    }
}
