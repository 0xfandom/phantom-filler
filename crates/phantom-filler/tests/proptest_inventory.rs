//! Property-based tests for inventory: risk management and P&L tracking.

use alloy::primitives::{address, B256, U256};
use phantom_inventory::pnl::{FillRecord, FillStatus, PnlTracker};
use phantom_inventory::risk::{RiskCheckOutcome, RiskManager};
use proptest::prelude::*;

// ─── Risk Manager Properties ────────────────────────────────────────

proptest! {
    #[test]
    fn risk_small_fill_always_passes(fill_value in 1u64..1_000_000_000) {
        let risk = RiskManager::with_defaults();
        let result = risk.check_fill(U256::from(fill_value), U256::ZERO);
        prop_assert_eq!(result.outcome, RiskCheckOutcome::Passed);
    }

    #[test]
    fn risk_huge_fill_always_rejected(fill_eth in 11u128..1000) {
        let risk = RiskManager::with_defaults();
        // Default max single fill is 10 ETH.
        let fill_value = U256::from(fill_eth * 1_000_000_000_000_000_000);
        let result = risk.check_fill(fill_value, U256::ZERO);
        prop_assert_eq!(result.outcome, RiskCheckOutcome::Rejected);
    }

    #[test]
    fn risk_pending_count_tracks_starts_and_completes(starts in 1u32..20, completes in 0u32..20) {
        let risk = RiskManager::with_defaults();
        let actual_completes = completes.min(starts);

        for _ in 0..starts {
            let _ = risk.record_fill_start();
        }
        for _ in 0..actual_completes {
            risk.record_fill_complete(0);
        }

        let expected = starts - actual_completes;
        prop_assert_eq!(risk.pending_count(), expected);
    }

    #[test]
    fn risk_check_is_deterministic(fill_value in 0u64..u64::MAX, position in 0u64..u64::MAX) {
        let risk = RiskManager::with_defaults();
        let a = risk.check_fill(U256::from(fill_value), U256::from(position));
        let b = risk.check_fill(U256::from(fill_value), U256::from(position));
        prop_assert_eq!(a.outcome, b.outcome);
    }

    #[test]
    fn risk_snapshot_reflects_state(n_starts in 0u32..10) {
        let risk = RiskManager::with_defaults();
        for _ in 0..n_starts {
            let _ = risk.record_fill_start();
        }
        let snapshot = risk.risk_snapshot();
        prop_assert_eq!(snapshot.pending_fills, n_starts);
    }
}

// ─── P&L Tracker Properties ────────────────────────────────────────

fn make_fill(id: u64, pnl: i128) -> FillRecord {
    FillRecord {
        fill_id: format!("prop-fill-{id}"),
        chain_id: 1,
        token_in: address!("0000000000000000000000000000000000000001"),
        token_out: address!("0000000000000000000000000000000000000002"),
        amount_in: U256::from(1_000_000_000_000_000_000u64),
        amount_out: U256::from(2_000_000_000u64),
        gas_cost_wei: 50_000_000_000_000u128,
        pnl_wei: pnl,
        tx_hash: B256::from([id as u8; 32]),
        timestamp: 1_700_000_000 + id,
        status: FillStatus::Confirmed,
    }
}

proptest! {
    #[test]
    fn pnl_fill_count_matches_recorded(n in 1u64..50) {
        let tracker = PnlTracker::with_defaults();
        for i in 0..n {
            tracker.record_fill(make_fill(i, 100_000_000_000_000));
        }
        assert_eq!(tracker.fill_count(), n);
    }

    #[test]
    fn pnl_summary_total_matches_count(n in 1u64..50) {
        let tracker = PnlTracker::with_defaults();
        for i in 0..n {
            tracker.record_fill(make_fill(i, 100_000_000_000_000));
        }
        let summary = tracker.summary();
        assert_eq!(summary.total_fills, n);
    }

    #[test]
    fn pnl_positive_fills_yield_positive_total(n in 1u64..20) {
        let tracker = PnlTracker::with_defaults();
        for i in 0..n {
            tracker.record_fill(make_fill(i, 100_000_000_000_000));
        }
        let summary = tracker.summary();
        prop_assert!(summary.total_realized_pnl_wei > 0);
    }

    #[test]
    fn pnl_negative_fills_yield_negative_total(n in 1u64..20) {
        let tracker = PnlTracker::with_defaults();
        for i in 0..n {
            tracker.record_fill(make_fill(i, -100_000_000_000_000));
        }
        let summary = tracker.summary();
        prop_assert!(summary.total_realized_pnl_wei < 0);
    }

    #[test]
    fn pnl_daily_and_token_not_empty_after_fills(n in 1u64..20) {
        let tracker = PnlTracker::with_defaults();
        for i in 0..n {
            tracker.record_fill(make_fill(i, 100_000_000_000_000));
        }
        prop_assert!(!tracker.all_daily_pnl().is_empty());
        prop_assert!(!tracker.all_token_pnl().is_empty());
    }
}
