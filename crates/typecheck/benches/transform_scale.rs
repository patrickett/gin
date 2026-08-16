use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use parser::parse_from_str;
use std::hint::black_box;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

fn registry_source(file: usize, declarations: usize) -> String {
    (0..declarations)
        .map(|index| format!("Type{file}_{index} is in 0...255\n"))
        .collect()
}

fn operator_source(expressions: usize) -> String {
    let mut source = String::from(
        "Word is in 0...255\n#intrinsic(BitsAdd)\nadd_bits(a Word, b Word) Word extern\n#operator(Add)\n#inline\nword_add(a Word, b Word) Word: add_bits(a, b)\n",
    );
    for index in 0..expressions {
        source.push_str(&format!(
            "apply{index}(a Word, b Word) Word: a + b + a + b\n"
        ));
    }
    source
}

fn alias_source(depth: usize, shared: usize) -> String {
    let mut source = String::from("Base is in 0...255\nAlias0 is Base\n");
    for index in 1..depth {
        source.push_str(&format!("Alias{index} is Alias{}\n", index - 1));
    }
    for index in 0..shared {
        source.push_str(&format!("Shared{index} is Alias{}\n", depth - 1));
    }
    source
}

fn benchmark_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("package_type_registry");
    for (files, declarations) in [(8, 32), (32, 32), (32, 128)] {
        let inputs: Vec<_> = (0..files)
            .map(|file| parse_from_str(&registry_source(file, declarations)))
            .collect();
        group.bench_with_input(
            BenchmarkId::new("files_x_declarations", format!("{files}x{declarations}")),
            &inputs,
            |bencher, inputs| {
                bencher.iter(|| {
                    black_box(transform_package_with_shared_context(
                        inputs.clone(),
                        PackageTransformOptions::FULL,
                    ))
                });
            },
        );
    }
    group.finish();
}

fn benchmark_operators(c: &mut Criterion) {
    let mut group = c.benchmark_group("operator_expression_lowering");
    for expressions in [100, 250] {
        let input = vec![parse_from_str(&operator_source(expressions))];
        group.bench_with_input(
            BenchmarkId::from_parameter(expressions),
            &input,
            |bencher, input| {
                bencher.iter(|| {
                    black_box(transform_package_with_shared_context(
                        input.clone(),
                        PackageTransformOptions::FULL,
                    ))
                });
            },
        );
    }
    group.finish();
}

fn benchmark_alias_graphs(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_and_shared_alias_graphs");
    for (depth, shared) in [(32, 64), (64, 256)] {
        let input = vec![parse_from_str(&alias_source(depth, shared))];
        group.bench_with_input(
            BenchmarkId::new("depth_x_shared", format!("{depth}x{shared}")),
            &input,
            |bencher, input| {
                bencher.iter(|| {
                    black_box(transform_package_with_shared_context(
                        input.clone(),
                        PackageTransformOptions::FULL,
                    ))
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    benchmark_registry,
    benchmark_operators,
    benchmark_alias_graphs
);
criterion_main!(benches);
