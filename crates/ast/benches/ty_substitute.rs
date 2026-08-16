use std::collections::HashMap;

use ast::{Ty, UnionVariant};
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use internment::Intern;

const DEPTH: usize = 8;
const WIDTH: usize = 6;

fn name(value: &str) -> Intern<String> {
    Intern::from_ref(value)
}

fn nested_type() -> Ty {
    let mut ty = Ty::Opaque(name("T"));

    for depth in 0..DEPTH {
        let fields = (0..WIDTH)
            .map(|field| {
                (
                    Intern::new(format!("field_{depth}_{field}")),
                    Box::new(if field == 0 {
                        ty.clone()
                    } else {
                        Ty::Opaque(name("Other"))
                    }),
                )
            })
            .collect();

        ty = if depth % 2 == 0 {
            Ty::Record {
                name: Intern::new(format!("Record{depth}")),
                fields,
                resolved_params: None,
            }
        } else {
            Ty::Union {
                name: Intern::new(format!("Union{depth}")),
                variants: vec![UnionVariant::new(
                    Intern::new(format!("Variant{depth}")),
                    fields,
                )],
                literal_values: None,
                resolved_params: None,
            }
        };
    }

    ty
}

fn bench_substitute(c: &mut Criterion) {
    let ty = nested_type();
    let cases = [
        ("empty", HashMap::new()),
        ("no_hit", HashMap::from([(name("Missing"), Ty::i64())])),
        ("hit", HashMap::from([(name("T"), Ty::i64())])),
    ];

    let mut group = c.benchmark_group("ty_substitute");
    group.throughput(Throughput::Elements((DEPTH * WIDTH) as u64));

    for (label, substitutions) in &cases {
        group.bench_with_input(
            BenchmarkId::new(format!("depth_{DEPTH}_width_{WIDTH}"), label),
            substitutions,
            |b, substitutions| b.iter(|| black_box(ty.substitute(black_box(substitutions)))),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_substitute);
criterion_main!(benches);
