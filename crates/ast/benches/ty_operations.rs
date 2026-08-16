use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ast::{Ty, UnionVariant};
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use internment::Intern;

fn name(value: &str) -> Intern<String> {
    Intern::from_ref(value)
}

fn leaf(index: usize) -> Ty {
    Ty::Record {
        name: Intern::new(format!("Leaf{index}")),
        fields: vec![
            (name("first"), Box::new(Ty::i64())),
            (name("second"), Box::new(Ty::Opaque(name("T")))),
        ],
        resolved_params: None,
    }
}

fn recursive_type(depth: usize, width: usize) -> Ty {
    let mut ty = leaf(0);
    for level in 1..=depth {
        let fields = (0..width)
            .map(|field| {
                let child = if field == 0 { ty.clone() } else { leaf(field) };
                (
                    Intern::new(format!("field_{level}_{field}")),
                    Box::new(child),
                )
            })
            .collect();
        ty = Ty::Union {
            name: Intern::new(format!("Union{level}")),
            variants: vec![UnionVariant::new(
                Intern::new(format!("Variant{level}")),
                fields,
            )],
            literal_values: None,
            resolved_params: None,
        };
    }
    ty
}

fn hash_ty(ty: &Ty) -> u64 {
    let mut hasher = DefaultHasher::new();
    ty.hash(&mut hasher);
    hasher.finish()
}

fn bench_ty_operations(c: &mut Criterion) {
    let cases = [
        ("scalar", Ty::i64(), 1),
        ("depth_2_width_4", recursive_type(2, 4), 23),
        ("depth_8_width_6", recursive_type(8, 6), 131),
    ];

    for (label, ty, nodes) in cases {
        let equal = ty.clone();
        let mismatch = Ty::Unit;
        let mut group = c.benchmark_group(format!("ty_operations/{label}"));
        group.throughput(Throughput::Elements(nodes));
        group.bench_function(BenchmarkId::new("clone", nodes), |b| {
            b.iter(|| black_box(black_box(&ty).clone()))
        });
        group.bench_function(BenchmarkId::new("eq_equal", nodes), |b| {
            b.iter(|| black_box(black_box(&ty) == black_box(&equal)))
        });
        group.bench_function(BenchmarkId::new("eq_variant_mismatch", nodes), |b| {
            b.iter(|| black_box(black_box(&ty) == black_box(&mismatch)))
        });
        group.bench_function(BenchmarkId::new("hash", nodes), |b| {
            b.iter(|| black_box(hash_ty(black_box(&ty))))
        });
        group.finish();
    }
}

criterion_group!(benches, bench_ty_operations);
criterion_main!(benches);
