//! CRUD crate 基准测试
//!
//! 测试核心类型和函数的性能：
//! - Filter 验证（SQL 注入防护）
//! - Sort SQL 生成
//! - PaginatedResponse 构建

use criterion::{criterion_group, criterion_main, Criterion};
use crud::filter::Filter as CrudFilter;
use crud::pagination::PaginatedResponse;
use std::hint::black_box;

// ═══════════════════════════════════════════════════════════════════════════════
// Filter 验证基准
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_filter_validate_valid(c: &mut Criterion) {
    let filter = CrudFilter {
        field: "notice".to_string(),
        op: "eq".to_string(),
        value: "test-value".to_string(),
    };

    c.bench_function("filter_validate_valid", |b| {
        b.iter(|| {
            let result = black_box(&filter).validate();
            assert!(result.is_ok());
        })
    });
}

fn bench_filter_validate_invalid_op(c: &mut Criterion) {
    let filter = CrudFilter {
        field: "notice".to_string(),
        op: "invalid_operator_xyz".to_string(),
        value: "test".to_string(),
    };

    c.bench_function("filter_validate_invalid_op", |b| {
        b.iter(|| {
            let result = black_box(&filter).validate();
            assert!(result.is_err());
        })
    });
}

fn bench_filter_validate_sql_injection(c: &mut Criterion) {
    let filter = CrudFilter {
        field: "notice".to_string(),
        op: "eq".to_string(),
        value: "'; DROP TABLE users; --".to_string(),
    };

    c.bench_function("filter_validate_sql_injection_value", |b| {
        b.iter(|| {
            let result = black_box(&filter).validate();
            assert!(result.is_ok()); // value itself is not validated for SQL
        })
    });
}

fn bench_filter_validate_field_injection(c: &mut Criterion) {
    let filter = CrudFilter {
        field: "notice; DROP TABLE users".to_string(),
        op: "eq".to_string(),
        value: "test".to_string(),
    };

    c.bench_function("filter_validate_field_injection", |b| {
        b.iter(|| {
            let result = black_box(&filter).validate();
            assert!(result.is_err());
        })
    });
}

fn bench_filter_to_sql(c: &mut Criterion) {
    let filter = CrudFilter {
        field: "notice".to_string(),
        op: "like".to_string(),
        value: "%search-term%".to_string(),
    };

    c.bench_function("filter_to_sql_like", |b| {
        b.iter(|| {
            let result = black_box(&filter).to_sql(black_box(1));
            assert!(result.is_some());
        })
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// PaginatedResponse 构建基准
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_paginated_response_small(c: &mut Criterion) {
    let items: Vec<i64> = (1..=20).collect();
    c.bench_function("paginated_response_20_items", |b| {
        b.iter(|| {
            let resp = PaginatedResponse::new(black_box(items.clone()), 100, 1, 20);
            assert_eq!(resp.items.len(), 20);
        })
    });
}

fn bench_paginated_response_large(c: &mut Criterion) {
    let items: Vec<i64> = (1..=1000).collect();
    c.bench_function("paginated_response_1000_items", |b| {
        b.iter(|| {
            let resp = PaginatedResponse::new(black_box(items.clone()), 10000, 1, 1000);
            assert_eq!(resp.items.len(), 1000);
        })
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// Criterion group & main
// ═══════════════════════════════════════════════════════════════════════════════

criterion_group!(
    name = filter_benches;
    config = Criterion::default().sample_size(100);
    targets =
        bench_filter_validate_valid,
        bench_filter_validate_invalid_op,
        bench_filter_validate_sql_injection,
        bench_filter_validate_field_injection,
        bench_filter_to_sql,
        bench_paginated_response_small,
        bench_paginated_response_large,
);

criterion_main!(filter_benches);
