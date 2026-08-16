use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use parser::cursor::TokenCursor;

fn qualification_fixture(def_count: usize, calls_per_def: usize) -> ast::FileAst {
    let mut source = String::with_capacity(def_count * calls_per_def * 12);

    for definition in 0..def_count {
        source.push_str("def_");
        source.push_str(&definition.to_string());
        source.push_str(": (");
        for call in 0..calls_per_def {
            if call > 0 {
                source.push_str(", ");
            }
            source.push_str("def_");
            source.push_str(&(call % def_count).to_string());
            source.push_str("()");
        }
        source.push_str(")\n");
    }

    let ast = TokenCursor::parse_source(&source);
    assert_eq!(ast.defs.len(), def_count);
    ast
}

fn bench_module_qualify(c: &mut Criterion) {
    let fixture = qualification_fixture(100, 100);
    let qualifiers = [
        ("depth_1", "pkg"),
        ("depth_3", "pkg.module.feature"),
        ("depth_8", "pkg.a.b.c.d.e.f.feature"),
    ];

    let mut group = c.benchmark_group("module_qualify");
    group.throughput(Throughput::Elements(10_000));

    for (label, qualifier) in qualifiers {
        group.bench_with_input(
            BenchmarkId::new("100_defs_10k_calls", label),
            qualifier,
            |b, qualifier| {
                b.iter_batched(
                    || fixture.clone(),
                    |ast| black_box(ast.qualify_module_defs(black_box(qualifier))),
                    BatchSize::SmallInput,
                )
            },
        );
    }

    group.finish();

    let mut group = c.benchmark_group("module_qualify_shape");
    for (label, def_count, calls_per_def) in [
        ("1_def_10k_calls", 1, 10_000),
        ("100_defs_100_calls", 100, 100),
        ("1k_defs_1_call", 1_000, 1),
    ] {
        let fixture = qualification_fixture(def_count, calls_per_def);
        group.throughput(Throughput::Elements((def_count * calls_per_def) as u64));
        group.bench_function(label, |b| {
            b.iter_batched(
                || fixture.clone(),
                |ast| black_box(ast.qualify_module_defs(black_box("pkg.module.feature"))),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_module_qualify);
criterion_main!(benches);
