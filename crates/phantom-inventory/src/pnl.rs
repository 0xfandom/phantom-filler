//! P&L tracking and fill accounting.
//!
//! Maintains detailed records of every fill execution, aggregating
//! realized P&L by day and by token/chain pair. Provides summary
//! snapshots for dashboards and risk reporting.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use alloy::primitives::{Address, B256, U256};
use chrono::{NaiveDate, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Configuration for the P&L tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlConfig {
    /// Maximum number of fill records to retain in memory.
    pub max_fill_history: usize,
    /// Whether to track per-token breakdowns.
    pub track_per_token: bool,
}

impl Default for PnlConfig {
    fn default() -> Self {
        Self {
            max_fill_history: 10_000,
            track_per_token: true,
        }
    }
}

/// Status of a recorded fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatus {
    /// Fill transaction submitted but not yet confirmed.
    Pending,
    /// Fill confirmed on-chain.
    Confirmed,
    /// Fill transaction reverted.
    Reverted,
}

/// A single fill execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillRecord {
    /// Unique identifier for this fill.
    pub fill_id: String,
    /// Chain where the fill was executed.
    pub chain_id: u64,
    /// Token spent (input).
    pub token_in: Address,
    /// Token received (output).
    pub token_out: Address,
    /// Amount of token_in spent.
    pub amount_in: U256,
    /// Amount of token_out received.
    pub amount_out: U256,
    /// Gas cost in wei.
    pub gas_cost_wei: u128,
    /// Net profit/loss in wei (amount_out value - amount_in value - gas).
    /// Positive = profit, negative = loss.
    pub pnl_wei: i128,
    /// Transaction hash.
    pub tx_hash: B256,
    /// Unix timestamp of the fill.
    pub timestamp: u64,
    /// Current status of the fill.
    pub status: FillStatus,
}

/// Key for per-token P&L aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenPnlKey {
    token: Address,
    chain_id: u64,
}

/// Aggregated P&L for a specific token on a specific chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPnl {
    /// Token address.
    pub token: Address,
    /// Chain identifier.
    pub chain_id: u64,
    /// Total volume bought (in token units).
    pub total_bought: U256,
    /// Total volume sold (in token units).
    pub total_sold: U256,
    /// Total gas spent in wei.
    pub total_gas_wei: u128,
    /// Net realized P&L in wei.
    pub net_pnl_wei: i128,
    /// Number of fills involving this token.
    pub fill_count: u32,
}

/// Key for daily P&L aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DailyKey {
    date: NaiveDate,
}

/// Aggregated P&L for a single day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPnl {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Net realized P&L in wei.
    pub realized_pnl_wei: i128,
    /// Total gas spent in wei.
    pub gas_spent_wei: u128,
    /// Number of confirmed fills.
    pub fill_count: u32,
    /// Number of reverted fills.
    pub reverted_count: u32,
}

/// Overall P&L summary snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlSummary {
    /// Total realized P&L across all time in wei.
    pub total_realized_pnl_wei: i64,
    /// Total number of fills recorded.
    pub total_fills: u64,
    /// Today's P&L.
    pub today: Option<DailyPnl>,
    /// Number of tokens tracked.
    pub tracked_tokens: usize,
    /// Number of days with recorded activity.
    pub active_days: usize,
}

/// Tracks fill-level P&L with daily and per-token aggregation.
///
/// Thread-safe via `DashMap` for concurrent fill recording and querying.
/// Atomic counters provide fast access to running totals.
pub struct PnlTracker {
    config: PnlConfig,
    /// All fill records keyed by fill_id.
    fills: DashMap<String, FillRecord>,
    /// Daily P&L aggregation.
    daily: DashMap<DailyKey, DailyPnl>,
    /// Per-token P&L aggregation.
    tokens: DashMap<TokenPnlKey, TokenPnl>,
    /// Running total realized P&L in wei (capped to i64 range).
    total_realized_wei: AtomicI64,
    /// Running total fill count.
    total_fills: AtomicU64,
    /// Ordered fill IDs for history retrieval (newest first).
    fill_order: DashMap<u64, String>,
    /// Next sequence number for ordering fills.
    next_seq: AtomicU64,
}

impl PnlTracker {
    /// Creates a new P&L tracker with the given configuration.
    pub fn new(config: PnlConfig) -> Self {
        Self {
            config,
            fills: DashMap::new(),
            daily: DashMap::new(),
            tokens: DashMap::new(),
            total_realized_wei: AtomicI64::new(0),
            total_fills: AtomicU64::new(0),
            fill_order: DashMap::new(),
            next_seq: AtomicU64::new(0),
        }
    }

