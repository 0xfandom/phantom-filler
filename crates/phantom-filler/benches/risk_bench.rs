//! Benchmarks for the risk management engine.

use alloy::primitives::U256;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use phantom_inventory::risk::RiskManager;

fn bench_check_fill_pass(c: &mut Criterion) {
    let risk = RiskManager::with_defaults();
    let fill = U256::from(1_000_000_000_000_000_000u64); // 1 ETH

    c.bench_function("risk_check_fill_pass", |b| {
        b.iter(|| {
            black_box(risk.check_fill(fill, U256::ZERO));
        })
    });
}

fn bench_check_fill_reject(c: &mut Criterion) {
    let risk = RiskManager::with_defaults();
    let fill = U256::from(100_000_000_000_000_000_000u128); // 100 ETH (oversized)

    c.bench_function("risk_check_fill_reject", |b| {
        b.iter(|| {
            black_box(risk.check_fill(fill, U256::ZERO));
        })
    });
}

fn bench_risk_snapshot(c: &mut Criterion) {
    let risk = RiskManager::with_defaults();
    // Simulate some activity.
    for _ in 0..10 {
        risk.record_fill_start().ok();
        risk.record_fill_complete(100_000i64);
    }

    c.bench_function("risk_snapshot", |b| {
        b.iter(|| {
            black_box(risk.risk_snapshot());
        })
    });
}

fn bench_record_fill_cycle(c: &mut Criterion) {
    c.bench_function("risk_fill_start_complete_cycle", |b| {
        let risk = RiskManager::with_defaults();
        b.iter(|| {
            risk.record_fill_start().ok();
            risk.record_fill_complete(black_box(50_000i64));
        })
    });
}

criterion_group!(
    benches,
    bench_check_fill_pass,
    bench_check_fill_reject,
    bench_risk_snapshot,
    bench_record_fill_cycle,
);
criterion_main!(benches);
