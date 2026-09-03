use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use encoding::crc32;
use encoding::rules::{EncodingContext, EncodingRule, EncodingRuleEngine, EncodingSegment};
use encoding::zuid::ZuidGenerator;
use std::hint::black_box;

fn bench_zuid_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("zuid_generation");
    group.throughput(Throughput::Elements(10000));
    let gen = ZuidGenerator::default();
    group.bench_function("generate_u64", |b| {
        b.iter(|| {
            for _ in 0..10000 {
                black_box(gen.generate_u64());
            }
        });
    });
    group.finish();
}

fn bench_crc32_compute(c: &mut Criterion) {
    let payload = vec![0xABu8; 1024];
    let mut group = c.benchmark_group("crc32_compute");
    group.throughput(Throughput::Bytes(1024));
    group.bench_function("1kb_payload", |b| {
        b.iter(|| {
            black_box(crc32::compute_checksum(black_box(&payload)));
        });
    });
    group.finish();
}

fn bench_rule_engine_apply(c: &mut Criterion) {
    let engine = EncodingRuleEngine::new();
    let rule = EncodingRule {
        id: "bench-rule".to_string(),
        name: "Benchmark Rule".to_string(),
        segments: vec![
            EncodingSegment::Prefix {
                value: "ORD".to_string(),
            },
            EncodingSegment::Date {
                format: "%Y%m%d".to_string(),
            },
            EncodingSegment::Literal {
                value: "-X".to_string(),
            },
        ],
        checksum_algorithm: None,
    };
    let ctx = EncodingContext::default();

    c.bench_function("rule_engine_apply_3_segments", |b| {
        b.iter(|| {
            black_box(engine.apply(black_box(&rule), black_box(&ctx)).unwrap());
        });
    });
}

criterion_group!(
    benches,
    bench_zuid_generation,
    bench_crc32_compute,
    bench_rule_engine_apply
);
criterion_main!(benches);