    /// Creates a tracker with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(PnlConfig::default())
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &PnlConfig {
        &self.config
    }

    /// Records a new fill execution.
    ///
    /// Updates daily and per-token aggregations atomically.
    pub fn record_fill(&self, record: FillRecord) {
        let fill_id = record.fill_id.clone();
        let pnl = record.pnl_wei;
        let gas = record.gas_cost_wei;
        let status = record.status;
        let chain_id = record.chain_id;
        let token_in = record.token_in;
        let token_out = record.token_out;
        let amount_in = record.amount_in;
        let amount_out = record.amount_out;
        let timestamp = record.timestamp;

        // Store fill record.
        self.fills.insert(fill_id.clone(), record);

        // Track ordering.
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        self.fill_order.insert(seq, fill_id.clone());

        // Update running totals.
        self.total_fills.fetch_add(1, Ordering::SeqCst);

        // Only count realized P&L for confirmed fills.
        if status == FillStatus::Confirmed {
            // Clamp i128 to i64 range for atomic update.
            let clamped = pnl.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            self.total_realized_wei.fetch_add(clamped, Ordering::SeqCst);
        }

        // Update daily aggregation.
        let date = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .unwrap_or_else(Utc::now)
            .date_naive();

        let daily_key = DailyKey { date };
        let mut daily = self.daily.entry(daily_key).or_insert_with(|| DailyPnl {
            date: date.format("%Y-%m-%d").to_string(),
            realized_pnl_wei: 0,
            gas_spent_wei: 0,
            fill_count: 0,
            reverted_count: 0,
        });

        daily.gas_spent_wei = daily.gas_spent_wei.saturating_add(gas);

        match status {
            FillStatus::Confirmed => {
                daily.realized_pnl_wei = daily.realized_pnl_wei.saturating_add(pnl);
                daily.fill_count = daily.fill_count.saturating_add(1);
            }
            FillStatus::Reverted => {
                daily.reverted_count = daily.reverted_count.saturating_add(1);
                // Gas is still spent on reverts.
            }
            FillStatus::Pending => {
                daily.fill_count = daily.fill_count.saturating_add(1);
            }
        }
        drop(daily);

        // Update per-token aggregation (for both input and output tokens).
        if self.config.track_per_token {
            self.update_token_pnl(
                token_in,
                chain_id,
                U256::ZERO,
                amount_in,
                gas / 2,
                pnl / 2,
                status,
            );
            self.update_token_pnl(
                token_out,
                chain_id,
                amount_out,
                U256::ZERO,
                gas / 2,
                pnl / 2,
                status,
            );
        }

        debug!(
            fill_id = %fill_id,
            pnl_wei = pnl,
            chain_id,
            "recorded fill"
        );

        // Prune old fills if over limit.
        self.prune_if_needed();
    }

    /// Updates P&L aggregation for a specific token.
    #[allow(clippy::too_many_arguments)]
    fn update_token_pnl(
        &self,
        token: Address,
        chain_id: u64,
        bought: U256,
        sold: U256,
        gas_wei: u128,
        pnl_share: i128,
        status: FillStatus,
    ) {
        let key = TokenPnlKey { token, chain_id };
        let mut entry = self.tokens.entry(key).or_insert_with(|| TokenPnl {
            token,
            chain_id,
            total_bought: U256::ZERO,
            total_sold: U256::ZERO,
            total_gas_wei: 0,
            net_pnl_wei: 0,
            fill_count: 0,
        });

        entry.total_bought = entry.total_bought.saturating_add(bought);
        entry.total_sold = entry.total_sold.saturating_add(sold);
        entry.total_gas_wei = entry.total_gas_wei.saturating_add(gas_wei);
        entry.fill_count = entry.fill_count.saturating_add(1);

        if status == FillStatus::Confirmed {
            entry.net_pnl_wei = entry.net_pnl_wei.saturating_add(pnl_share);
        }
    }

