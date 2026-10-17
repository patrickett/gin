mod support;

use ast::ConstValue;
use internment::Intern;
use typecheck::eval::{CompileTimeHost, Evaluator};

fn evaluate(body: &str) -> Result<ConstValue, diagnostic::Diagnostic> {
    let source = format!(
        "Byte is in 0 to 255\nByte has\n    finish(self): return\nreplace(previous Byte, next Byte) Byte: return next\n{body}\n"
    );
    let typed = support::check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target))
        .call(typecheck::DefId::named(Intern::from_ref("run")), &[])
}

#[test]
fn slices_index_and_iterate_the_requested_region() {
    let result = evaluate(
        r#"
run Byte:
    source [Byte; 4]: [10, 20, 30, 40]
    view: ref source.[1; 2]
    total Byte: 0
    for value in view
        previous is in Byte: total
        amount is in Byte: value
        total:: replace(total, ((previous + amount) & 255) as Byte)
    return total
"#,
    )
    .unwrap();
    assert_eq!(result, ConstValue::Int(50.into()));
}

#[test]
fn mutable_subslices_share_the_backing_storage() {
    let result = evaluate(
        r#"
run Byte:
    source [Byte; 4]: [10, 20, 30, 40]
    view: mut source.[1; 2]
    nested: mut view.[1; 1]
    nested.[0]:: replace(nested.[0], 7)
    return source.[2]
"#,
    )
    .unwrap();
    assert_eq!(result, ConstValue::Int(7.into()));
}

#[test]
fn empty_tail_slices_skip_iteration() {
    let result = evaluate(
        r#"
run Byte:
    source [Byte; 2]: [10, 20]
    view: ref source.[2; 0]
    total Byte: 7
    for value in view
        total:: replace(total, 99)
    return total
"#,
    )
    .unwrap();
    assert_eq!(result, ConstValue::Int(7.into()));
}

#[test]
fn records_preserve_dependent_reference_fields() {
    let result = evaluate(
        r#"
View has
    length Byte
    data ref [Byte; length]
    finish(self): return
prefix(n Byte, source ref [Byte; 4]) View:
    data: ref source.[1; n]
    return View(length: n, data: data)
run Byte:
    source [Byte; 4]: [10, 20, 30, 40]
    view: prefix(2, source)
    answer Byte: view.data.[1]
    view.finish
    return answer
"#,
    )
    .unwrap();
    assert_eq!(result, ConstValue::Int(30.into()));
}

#[test]
fn compile_time_slicing_checks_the_actual_extent() {
    let error = evaluate(
        r#"
prefix(n Byte, source ref [Byte; 4]) ref [Byte; n]: return ref source.[1; n]
run Byte:
    source [Byte; 4]: [10, 20, 30, 40]
    view: prefix(4, source)
    return view.[0]
"#,
    )
    .unwrap_err();
    assert_eq!(
        error.code.slug().as_str(),
        "compile-time-index-out-of-bounds"
    );
}
