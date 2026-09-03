//! Runtime Engine 基准测试
//!
//! 测试核心行为引擎的性能：
//! - State 构建与配置
//! - StateMachine 构建
//! - parse_states 批量解析

use criterion::{criterion_group, criterion_main, Criterion};
use runtime_contract::behavior::{parse_states, State, StateMachine};
use std::hint::black_box;

fn bench_state_new(c: &mut Criterion) {
    c.bench_function("state_new", |b| {
        b.iter(|| {
            let state = State::new(black_box("Pending"));
            black_box(state);
        })
    });
}

fn bench_state_full_builder(c: &mut Criterion) {
    c.bench_function("state_full_builder", |b| {
        b.iter(|| {
            let state = State::new("Processing")
                .with_description("Order is being processed")
                .as_final();
            black_box(state);
        })
    });
}

fn bench_statemachine_new(c: &mut Criterion) {
    c.bench_function("statemachine_new", |b| {
        b.iter(|| {
            let sm = StateMachine::default();
            black_box(sm);
        })
    });
}

fn bench_parse_states_small(c: &mut Criterion) {
    let params: Vec<String> = vec![
        "Pending".into(),
        "Processing".into(),
        "Shipping".into(),
        "Delivered".into(),
        "Completed".into(),
    ];

    c.bench_function("parse_states_5", |b| {
        b.iter(|| {
            let states = parse_states(black_box(&params));
            black_box(states);
        })
    });
}

fn bench_parse_states_large(c: &mut Criterion) {
    let params: Vec<String> = (0..50).map(|i| format!("State{:02}", i)).collect();

    c.bench_function("parse_states_50", |b| {
        b.iter(|| {
            let states = parse_states(black_box(&params));
            black_box(states);
        })
    });
}

fn bench_statemachine_with_states(c: &mut Criterion) {
    c.bench_function("statemachine_with_20_states", |b| {
        b.iter(|| {
            let sm = StateMachine {
                enabled: true,
                states: (0..20)
                    .map(|i| State::new(format!("State{:02}", i)))
                    .collect(),
                initial_state: Some("State00".into()),
                state_field: Some("status".into()),
            };
            black_box(sm);
        })
    });
}

criterion_group!(
    name = runtime_benches;
    config = Criterion::default().sample_size(100);
    targets =
        bench_state_new,
        bench_state_full_builder,
        bench_statemachine_new,
        bench_parse_states_small,
        bench_parse_states_large,
        bench_statemachine_with_states,
);

criterion_main!(runtime_benches);