    /// Updates the status of an existing fill record.
    ///
    /// When a fill transitions to `Confirmed`, its P&L is added to the
    /// realized total. When it transitions to `Reverted`, the daily
    /// revert counter is incremented.
    pub fn update_fill_status(&self, fill_id: &str, new_status: FillStatus) -> bool {
        let mut entry = match self.fills.get_mut(fill_id) {
            Some(e) => e,
            None => return false,
        };

        let old_status = entry.status;
        if old_status == new_status {
            return true;
        }

        let pnl = entry.pnl_wei;
        let timestamp = entry.timestamp;
        entry.status = new_status;
        drop(entry);

        // Update realized P&L when transitioning to Confirmed.
        if old_status == FillStatus::Pending && new_status == FillStatus::Confirmed {
            let clamped = pnl.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            self.total_realized_wei.fetch_add(clamped, Ordering::SeqCst);
        }

        // Update daily aggregation for status changes.
        let date = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .unwrap_or_else(Utc::now)
            .date_naive();
        let daily_key = DailyKey { date };

        if let Some(mut daily) = self.daily.get_mut(&daily_key) {
            match new_status {
                FillStatus::Confirmed => {
                    if old_status == FillStatus::Pending {
                        daily.realized_pnl_wei = daily.realized_pnl_wei.saturating_add(pnl);
                    }
                }
                FillStatus::Reverted => {
                    daily.reverted_count = daily.reverted_count.saturating_add(1);
                    if old_status == FillStatus::Pending {
                        // Remove from fill count since it never completed.
                        daily.fill_count = daily.fill_count.saturating_sub(1);
                    }
                }
                FillStatus::Pending => {} // no-op
            }
        }

        info!(fill_id, ?old_status, ?new_status, "updated fill status");
        true
    }

    /// Returns the fill record for the given ID.
    pub fn get_fill(&self, fill_id: &str) -> Option<FillRecord> {
        self.fills.get(fill_id).map(|r| r.clone())
    }

    /// Returns the daily P&L for the given date.
    pub fn get_daily_pnl(&self, date: NaiveDate) -> Option<DailyPnl> {
        let key = DailyKey { date };
        self.daily.get(&key).map(|r| r.clone())
    }

    /// Returns today's P&L.
    pub fn get_today_pnl(&self) -> Option<DailyPnl> {
        self.get_daily_pnl(Utc::now().date_naive())
    }

    /// Returns the per-token P&L for the given token and chain.
    pub fn get_token_pnl(&self, token: Address, chain_id: u64) -> Option<TokenPnl> {
        let key = TokenPnlKey { token, chain_id };
        self.tokens.get(&key).map(|r| r.clone())
    }

    /// Returns all per-token P&L records.
    pub fn all_token_pnl(&self) -> Vec<TokenPnl> {
        self.tokens.iter().map(|r| r.value().clone()).collect()
    }

    /// Returns all daily P&L records sorted by date descending.
    pub fn all_daily_pnl(&self) -> Vec<DailyPnl> {
        let mut days: Vec<DailyPnl> = self.daily.iter().map(|r| r.value().clone()).collect();
        days.sort_by(|a, b| b.date.cmp(&a.date));
        days
    }

    /// Returns the most recent fill records up to `limit`.
    pub fn recent_fills(&self, limit: usize) -> Vec<FillRecord> {
        let total = self.next_seq.load(Ordering::SeqCst);
        let start = total.saturating_sub(limit as u64);

        let mut fills = Vec::with_capacity(limit);
        for seq in (start..total).rev() {
            if let Some(fill_id) = self.fill_order.get(&seq) {
                if let Some(record) = self.fills.get(fill_id.value()) {
                    fills.push(record.clone());
                }
            }
            if fills.len() >= limit {
                break;
            }
        }
        fills
    }

    /// Returns an overall P&L summary snapshot.
    pub fn summary(&self) -> PnlSummary {
        PnlSummary {
            total_realized_pnl_wei: self.total_realized_wei.load(Ordering::SeqCst),
            total_fills: self.total_fills.load(Ordering::SeqCst),
            today: self.get_today_pnl(),
            tracked_tokens: self.tokens.len(),
            active_days: self.daily.len(),
        }
    }

    /// Returns the total number of fill records.
    pub fn fill_count(&self) -> u64 {
        self.total_fills.load(Ordering::SeqCst)
    }

    /// Returns the total realized P&L in wei.
    pub fn total_realized_pnl(&self) -> i64 {
        self.total_realized_wei.load(Ordering::SeqCst)
    }

    /// Removes old fill records when exceeding the configured limit.
    fn prune_if_needed(&self) {
        let total = self.next_seq.load(Ordering::SeqCst) as usize;
        if total <= self.config.max_fill_history {
            return;
        }

        let to_remove = total - self.config.max_fill_history;
        let start_seq = total.saturating_sub(total) as u64; // always 0
        for seq in start_seq..(to_remove as u64) {
            if let Some((_, fill_id)) = self.fill_order.remove(&seq) {
                self.fills.remove(&fill_id);
            }
        }

        debug!(
            removed = to_remove,
            remaining = self.fills.len(),
            "pruned old fill records"
        );
    }
}

