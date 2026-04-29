use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use parser::parse_source_full;
use typeck::TyEnv;

fn string_heavy_source() -> String {
    let mut s = String::with_capacity(4096);
    // Many string literal binds — exercises str_record_ty()
    for i in 0..50 {
        s.push_str(&format!("str_{i}: 'hello world {i}'\n"));
    }
    s.push('\n');
    // Many format string binds
    for i in 0..50 {
        s.push_str(&format!("fmt_{i}: \"value is ({i})\"\n"));
    }
    s
}

fn many_bindings_source() -> String {
    let mut s = String::with_capacity(4096);
    // A function with many local bindings — exercises locals.clone() and expr_references_name
    s.push_str("compute(a, b):\n");
    for i in 0..30 {
        let c = (b'a' + (i % 26) as u8) as char;
        s.push_str(&format!("    {c}: a + b + {i}\n"));
    }
    s.push_str("return z\n");
    s
}

fn mixed_source() -> String {
    let mut s = String::with_capacity(4096);
    // Unions with variants
    for i in 0..10 {
        s.push_str(&format!("Result{i} is Success{i} or Error{i}\n"));
    }
    s.push('\n');
    // Functions with multi-segment calls (exercises mangled_fn_call_name)
    for i in 0..20 {
        let c = (b'a' + (i % 26) as u8) as char;
        s.push_str(&format!("fn{c}(x): io.println(x)\n"));
    }
    s.push('\n');
    // Some string binds
    for i in 0..20 {
        s.push_str(&format!("msg{i}: 'test {i}'\n"));
    }
    s
}

fn parse_and_typecheck(source: &str) -> TyEnv {
    let output = parse_source_full(source);
    TyEnv::from_file_ast(&output.ast)
}

fn bench_typecheck(c: &mut Criterion) {
    let string_heavy = string_heavy_source();
    let many_bindings = many_bindings_source();
    let mixed = mixed_source();

    let inputs: &[(&str, &str)] = &[
        ("string_heavy", &string_heavy),
        ("many_bindings", &many_bindings),
        ("mixed", &mixed),
    ];

    let mut group = c.benchmark_group("typecheck");
    for (label, source) in inputs {
        let size = source.len();
        group.throughput(criterion::Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("full_pipeline", label),
            source,
            |b, src| {
                b.iter(|| {
                    let ty_env = parse_and_typecheck(src);
                    std::hint::black_box(&ty_env);
                });
            },
        );
    }
    group.finish();
}

fn bench_typecheck_only(c: &mut Criterion) {
    let string_heavy = string_heavy_source();
    let many_bindings = many_bindings_source();
    let mixed = mixed_source();

    let inputs: &[(&str, &str)] = &[
        ("string_heavy", &string_heavy),
        ("many_bindings", &many_bindings),
        ("mixed", &mixed),
    ];

    let mut group = c.benchmark_group("typecheck_only");
    for (label, source) in inputs {
        // Pre-parse so we only measure type-checking
        let output = parse_source_full(source);

        group.throughput(criterion::Throughput::Bytes(source.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("from_file_ast", label),
            &output,
            |b, parsed| {
                b.iter(|| {
                    let ty_env = TyEnv::from_file_ast(&parsed.ast);
                    std::hint::black_box(&ty_env);
                });
            },
        );
    }
    group.finish();
}

fn bench_check_unknowns(c: &mut Criterion) {
    let many_bindings = many_bindings_source();
    let output = parse_source_full(&many_bindings);
    let ty_env = TyEnv::from_file_ast(&output.ast);

    let mut group = c.benchmark_group("check_unknowns");
    group.throughput(criterion::Throughput::Bytes(many_bindings.len() as u64));

    group.bench_function("many_bindings", |b| {
        b.iter(|| {
            let mut symptoms = Vec::new();
            ty_env.check_unknowns(&output.ast, &mut symptoms);
            std::hint::black_box(&symptoms);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_typecheck,
    bench_typecheck_only,
    bench_check_unknowns
);
criterion_main!(benches);

#[cfg(test)]
mod tests {

    #[test]
    fn test_string_heavy_source() {
        let ty_env = parse_and_typecheck(&string_heavy_source());
        assert!(!ty_env.tag_types.is_empty() || !ty_env.fn_return_types.is_empty());
    }

    #[test]
    fn test_many_bindings_source() {
        let ty_env = parse_and_typecheck(&many_bindings_source());
        assert!(!ty_env.fn_return_types.is_empty());
    }

    #[test]
    fn test_mixed_source() {
        let ty_env = parse_and_typecheck(&mixed_source());
        assert!(!ty_env.fn_return_types.is_empty());
    }
}
