use codegen::{
    CodegenContext, CodegenSourceMap, ContextExt, NativeCompiler, ScopedSymbolTable, Slot,
};
use criterion::{Criterion, criterion_group, criterion_main};
use internment::Intern;
use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Value};
use parser::parse_from_str;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn codegen_source() -> String {
    let mut source = String::from(test_fixtures::preludes::BYTE_PRELUDE);
    for i in 0..15 {
        source.push_str(&format!(
            "function_{i}(x Byte) Byte:\n    y: x\n    z: y\n    w: when x is 0 then z else y\n    return w\n"
        ));
    }
    source
}

fn bench_codegen(c: &mut Criterion) {
    let source = codegen_source();
    let parsed = parse_from_str(&source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut transform_context = TransformCtx::new();
    transform_context.compile_target = target.clone();
    let typed = transform(&parsed, FileId(0), &transform_context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let context = NativeCompiler::create_context();
    let build = |context| {
        CodegenContext::build_module_from_resolved_program_for_target(
            context,
            typed.resolved_program(),
            CodegenSourceMap::new("bench.gin", &source, &typed.span_table),
            &target,
        )
    };
    let (module, diagnostics) = build(&context);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let module = module.expect("benchmark must lower a module");
    assert!(module.as_operation().verify());
    let function_count = std::iter::successors(module.body().first_operation(), |operation| {
        operation.next_in_block()
    })
    .count();
    assert!(function_count >= 15);
    drop(module);

    let mut group = c.benchmark_group("codegen");
    group.throughput(criterion::Throughput::Bytes(source.len() as u64));
    group.sample_size(20);

    group.bench_function("build_module/codegen_source", |b| {
        b.iter(|| {
            let context = NativeCompiler::create_context();
            std::hint::black_box(
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("bench.gin", &source, &typed.span_table),
                    &target,
                ),
            );
        });
    });
    group.bench_function("lower_module/codegen_source", |b| {
        b.iter(|| std::hint::black_box(build(&context)));
    });
    group.finish();
}

fn bench_scope_symbols(c: &mut Criterion) {
    let context = NativeCompiler::create_context();
    let block = Block::new(&[(context.i64(), Location::unknown(&context))]);
    let value: Value = block.argument(0).unwrap().into();
    let names = (0..64)
        .map(|i| Intern::<String>::new(format!("local_{i}")))
        .collect::<Vec<_>>();
    let mut symbols = ScopedSymbolTable::new();
    for name in &names {
        symbols.insert(*name, value, typecheck::ty::Ty::i64(), false);
    }
    symbols.push_scope();
    for name in &names[..16] {
        symbols.insert(*name, value, typecheck::ty::Ty::i64(), false);
    }

    let mut group = c.benchmark_group("scope_symbols");
    group.sample_size(20);
    group.bench_function("clone", |b| {
        b.iter(|| std::hint::black_box(symbols.clone()));
    });
    group.bench_function("assign", |b| {
        b.iter(|| {
            for name in &names {
                let assigned = symbols.assign(
                    name,
                    Slot {
                        value,
                        ty: typecheck::ty::Ty::i64(),
                        is_slot: false,
                    },
                );
                assert!(assigned);
            }
            std::hint::black_box(&symbols);
        });
    });
    group.finish();
}

fn type_mapping_source(distinct_records: bool) -> String {
    let mut source = String::from(format!(
        "{}Word is in 0 to 18446744073709551615\nWord has\n    finish(self):\n        return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    let record_count = if distinct_records { 128 } else { 1 };
    for index in 0..record_count {
        source.push_str(&format!(
            "Record{index} has\n    first Byte\n    second Word\n    finish(self):\n        return\n"
        ));
    }
    for index in 0..128 {
        let record = index % record_count;
        let length = if distinct_records { index % 16 + 1 } else { 16 };
        source.push_str(&format!(
            "identity_{index}(value Record{record}) Record{record}: return value\nsecond_{index}(value ref Record{record}) Word: return value.second\narray_{index}(value ref [Byte; {length}], index Byte) Byte: return value.[index]\n"
        ));
    }
    source
}

fn bench_type_mapping(c: &mut Criterion) {
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut group = c.benchmark_group("codegen_types");
    group.sample_size(20);
    for (name, distinct) in [("repeated_records", false), ("distinct_records", true)] {
        let source = type_mapping_source(distinct);
        let parsed = parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let context = NativeCompiler::create_context();
        let build = || {
            CodegenContext::build_module_from_resolved_program_for_target(
                &context,
                typed.resolved_program(),
                CodegenSourceMap::new("types.gin", &source, &typed.span_table),
                &target,
            )
        };
        let (module, diagnostics) = build();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let module = module.expect("type-mapping workload must lower a module");
        assert!(module.as_operation().verify());
        let count =
            std::iter::successors(module.body().first_operation(), |op| op.next_in_block()).count();
        assert!(count >= 384);
        drop(module);
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_function(name, |b| b.iter(|| std::hint::black_box(build())));
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_codegen,
    bench_scope_symbols,
    bench_type_mapping
);
criterion_main!(benches);
