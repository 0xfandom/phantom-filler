//! Benchmarks for the in-memory order book.

use alloy::primitives::{address, U256};
use chrono::{Duration, Utc};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use phantom_common::types::{
    ChainId, DutchAuctionOrder, OrderId, OrderInput, OrderOutput, OrderStatus,
};
use phantom_discovery::orderbook::OrderBook;

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

fn bench_insert(c: &mut Criterion) {
    c.bench_function("orderbook_insert", |b| {
        b.iter_batched(
            || (OrderBook::new(), make_order(1)),
            |(book, order)| {
                book.insert(order).unwrap();
                black_box(());
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_get(c: &mut Criterion) {
    let book = OrderBook::new();
    let order = make_order(1);
    let id = order.id;
    book.insert(order).unwrap();

    c.bench_function("orderbook_get", |b| {
        b.iter(|| {
            black_box(book.get(&id));
        })
    });
}

fn bench_activate(c: &mut Criterion) {
    c.bench_function("orderbook_activate", |b| {
        b.iter_batched(
            || {
                let book = OrderBook::new();
                let order = make_order(1);
                let id = order.id;
                book.insert(order).unwrap();
                (book, id)
            },
            |(book, id)| {
                book.activate(&id).unwrap();
                black_box(());
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_insert_100_orders(c: &mut Criterion) {
    c.bench_function("orderbook_insert_100", |b| {
        b.iter_batched(
            || {
                let orders: Vec<_> = (0..100).map(make_order).collect();
                (OrderBook::new(), orders)
            },
            |(book, orders)| {
                for order in orders {
                    book.insert(order).unwrap();
                }
                black_box(());
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_get_active_orders(c: &mut Criterion) {
    let book = OrderBook::new();
    for i in 0..100 {
        let order = make_order(i);
        let id = order.id;
        book.insert(order).unwrap();
        book.activate(&id).unwrap();
    }

    c.bench_function("orderbook_get_active_100", |b| {
        b.iter(|| {
            black_box(book.get_active_orders());
        })
    });
}

fn bench_get_by_status(c: &mut Criterion) {
    let book = OrderBook::new();
    for i in 0..200 {
        let order = make_order(i);
        let id = order.id;
        book.insert(order).unwrap();
        if i.is_multiple_of(2) {
            book.activate(&id).unwrap();
        }
    }

    c.bench_function("orderbook_get_by_status_200", |b| {
        b.iter(|| {
            black_box(book.get_by_status(OrderStatus::Active));
        })
    });
}

fn bench_dutch_auction_decay(c: &mut Criterion) {
    let order = make_order(1);
    let midpoint = order.decay_start_time + Duration::minutes(30);

    c.bench_function("dutch_auction_decay_calc", |b| {
        b.iter(|| {
            black_box(order.current_output_amount(0, midpoint));
        })
    });
}

criterion_group!(
    benches,
    bench_insert,
    bench_get,
    bench_activate,
    bench_insert_100_orders,
    bench_get_active_orders,
    bench_get_by_status,
    bench_dutch_auction_decay,
);
criterion_main!(benches);
