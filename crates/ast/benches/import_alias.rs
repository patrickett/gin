use ast::{FileAst, ModPath, SpanId, Spanned, SymbolAlias};
use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use internment::Intern;
use parser::query::SourceParseExt;

fn fixture(occurrences: usize, path_depth: usize, alias: &str) -> FileAst {
    let mut source = String::from("main:\n");
    for _ in 0..occurrences {
        source.push_str("    imported_symbol\n");
    }
    source.push_str("return\n");

    let mut ast = source.parse_source_full().ast;
    let segments = (0..path_depth)
        .map(|index| Intern::new(format!("segment_{index}")))
        .collect();
    ast.symbol_aliases.push(SymbolAlias {
        alias: Intern::from_ref(alias),
        target: Spanned::new(
            ModPath::new(Intern::from_ref("package"), segments),
            SpanId::INVALID,
        ),
    });
    ast
}

fn bench_import_alias(c: &mut Criterion) {
    let mut group = c.benchmark_group("import_alias");

    for occurrences in [10, 100, 1_000] {
        group.throughput(Throughput::Elements(occurrences as u64));

        for path_depth in [1, 4, 16] {
            for (lookup, alias) in [("hit", "imported_symbol"), ("no_hit", "missing_symbol")] {
                let template = fixture(occurrences, path_depth, alias);
                group.bench_with_input(
                    BenchmarkId::new(
                        format!("occurrences_{occurrences}_{lookup}"),
                        format!("path_depth_{path_depth}"),
                    ),
                    &template,
                    |b, template| {
                        b.iter_batched(
                            || template.clone(),
                            |mut ast| {
                                ast.apply_symbol_aliases();
                                black_box(ast)
                            },
                            BatchSize::SmallInput,
                        )
                    },
                );
            }
        }
    }

    group.finish();
}

criterion_group!(benches, bench_import_alias);
criterion_main!(benches);
