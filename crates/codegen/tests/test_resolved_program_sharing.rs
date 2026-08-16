use ast::ConstValue;
use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use internment::Intern;
use melior::ir::operation::{OperationLike, OperationPrintingFlags};
use parser::parse_from_str;
use typecheck::staging::{NormalizeResult, normalize_resolved};
use typecheck::transform::transform_file;
use typecheck::{BindBody, DefId, DepSubst, FileId};

#[test]
fn ctfe_and_codegen_consume_the_same_resolved_program() {
    let source = "answer: 42\n";
    let typed = transform_file(parse_from_str(source), FileId(0));
    let program = typed.resolved_program();
    let bind = &program.defs[&DefId(Intern::from_ref("answer"))];
    let expr = match bind.body {
        BindBody::Expr(expr) => expr,
        _ => panic!("expected expression body"),
    };

    assert_eq!(
        normalize_resolved(program, expr, &DepSubst::new()),
        NormalizeResult::Value(ConstValue::Int(42.into()))
    );

    let context = NativeCompiler::create_context();
    let (module, flaws) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        program,
        CodegenSourceMap::new("resolved_program.gin", source, &typed.span_table),
        &flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux-gnu").expect("test target"),
        ),
    );
    assert!(flaws.is_empty(), "{flaws:#?}");
    let mlir = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string_with_flags(OperationPrintingFlags::new().print_generic_operation_form())
        .expect("MLIR print");
    assert!(mlir.contains("arith.constant"), "{mlir}");
    assert!(mlir.contains("i6"), "{mlir}");
}
