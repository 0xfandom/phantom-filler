//! Benchmarks for the transaction builder.

use alloy::primitives::{address, Bytes, U256};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use phantom_execution::builder::{TransactionBuilder, TransactionParams};

fn sample_params() -> TransactionParams {
    TransactionParams {
        from: address!("0000000000000000000000000000000000000001"),
        to: address!("0000000000000000000000000000000000000002"),
        calldata: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
        value: U256::ZERO,
        chain_id: 1,
        max_fee_per_gas: 30_000_000_000u128,
        max_priority_fee_per_gas: 1_000_000_000u128,
        gas_limit: 200_000,
        nonce: 42,
    }
}

fn bench_build_transaction(c: &mut Criterion) {
    let builder = TransactionBuilder::with_defaults();
    let params = sample_params();

    c.bench_function("tx_builder_build", |b| {
        b.iter(|| {
            black_box(builder.build(&params).unwrap());
        })
    });
}

fn bench_build_with_value(c: &mut Criterion) {
    let builder = TransactionBuilder::with_defaults();
    let mut params = sample_params();
    params.value = U256::from(1_000_000_000_000_000_000u64);

    c.bench_function("tx_builder_build_with_value", |b| {
        b.iter(|| {
            black_box(builder.build(&params).unwrap());
        })
    });
}

fn bench_build_large_calldata(c: &mut Criterion) {
    let builder = TransactionBuilder::with_defaults();
    let mut params = sample_params();
    params.calldata = Bytes::from(vec![0xAA; 4096]);

    c.bench_function("tx_builder_build_4kb_calldata", |b| {
        b.iter(|| {
            black_box(builder.build(&params).unwrap());
        })
    });
}

fn bench_build_100_transactions(c: &mut Criterion) {
    let builder = TransactionBuilder::with_defaults();

    c.bench_function("tx_builder_build_100", |b| {
        b.iter(|| {
            for nonce in 0..100u64 {
                let mut params = sample_params();
                params.nonce = nonce;
                black_box(builder.build(&params).unwrap());
            }
        })
    });
}

criterion_group!(
    benches,
    bench_build_transaction,
    bench_build_with_value,
    bench_build_large_calldata,
    bench_build_100_transactions,
);
criterion_main!(benches);
