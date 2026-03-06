//! Benchmarks for the P&L tracker.

use alloy::primitives::{address, B256, U256};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use phantom_inventory::pnl::{FillRecord, FillStatus, PnlTracker};

fn make_fill(i: u64) -> FillRecord {
    FillRecord {
        fill_id: format!("bench-fill-{i}"),
        chain_id: if i.is_multiple_of(2) { 1 } else { 42161 },
        token_in: address!("0000000000000000000000000000000000000001"),
        token_out: address!("0000000000000000000000000000000000000002"),
        amount_in: U256::from(1_000_000_000_000_000_000u64),
        amount_out: U256::from(2_000_000_000u64),
        gas_cost_wei: 50_000_000_000_000u128,
        pnl_wei: 100_000_000_000_000i128,
        tx_hash: B256::from([i as u8; 32]),
        timestamp: 1_700_000_000 + i,
        status: FillStatus::Confirmed,
    }
}

fn bench_record_fill(c: &mut Criterion) {
    c.bench_function("pnl_record_fill", |b| {
        b.iter_batched(
            || (PnlTracker::with_defaults(), make_fill(1)),
            |(tracker, fill)| {
                tracker.record_fill(fill);
                black_box(());
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_summary_small(c: &mut Criterion) {
    let tracker = PnlTracker::with_defaults();
    for i in 0..10 {
        tracker.record_fill(make_fill(i));
    }

    c.bench_function("pnl_summary_10_fills", |b| {
        b.iter(|| {
            black_box(tracker.summary());
        })
    });
}

fn bench_summary_large(c: &mut Criterion) {
    let tracker = PnlTracker::with_defaults();
    for i in 0..1000 {
        tracker.record_fill(make_fill(i));
    }

    c.bench_function("pnl_summary_1000_fills", |b| {
        b.iter(|| {
            black_box(tracker.summary());
        })
    });
}

fn bench_daily_pnl(c: &mut Criterion) {
    let tracker = PnlTracker::with_defaults();
    for i in 0..500 {
        tracker.record_fill(make_fill(i));
    }

    c.bench_function("pnl_all_daily_500_fills", |b| {
        b.iter(|| {
            black_box(tracker.all_daily_pnl());
        })
    });
}

fn bench_token_pnl(c: &mut Criterion) {
    let tracker = PnlTracker::with_defaults();
    for i in 0..500 {
        tracker.record_fill(make_fill(i));
    }

    c.bench_function("pnl_all_token_500_fills", |b| {
        b.iter(|| {
            black_box(tracker.all_token_pnl());
        })
    });
}

fn bench_record_100_fills(c: &mut Criterion) {
    c.bench_function("pnl_record_100_fills", |b| {
        b.iter_batched(
            || {
                let fills: Vec<_> = (0..100).map(make_fill).collect();
                (PnlTracker::with_defaults(), fills)
            },
            |(tracker, fills)| {
                for fill in fills {
                    tracker.record_fill(fill);
                }
                black_box(());
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    benches,
    bench_record_fill,
    bench_summary_small,
    bench_summary_large,
    bench_daily_pnl,
    bench_token_pnl,
    bench_record_100_fills,
);
criterion_main!(benches);