impl Default for PnlTracker {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, b256};

    fn sample_fill(id: &str, pnl: i128, status: FillStatus) -> FillRecord {
        FillRecord {
            fill_id: id.to_string(),
            chain_id: 1,
            token_in: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            token_out: address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            amount_in: U256::from(1000u64),
            amount_out: U256::from(1100u64),
            gas_cost_wei: 50_000,
            pnl_wei: pnl,
            tx_hash: b256!("0x1111111111111111111111111111111111111111111111111111111111111111"),
            timestamp: Utc::now().timestamp() as u64,
            status,
        }
    }

    #[test]
    fn config_default() {
        let config = PnlConfig::default();
        assert_eq!(config.max_fill_history, 10_000);
        assert!(config.track_per_token);
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = PnlConfig {
            max_fill_history: 500,
            track_per_token: false,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: PnlConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.max_fill_history, 500);
        assert!(!deserialized.track_per_token);
    }

    #[test]
    fn fill_status_serde() {
        for status in [
            FillStatus::Pending,
            FillStatus::Confirmed,
            FillStatus::Reverted,
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            let deserialized: FillStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn fill_record_serde_roundtrip() {
        let record = sample_fill("fill-1", 500, FillStatus::Confirmed);
        let json = serde_json::to_string(&record).expect("serialize");
        let deserialized: FillRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.fill_id, "fill-1");
        assert_eq!(deserialized.pnl_wei, 500);
        assert_eq!(deserialized.status, FillStatus::Confirmed);
    }

    #[test]
    fn record_and_retrieve_fill() {
        let tracker = PnlTracker::with_defaults();
        let fill = sample_fill("fill-1", 1000, FillStatus::Confirmed);
        tracker.record_fill(fill);

        let retrieved = tracker.get_fill("fill-1").expect("fill exists");
        assert_eq!(retrieved.pnl_wei, 1000);
        assert_eq!(tracker.fill_count(), 1);
    }

    #[test]
    fn total_realized_pnl_confirmed_only() {
        let tracker = PnlTracker::with_defaults();

        // Confirmed fill: counted.
        tracker.record_fill(sample_fill("fill-1", 500, FillStatus::Confirmed));
        assert_eq!(tracker.total_realized_pnl(), 500);

        // Pending fill: not counted.
        tracker.record_fill(sample_fill("fill-2", 300, FillStatus::Pending));
        assert_eq!(tracker.total_realized_pnl(), 500);

        // Reverted fill: not counted.
        tracker.record_fill(sample_fill("fill-3", -200, FillStatus::Reverted));
        assert_eq!(tracker.total_realized_pnl(), 500);
    }

    #[test]
    fn total_pnl_with_losses() {
        let tracker = PnlTracker::with_defaults();

        tracker.record_fill(sample_fill("fill-1", 1000, FillStatus::Confirmed));
        tracker.record_fill(sample_fill("fill-2", -600, FillStatus::Confirmed));
        assert_eq!(tracker.total_realized_pnl(), 400);
    }

    #[test]
    fn daily_pnl_aggregation() {
        let tracker = PnlTracker::with_defaults();
        let today = Utc::now().date_naive();

        tracker.record_fill(sample_fill("fill-1", 500, FillStatus::Confirmed));
        tracker.record_fill(sample_fill("fill-2", -200, FillStatus::Confirmed));
        tracker.record_fill(sample_fill("fill-3", 0, FillStatus::Reverted));

        let daily = tracker.get_daily_pnl(today).expect("daily exists");
        assert_eq!(daily.realized_pnl_wei, 300); // 500 + (-200)
        assert_eq!(daily.fill_count, 2); // 2 confirmed
        assert_eq!(daily.reverted_count, 1);
        assert!(daily.gas_spent_wei > 0);
    }

    #[test]
    fn today_pnl() {
        let tracker = PnlTracker::with_defaults();
        assert!(tracker.get_today_pnl().is_none());

        tracker.record_fill(sample_fill("fill-1", 100, FillStatus::Confirmed));
        let today = tracker.get_today_pnl().expect("today exists");
        assert_eq!(today.realized_pnl_wei, 100);
    }

    #[test]
    fn per_token_pnl() {
        let config = PnlConfig {
            track_per_token: true,
            ..Default::default()
        };
        let tracker = PnlTracker::new(config);
        let usdc = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

        tracker.record_fill(sample_fill("fill-1", 1000, FillStatus::Confirmed));

        let token_pnl = tracker.get_token_pnl(usdc, 1).expect("token pnl exists");
        assert_eq!(token_pnl.token, usdc);
        assert_eq!(token_pnl.chain_id, 1);
        assert!(token_pnl.fill_count > 0);
    }

    #[test]
    fn per_token_disabled() {
        let config = PnlConfig {
            track_per_token: false,
            ..Default::default()
        };
        let tracker = PnlTracker::new(config);
        let usdc = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

        tracker.record_fill(sample_fill("fill-1", 1000, FillStatus::Confirmed));

        assert!(tracker.get_token_pnl(usdc, 1).is_none());
        assert!(tracker.all_token_pnl().is_empty());
    }

    #[test]
    fn update_fill_status_pending_to_confirmed() {
        let tracker = PnlTracker::with_defaults();

        tracker.record_fill(sample_fill("fill-1", 800, FillStatus::Pending));
        assert_eq!(tracker.total_realized_pnl(), 0); // pending, not realized

        let updated = tracker.update_fill_status("fill-1", FillStatus::Confirmed);
        assert!(updated);
        assert_eq!(tracker.total_realized_pnl(), 800); // now realized
    }

    #[test]
    fn update_fill_status_pending_to_reverted() {
        let tracker = PnlTracker::with_defaults();
        let today = Utc::now().date_naive();

        tracker.record_fill(sample_fill("fill-1", 800, FillStatus::Pending));
        let daily = tracker.get_daily_pnl(today).expect("daily");
        assert_eq!(daily.fill_count, 1);
        assert_eq!(daily.reverted_count, 0);

        let updated = tracker.update_fill_status("fill-1", FillStatus::Reverted);
        assert!(updated);
        assert_eq!(tracker.total_realized_pnl(), 0); // reverted, not realized

        let daily = tracker.get_daily_pnl(today).expect("daily");
        assert_eq!(daily.fill_count, 0); // removed from fill count
        assert_eq!(daily.reverted_count, 1);
    }

    #[test]
    fn update_nonexistent_fill() {
        let tracker = PnlTracker::with_defaults();
        assert!(!tracker.update_fill_status("nonexistent", FillStatus::Confirmed));
    }

    #[test]
    fn update_same_status_noop() {
        let tracker = PnlTracker::with_defaults();
        tracker.record_fill(sample_fill("fill-1", 500, FillStatus::Confirmed));

        let updated = tracker.update_fill_status("fill-1", FillStatus::Confirmed);
        assert!(updated);
        assert_eq!(tracker.total_realized_pnl(), 500); // unchanged
    }

    #[test]
    fn recent_fills_ordering() {
        let tracker = PnlTracker::with_defaults();

        for i in 0..5 {
            tracker.record_fill(sample_fill(
                &format!("fill-{i}"),
                i * 100,
                FillStatus::Confirmed,
            ));
        }

        let recent = tracker.recent_fills(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].fill_id, "fill-4"); // most recent first
        assert_eq!(recent[1].fill_id, "fill-3");
        assert_eq!(recent[2].fill_id, "fill-2");
    }

    #[test]
    fn recent_fills_fewer_than_limit() {
        let tracker = PnlTracker::with_defaults();
        tracker.record_fill(sample_fill("fill-1", 100, FillStatus::Confirmed));

        let recent = tracker.recent_fills(10);
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn summary_snapshot() {
        let tracker = PnlTracker::with_defaults();

        tracker.record_fill(sample_fill("fill-1", 500, FillStatus::Confirmed));
        tracker.record_fill(sample_fill("fill-2", -100, FillStatus::Confirmed));

        let summary = tracker.summary();
        assert_eq!(summary.total_realized_pnl_wei, 400);
        assert_eq!(summary.total_fills, 2);
        assert!(summary.today.is_some());
        assert_eq!(summary.active_days, 1);
    }

    #[test]
    fn summary_serde_roundtrip() {
        let summary = PnlSummary {
            total_realized_pnl_wei: 1000,
            total_fills: 5,
            today: Some(DailyPnl {
                date: "2025-01-15".to_string(),
                realized_pnl_wei: 1000,
                gas_spent_wei: 50000,
                fill_count: 5,
                reverted_count: 0,
            }),
            tracked_tokens: 3,
            active_days: 1,
        };
        let json = serde_json::to_string(&summary).expect("serialize");
        let deserialized: PnlSummary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.total_realized_pnl_wei, 1000);
        assert_eq!(deserialized.total_fills, 5);
    }

    #[test]
    fn daily_pnl_serde_roundtrip() {
        let daily = DailyPnl {
            date: "2025-01-15".to_string(),
            realized_pnl_wei: -500,
            gas_spent_wei: 100_000,
            fill_count: 10,
            reverted_count: 2,
        };
        let json = serde_json::to_string(&daily).expect("serialize");
        let deserialized: DailyPnl = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.realized_pnl_wei, -500);
        assert_eq!(deserialized.reverted_count, 2);
    }

    #[test]
    fn token_pnl_serde_roundtrip() {
        let token_pnl = TokenPnl {
            token: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            chain_id: 1,
            total_bought: U256::from(5000u64),
            total_sold: U256::from(4000u64),
            total_gas_wei: 200_000,
            net_pnl_wei: 1000,
            fill_count: 8,
        };
        let json = serde_json::to_string(&token_pnl).expect("serialize");
        let deserialized: TokenPnl = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.fill_count, 8);
        assert_eq!(deserialized.net_pnl_wei, 1000);
    }

    #[test]
    fn prune_old_fills() {
        let config = PnlConfig {
            max_fill_history: 5,
            track_per_token: false,
        };
        let tracker = PnlTracker::new(config);

        for i in 0..10 {
            tracker.record_fill(sample_fill(
                &format!("fill-{i}"),
                100,
                FillStatus::Confirmed,
            ));
        }

        // Oldest fills should be pruned.
        assert!(tracker.get_fill("fill-0").is_none());
        assert!(tracker.get_fill("fill-1").is_none());
        // Recent fills should remain.
        assert!(tracker.get_fill("fill-9").is_some());
        assert!(tracker.get_fill("fill-8").is_some());
    }

    #[test]
    fn all_daily_pnl_sorted() {
        let tracker = PnlTracker::with_defaults();

        // Create fills on "different days" by using different timestamps.
        let mut fill1 = sample_fill("fill-1", 100, FillStatus::Confirmed);
        fill1.timestamp = 1704067200; // 2024-01-01

        let mut fill2 = sample_fill("fill-2", 200, FillStatus::Confirmed);
        fill2.timestamp = 1704153600; // 2024-01-02

        tracker.record_fill(fill1);
        tracker.record_fill(fill2);

        let all = tracker.all_daily_pnl();
        assert_eq!(all.len(), 2);
        // Sorted descending by date.
        assert!(all[0].date > all[1].date);
    }

    #[test]
    fn default_tracker() {
        let tracker = PnlTracker::default();
        assert_eq!(tracker.fill_count(), 0);
        assert_eq!(tracker.total_realized_pnl(), 0);
        assert!(tracker.summary().today.is_none());
    }

    #[test]
    fn concurrent_fill_recording() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(PnlTracker::with_defaults());
        let mut handles = vec![];

        for i in 0..10 {
            let tracker = Arc::clone(&tracker);
            handles.push(thread::spawn(move || {
                tracker.record_fill(sample_fill(
                    &format!("concurrent-{i}"),
                    (i as i128) * 100,
                    FillStatus::Confirmed,
                ));
            }));
        }

        for handle in handles {
            handle.join().expect("thread panicked");
        }

        assert_eq!(tracker.fill_count(), 10);
        // P&L should be sum of 0+100+200+...+900 = 4500
        assert_eq!(tracker.total_realized_pnl(), 4500);
    }

    #[test]
    fn multiple_chains_token_pnl() {
        let tracker = PnlTracker::with_defaults();
        let usdc = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");

        // Fill on chain 1.
        let mut fill1 = sample_fill("fill-1", 500, FillStatus::Confirmed);
        fill1.chain_id = 1;
        tracker.record_fill(fill1);

        // Fill on chain 42161 (Arbitrum).
        let mut fill2 = sample_fill("fill-2", 300, FillStatus::Confirmed);
        fill2.chain_id = 42161;
        tracker.record_fill(fill2);

        // Token P&L should be separate per chain.
        let pnl_eth = tracker.get_token_pnl(usdc, 1).expect("chain 1");
        let pnl_arb = tracker.get_token_pnl(usdc, 42161).expect("chain 42161");
        assert_ne!(pnl_eth.chain_id, pnl_arb.chain_id);
    }
}
