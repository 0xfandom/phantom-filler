//! Property-based tests for the order book.

use alloy::primitives::{address, U256};
use chrono::{Duration, Utc};
use phantom_common::types::{
    ChainId, DutchAuctionOrder, OrderId, OrderInput, OrderOutput, OrderStatus,
};
use phantom_discovery::orderbook::OrderBook;
use proptest::prelude::*;

fn make_order(i: u64) -> DutchAuctionOrder {
    let now = Utc::now();
    let mut id_bytes = [0u8; 32];
    id_bytes[..8].copy_from_slice(&i.to_le_bytes());
    DutchAuctionOrder {
        id: OrderId::new(id_bytes.into()),
        chain_id: ChainId::Ethereum,
        reactor: address!("0000000000000000000000000000000000000099"),
        signer: address!("0000000000000000000000000000000000000010"),
        nonce: U256::from(i),
        decay_start_time: now,
        decay_end_time: now + Duration::minutes(60),
        deadline: now + Duration::minutes(120),
        input: OrderInput {
            token: address!("0000000000000000000000000000000000000001"),
            amount: U256::from(1_000_000_000_000_000_000u64),
        },
        outputs: vec![OrderOutput {
            token: address!("0000000000000000000000000000000000000002"),
            start_amount: U256::from(2_000_000_000u64),
            end_amount: U256::from(1_900_000_000u64),
            recipient: address!("0000000000000000000000000000000000000010"),
        }],
    }
}

proptest! {
    #[test]
    fn orderbook_count_matches_inserts(n in 1u64..100) {
        let book = OrderBook::new();
        for i in 0..n {
            book.insert(make_order(i)).unwrap();
        }
        prop_assert_eq!(book.order_count(), n as usize);
    }

    #[test]
    fn orderbook_all_start_as_pending(n in 1u64..50) {
        let book = OrderBook::new();
        for i in 0..n {
            book.insert(make_order(i)).unwrap();
        }
        prop_assert_eq!(book.pending_count(), n as usize);
        prop_assert_eq!(book.active_count(), 0);
    }

    #[test]
    fn orderbook_active_count_matches_activations(total in 2u64..50, active_frac in 0.0f64..1.0) {
        let book = OrderBook::new();
        let n_active = ((total as f64) * active_frac) as u64;

        for i in 0..total {
            book.insert(make_order(i)).unwrap();
        }
        for i in 0..n_active {
            book.activate(&make_order(i).id).unwrap();
        }

        prop_assert_eq!(book.active_count(), n_active as usize);
        prop_assert_eq!(book.pending_count(), (total - n_active) as usize);
    }

    #[test]
    fn orderbook_get_returns_inserted_order(i in 0u64..1000) {
        let book = OrderBook::new();
        let order = make_order(i);
        let id = order.id;
        book.insert(order).unwrap();

        let entry = book.get(&id);
        prop_assert!(entry.is_some());
        prop_assert_eq!(entry.unwrap().status, OrderStatus::Pending);
    }

    #[test]
    fn orderbook_remove_decreases_count(n in 2u64..50, remove_idx in 0u64..50) {
        let n = n.max(2);
        let remove_idx = remove_idx % n;
        let book = OrderBook::new();

        for i in 0..n {
            book.insert(make_order(i)).unwrap();
        }
        let id = make_order(remove_idx).id;
        book.remove(&id);

        prop_assert_eq!(book.order_count(), (n - 1) as usize);
        prop_assert!(book.get(&id).is_none());
    }

    #[test]
    fn orderbook_duplicate_insert_fails(i in 0u64..1000) {
        let book = OrderBook::new();
        let order = make_order(i);
        book.insert(order.clone()).unwrap();
        let result = book.insert(order);
        prop_assert!(result.is_err());
    }

    #[test]
    fn orderbook_lifecycle_pending_to_filled(i in 0u64..1000) {
        let book = OrderBook::new();
        let order = make_order(i);
        let id = order.id;

        book.insert(order).unwrap();
        prop_assert_eq!(book.get(&id).unwrap().status, OrderStatus::Pending);

        book.activate(&id).unwrap();
        prop_assert_eq!(book.get(&id).unwrap().status, OrderStatus::Active);

        book.mark_filled(&id).unwrap();
        prop_assert_eq!(book.get(&id).unwrap().status, OrderStatus::Filled);
        prop_assert_eq!(book.active_count(), 0);
    }

    #[test]
    fn orderbook_lifecycle_pending_to_expired(i in 0u64..1000) {
        let book = OrderBook::new();
        let order = make_order(i);
        let id = order.id;

        book.insert(order).unwrap();
        book.activate(&id).unwrap();
        book.mark_expired(&id).unwrap();

        prop_assert_eq!(book.get(&id).unwrap().status, OrderStatus::Expired);
        prop_assert_eq!(book.active_count(), 0);
    }

    #[test]
    fn orderbook_get_by_status_matches_count(
        total in 2u64..30,
        n_active in 0u64..30,
    ) {
        let book = OrderBook::new();
        let n_active = n_active.min(total);

        for i in 0..total {
            book.insert(make_order(i)).unwrap();
        }
        for i in 0..n_active {
            book.activate(&make_order(i).id).unwrap();
        }

        let active_orders = book.get_by_status(OrderStatus::Active);
        let pending_orders = book.get_by_status(OrderStatus::Pending);

        prop_assert_eq!(active_orders.len(), n_active as usize);
        prop_assert_eq!(pending_orders.len(), (total - n_active) as usize);
    }
}
