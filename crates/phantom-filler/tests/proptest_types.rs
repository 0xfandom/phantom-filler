//! Property-based tests for core types and invariants.

use alloy::primitives::{Address, B256, U256};
use chrono::{Duration, Utc};
use phantom_common::types::{
    ChainId, DutchAuctionOrder, OrderId, OrderInput, OrderOutput, OrderStatus, Token, TokenAmount,
};
use proptest::prelude::*;

// ─── Strategies ─────────────────────────────────────────────────────

fn arb_chain_id() -> impl Strategy<Value = ChainId> {
    prop_oneof![
        Just(ChainId::Ethereum),
        Just(ChainId::Arbitrum),
        Just(ChainId::Base),
        Just(ChainId::Polygon),
        Just(ChainId::Optimism),
    ]
}

fn arb_address() -> impl Strategy<Value = Address> {
    prop::array::uniform20(any::<u8>()).prop_map(Address::from)
}

fn arb_b256() -> impl Strategy<Value = B256> {
    prop::array::uniform32(any::<u8>()).prop_map(B256::from)
}

fn arb_order_id() -> impl Strategy<Value = OrderId> {
    arb_b256().prop_map(OrderId::new)
}

fn arb_token() -> impl Strategy<Value = Token> {
    (arb_address(), arb_chain_id(), 0u8..18, "[A-Z]{3,5}")
        .prop_map(|(addr, chain, dec, sym)| Token::new(addr, chain, dec, sym))
}

fn arb_token_amount() -> impl Strategy<Value = TokenAmount> {
    (arb_token(), 0u64..u64::MAX).prop_map(|(token, amt)| TokenAmount::new(token, U256::from(amt)))
}

fn arb_order_status() -> impl Strategy<Value = OrderStatus> {
    prop_oneof![
        Just(OrderStatus::Pending),
        Just(OrderStatus::Active),
        Just(OrderStatus::Filled),
        Just(OrderStatus::Expired),
        Just(OrderStatus::Cancelled),
    ]
}

fn arb_dutch_auction_order() -> impl Strategy<Value = DutchAuctionOrder> {
    (
        arb_order_id(),
        arb_chain_id(),
        arb_address(),
        arb_address(),
        0u64..1_000_000,
        arb_address(),
        1u64..1_000_000_000_000_000_000,
        arb_address(),
        1u64..1_000_000_000_000_000_000,
        1u64..1_000_000_000_000_000_000,
        arb_address(),
    )
        .prop_map(
            |(
                id,
                chain,
                reactor,
                signer,
                nonce,
                in_token,
                in_amount,
                out_token,
                start,
                end,
                recipient,
            )| {
                let now = Utc::now();
                let (start_amount, end_amount) = if start >= end {
                    (start, end)
                } else {
                    (end, start)
                };
                DutchAuctionOrder {
                    id,
                    chain_id: chain,
                    reactor,
                    signer,
                    nonce: U256::from(nonce),
                    decay_start_time: now,
                    decay_end_time: now + Duration::minutes(60),
                    deadline: now + Duration::minutes(120),
                    input: OrderInput {
                        token: in_token,
                        amount: U256::from(in_amount),
                    },
                    outputs: vec![OrderOutput {
                        token: out_token,
                        start_amount: U256::from(start_amount),
                        end_amount: U256::from(end_amount),
                        recipient,
                    }],
                }
            },
        )
}

// ─── Token Properties ───────────────────────────────────────────────

proptest! {
    #[test]
    fn token_serde_roundtrip(token in arb_token()) {
        let json = serde_json::to_string(&token).unwrap();
        let deserialized: Token = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(token, deserialized);
    }

    #[test]
    fn token_amount_zero_iff_amount_is_zero(amount in 0u64..u64::MAX) {
        let token = Token::new(
            Address::ZERO,
            ChainId::Ethereum,
            18,
            "TEST",
        );
        let ta = TokenAmount::new(token, U256::from(amount));
        prop_assert_eq!(ta.is_zero(), amount == 0);
    }

    #[test]
    fn token_amount_serde_roundtrip(ta in arb_token_amount()) {
        let json = serde_json::to_string(&ta).unwrap();
        let deserialized: TokenAmount = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(ta, deserialized);
    }
}

// ─── ChainId Properties ────────────────────────────────────────────

proptest! {
    #[test]
    fn chain_id_serde_roundtrip(chain in arb_chain_id()) {
        let json = serde_json::to_string(&chain).unwrap();
        let deserialized: ChainId = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(chain, deserialized);
    }
}

