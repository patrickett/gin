use criterion::{Criterion, criterion_group, criterion_main};
use parser::parse_source_full;
use typecheck::transform::transform_file;
use typed_ast::FileId;

fn codegen_source() -> String {
    let mut s = String::with_capacity(4096);
    // Functions with local mutable variables — exercises var_types HashMap
    for i in 0..15 {
        let c = (b'a' + (i % 26) as u8) as char;
        s.push_str(&format!(
            "fn{c}(x):\n    y: x + {i}\n    z: y * 2\n    w: z + 1\nreturn w\n"
        ));
    }
    s.push('\n');
    // Const binds
    for i in 0..10 {
        s.push_str(&format!("c{i}: {i}\n"));
    }
    s
}

fn bench_codegen(c: &mut Criterion) {
    let source = codegen_source();
    let parsed = parse_source_full(&source);
    let typed = transform_file(parsed.ast, FileId(0));

    let mut group = c.benchmark_group("codegen");
    group.throughput(criterion::Throughput::Bytes(source.len() as u64));
    group.sample_size(20);

    group.bench_function("build_module/codegen_source", |b| {
        b.iter(|| {
            let context = melior::Context::new();
            let result =
                codegen::build_module_from_typed_ast(&context, &typed, &source, "bench.gin", None);
            std::hint::black_box(&result);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_codegen);
criterion_main!(benches);

#[cfg(test)]
mod tests {

    #[test]
    fn test_codegen_source_compiles() {
        let source = codegen_source();
        let parsed = parse_source_full(&source);
        let typed = transform_file(parsed.ast, FileId(0));
        assert!(!typed.defs.is_empty());
        let context = melior::Context::new();
        let (module, _) =
            codegen::build_module_from_typed_ast(&context, &typed, &source, "test.gin", None);
        assert!(module.is_some(), "codegen should succeed");
    }
}
