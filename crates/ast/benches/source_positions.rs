//! Baseline microbenchmarks for current `SourceExt` position conversion.
//!
//! Measures the linear-scan cost of `byte_offset_to_position` and
//! `position_to_byte_offset` at representative file sizes.
//!
//! Run:  cargo bench -p ast --bench source_positions
//! Save: cargo bench -p ast --bench source_positions -- --save-baseline before-line-index

use ast::source::LineIndex;
use ast::source::SourceExt;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

/// Synthetic Gin-like source — ~7 KB to exercise position conversions.
/// Built once via `std::sync::OnceLock`.
fn synthetic_gin() -> &'static str {
    use std::sync::OnceLock;
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| {
        let mut s = String::with_capacity(7_000);
        for i in 0..200 {
            s.push_str("register_");
            s.push_str(&i.to_string());
            s.push_str(" has value Str\n\n");
            s.push_str("SyscallSpec_");
            s.push_str(&i.to_string());
            s.push_str(" has template Str, constraints Str\n\n");
        }
        // Ensure at least 7KB
        while s.len() < 7_000 {
            s.push_str("Padding_");
            s.push_str(&s.len().to_string());
            s.push_str(" is Unit\n");
        }
        s
    })
    .as_str()
}

fn make_gin(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 20);
    for i in 0..lines {
        s.push_str("decl_");
        s.push_str(&i.to_string());
        s.push_str(" is Unit\n");
    }
    s
}

fn make_unicode_gin(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 32);
    for i in 0..lines {
        s.push_str("déclaration_");
        s.push_str(&i.to_string());
        s.push_str(" is '😀💯'\n");
    }
    s
}

fn make_sources() -> Vec<(&'static str, String)> {
    vec![
        ("synthetic_7k", synthetic_gin().to_string()),
        ("synth_100ln", make_gin(100)),
        ("synth_1k", make_gin(1_000)),
        ("synth_10k", make_gin(10_000)),
        ("unicode_1k", make_unicode_gin(1_000)),
    ]
}

fn bench_byte_to_position(c: &mut Criterion) {
    let mut group = c.benchmark_group("byte_offset_to_position");
    let cases = make_sources();

    for (label, source) in &cases {
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));

        // Start of file (cheap — barely any scanning).
        group.bench_with_input(format!("{label}_start"), source.as_str(), |b, src| {
            b.iter(|| black_box(src.byte_offset_to_position(black_box(0))))
        });

        // End of file (worst case — linear scan from the beginning).
        let end = source.len().saturating_sub(1);
        group.bench_with_input(format!("{label}_end"), source.as_str(), |b, src| {
            b.iter(|| black_box(src.byte_offset_to_position(black_box(end))))
        });
    }

    group.finish();
}

fn bench_position_to_byte(c: &mut Criterion) {
    let mut group = c.benchmark_group("position_to_byte_offset");
    let cases = make_sources();

    for (label, source) in &cases {
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));

        // Last line, column 0 (worst case).
        let end_line = source.matches('\n').count() as u32;
        group.bench_with_input(format!("{label}_end"), source.as_str(), |b, src| {
            b.iter(|| black_box(src.position_to_byte_offset(black_box(end_line), black_box(0))))
        });
    }

    group.finish();
}

fn bench_compute_line_starts(c: &mut Criterion) {
    let mut group = c.benchmark_group("compute_line_starts");
    let cases = make_sources();

    for (label, source) in &cases {
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_with_input(*label, source.as_str(), |b, src| {
            b.iter(|| black_box(src.compute_line_starts()))
        });
    }

    group.finish();
}

fn bench_repeated_spans(c: &mut Criterion) {
    let source = make_gin(10_000);
    let len = source.len().max(2);
    let spans: Vec<(usize, usize)> = (0..500)
        .map(|i| {
            let start = i * len / 500;
            let end = (start + 5).min(len - 1);
            (start, end)
        })
        .collect();

    let mut group = c.benchmark_group("repeated_spans");
    group.throughput(criterion::Throughput::Elements(500));

    group.bench_function("synth_10k__500_spans", |b| {
        b.iter(|| {
            for &(start, end) in &spans {
                let (l1, c1) = source.byte_offset_to_position(black_box(start));
                let (l2, c2) = source.byte_offset_to_position(black_box(end));
                black_box((l1, c1, l2, c2));
            }
        })
    });

    group.finish();
}

fn bench_line_index_new(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index_new");
    let cases = make_sources();

    for (label, source) in &cases {
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_with_input(*label, source.as_str(), |b, src| {
            b.iter(|| black_box(LineIndex::new(src)))
        });
    }

    group.finish();
}

fn bench_line_index_byte_to_position(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index_byte_to_position");
    let cases = make_sources();

    for (label, source) in &cases {
        let idx = LineIndex::new(source.as_str());
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));

        group.bench_with_input(format!("{label}_start"), &idx, |b, index| {
            b.iter(|| black_box(index.byte_to_position(black_box(source.as_str()), 0)))
        });

        let end = source.len().saturating_sub(1);
        group.bench_with_input(format!("{label}_end"), &idx, |b, index| {
            b.iter(|| black_box(index.byte_to_position(black_box(source.as_str()), black_box(end))))
        });
    }

    group.finish();
}

fn bench_line_index_position_to_byte(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index_position_to_byte");
    let cases = make_sources();

    for (label, source) in &cases {
        let idx = LineIndex::new(source.as_str());
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));

        let end_line = source.matches('\n').count() as u32;
        group.bench_with_input(format!("{label}_end"), &idx, |b, index| {
            b.iter(|| {
                black_box(index.position_to_byte(
                    black_box(source.as_str()),
                    black_box(end_line),
                    black_box(0),
                ))
            })
        });
    }

    group.finish();
}

fn bench_line_index_repeated_spans(c: &mut Criterion) {
    let source = make_gin(10_000);
    let idx = LineIndex::new(&source);
    let len = source.len().max(2);
    let spans: Vec<(usize, usize)> = (0..500)
        .map(|i| {
            let start = i * len / 500;
            let end = (start + 5).min(len - 1);
            (start, end)
        })
        .collect();

    let mut group = c.benchmark_group("line_index_repeated_spans");
    group.throughput(criterion::Throughput::Elements(500));

    group.bench_function("synth_10k__500_spans", |b| {
        b.iter(|| {
            for &(start, end) in &spans {
                let (l1, c1) = idx.byte_to_position(black_box(&source), black_box(start));
                let (l2, c2) = idx.byte_to_position(black_box(&source), black_box(end));
                black_box((l1, c1, l2, c2));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_byte_to_position,
    bench_position_to_byte,
    bench_compute_line_starts,
    bench_repeated_spans,
    bench_line_index_new,
    bench_line_index_byte_to_position,
    bench_line_index_position_to_byte,
    bench_line_index_repeated_spans,
);
criterion_main!(benches);