// ─── OrderStatus Properties ────────────────────────────────────────

proptest! {
    #[test]
    fn order_status_serde_roundtrip(status in arb_order_status()) {
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: OrderStatus = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(status, deserialized);
    }

    #[test]
    fn order_status_display_not_empty(status in arb_order_status()) {
        let display = status.to_string();
        prop_assert!(!display.is_empty());
    }
}

// ─── OrderId Properties ────────────────────────────────────────────

proptest! {
    #[test]
    fn order_id_display_has_0x_prefix(id in arb_order_id()) {
        let display = id.to_string();
        prop_assert!(display.starts_with("0x"));
        prop_assert_eq!(display.len(), 66); // 0x + 64 hex chars
    }

    #[test]
    fn order_id_from_bytes_roundtrip(bytes in prop::array::uniform32(any::<u8>())) {
        let fixed: alloy::primitives::FixedBytes<32> = bytes.into();
        let id = OrderId::new(fixed);
        prop_assert_eq!(*id.as_bytes(), fixed);
    }
}

// ─── Dutch Auction Decay Properties ─────────────────────────────────

proptest! {
    #[test]
    fn decay_at_start_returns_start_amount(order in arb_dutch_auction_order()) {
        let amount = order.current_output_amount(0, order.decay_start_time);
        prop_assert!(amount.is_some());
        prop_assert_eq!(amount.unwrap(), order.outputs[0].start_amount);
    }

    #[test]
    fn decay_at_end_returns_end_amount(order in arb_dutch_auction_order()) {
        let amount = order.current_output_amount(0, order.decay_end_time);
        prop_assert!(amount.is_some());
        prop_assert_eq!(amount.unwrap(), order.outputs[0].end_amount);
    }

    #[test]
    fn decay_before_start_returns_start_amount(order in arb_dutch_auction_order()) {
        let before = order.decay_start_time - Duration::seconds(1);
        let amount = order.current_output_amount(0, before);
        prop_assert!(amount.is_some());
        prop_assert_eq!(amount.unwrap(), order.outputs[0].start_amount);
    }

    #[test]
    fn decay_monotonically_decreasing(
        order in arb_dutch_auction_order(),
        t1_secs in 0u32..3600,
        t2_secs in 0u32..3600,
    ) {
        let (early_secs, late_secs) = if t1_secs <= t2_secs { (t1_secs, t2_secs) } else { (t2_secs, t1_secs) };
        let early = order.decay_start_time + Duration::seconds(i64::from(early_secs));
        let late = order.decay_start_time + Duration::seconds(i64::from(late_secs));

        let early_amount = order.current_output_amount(0, early).unwrap();
        let late_amount = order.current_output_amount(0, late).unwrap();

        // Decay is monotonically decreasing (start >= end).
        prop_assert!(early_amount >= late_amount);
    }

    #[test]
    fn decay_amount_within_bounds(
        order in arb_dutch_auction_order(),
        secs in 0u32..7200,
    ) {
        let t = order.decay_start_time + Duration::seconds(i64::from(secs));
        let amount = order.current_output_amount(0, t).unwrap();
        prop_assert!(amount <= order.outputs[0].start_amount);
        prop_assert!(amount >= order.outputs[0].end_amount);
    }

    #[test]
    fn invalid_output_index_returns_none(
        order in arb_dutch_auction_order(),
        idx in 1usize..100,
    ) {
        // Orders have exactly 1 output, so any index > 0 should return None.
        let result = order.current_output_amount(idx, Utc::now());
        prop_assert!(result.is_none());
    }

    #[test]
    fn order_expired_after_deadline(order in arb_dutch_auction_order()) {
        let after = order.deadline + Duration::seconds(1);
        prop_assert!(order.is_expired(after));
    }

    #[test]
    fn order_not_expired_before_deadline(order in arb_dutch_auction_order()) {
        let before = order.deadline - Duration::seconds(1);
        prop_assert!(!order.is_expired(before));
    }
}

// ─── DutchAuctionOrder Serde Roundtrip ──────────────────────────────

proptest! {
    #[test]
    fn dutch_auction_order_serde_roundtrip(order in arb_dutch_auction_order()) {
        let json = serde_json::to_string(&order).unwrap();
        let deserialized: DutchAuctionOrder = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(order, deserialized);
    }
}
