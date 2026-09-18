use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedFileAst};

fn check(replacement: &str) -> TypedFileAst {
    let source = format!(
        "Byte is in 0 to 255\n\
         replace(previous Byte, next Byte) Byte: next\n\
         inspect(previous ref Byte, next Byte) Byte: next\n\
         copy_one(source ref Buffer(Byte,4), destination mut Buffer(Byte,4), index is in 0 to 3) Byte:\n    \
             destination.(index) :: {replacement}\n\
         return destination.(index)\n\n\
         #fixed_array(element,count)\n\
         Buffer(element Type,count BigInt) has\n"
    );
    transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    )
}

#[test]
fn indexed_replacement_explicitly_consumes_the_previous_byte() {
    let typed = check("replace(destination.(index)
use source index)");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let byte = &typed.tags[&typecheck::TagId(Intern::from_ref("Byte"))].resolved_ty;
    for (id, kind) in typed.exprs.kind.iter().enumerate() {
        if matches!(
            kind,
            typecheck::TypedExprKind::BufGet { .. } | typecheck::TypedExprKind::BufSet { .. }
        ) {
            assert_eq!(&typed.exprs.ty[id], byte);
        }
    }
}

#[test]
fn indexed_replacement_cannot_silently_discard_a_byte() {
    for replacement in [
        "source.(index)",
        "inspect(ref destination.(index)
use source index)",
        "replace(destination.(0)
use source index)",
    ] {
        let typed = check(replacement);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-rebind-would-discard-value"),
            "{replacement}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn compile_time_copy_updates_the_callers_buffer() {
    let source = include_str!("fixtures/indexed_copy.gin");
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    for count in 0..=4 {
        let result = evaluator
            .call(
                typecheck::DefId(Intern::from_ref("run")),
                &[ast::ConstValue::Int(count.into())],
            )
            .unwrap();
        let expected: Vec<_> = [3, 17, 128, 255]
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                ast::ConstValue::Int(if index < count as usize {
                    value.into()
                } else {
                    41.into()
                })
            })
            .collect();
        assert_eq!(result, ast::ConstValue::FixedArray(expected.into()));
    }
}

#[test]
fn a_closed_copy_call_normalizes_to_a_frozen_buffer() {
    let source = format!(
        "copied: run(4)\n{}",
        include_str!("fixtures/indexed_copy.gin")
    );
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let typecheck::BindBody::Expr(expr) =
        typed.defs[&typecheck::DefId(Intern::from_ref("copied"))].body
    else {
        panic!("expected expression body")
    };
    assert_eq!(
        typecheck::staging::normalize(&typed, expr, &typecheck::DepSubst::new()),
        typecheck::staging::NormalizeResult::Value(ast::ConstValue::FixedArray(
            [3, 17, 128, 255]
                .into_iter()
                .map(|value| ast::ConstValue::Int(value.into()))
                .collect::<Vec<_>>()
                .into()
        ))
    );
}

#[test]
fn copying_overlapping_buffers_is_rejected() {
    let source = format!(
        "overlap(buffer mut Buffer(Byte,4)): copy(ref buffer, mut buffer, 4)\n{}",
        include_str!("fixtures/indexed_copy.gin")
    );
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-overlapping-target-group-arguments"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn copying_more_than_the_buffer_length_is_rejected() {
    let source = format!(
        "invalid: run(5)\n{}",
        include_str!("fixtures/indexed_copy.gin")
    );
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| flaw.code.slug()
            == "type-parameter-refinement-failed"
            || flaw.code.slug() == "type-argument-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn projections_share_storage_with_their_parent_and_siblings() {
    use ast::ConstValue;
    let mut heap = typecheck::eval::EvalHeap::default();
    let child = ConstValue::FixedArray(vec![ConstValue::ZERO, ConstValue::ONE].into());
    let root = heap.allocate(ConstValue::FixedArray(
        vec![child.clone(), child.clone()].into(),
    ));
    let first = heap.project(root, 0).unwrap();
    let second = heap.project(root, 1).unwrap();
    let element = heap.project(first, 1).unwrap();
    heap.store(element, ConstValue::ZERO).unwrap();
    assert_eq!(
        heap.freeze(first).unwrap(),
        ConstValue::FixedArray(vec![ConstValue::ZERO, ConstValue::ZERO].into())
    );
    assert_eq!(heap.freeze(second).unwrap(), child);
    assert_eq!(heap.freeze(element).unwrap(), ConstValue::ZERO);
    assert_eq!(
        heap.freeze(element.offset(-1).unwrap()).unwrap(),
        ConstValue::ZERO
    );
    assert!(heap.project(first, 2).is_err());
    assert!(
        heap.store(element.offset(1).unwrap(), ConstValue::ONE)
            .is_err()
    );
    let empty = heap.allocate(ConstValue::FixedArray(vec![].into()));
    assert!(heap.project(empty, 0).is_err());
}

#[test]
fn compile_time_out_of_bounds_access_is_a_trap() {
    let source = format!(
        "bad(source ref Buffer(Byte,4)) Byte: source.(4)\nfail:\n    buffer Buffer(Byte,4): [41;4]\nreturn bad(ref buffer)\n{}",
        include_str!("fixtures/indexed_copy.gin")
    );
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    let target = flask::CompileTarget::Library;
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    assert_eq!(
        evaluator
            .call(typecheck::DefId(Intern::from_ref("fail")), &[])
            .unwrap_err()
            .code
            .slug(),
        "compile-time-index-out-of-bounds"
    );
    assert!(
        evaluator
            .call(
                typecheck::DefId(Intern::from_ref("run")),
                &[ast::ConstValue::ZERO]
            )
            .is_ok()
    );
}

#[test]
fn mutating_a_callers_storage_is_not_folded_away() {
    let source = include_str!("fixtures/indexed_copy.gin");
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let bind = &typed.defs[&typecheck::DefId(Intern::from_ref("forward"))];
    assert_eq!(
        bind.call_capability,
        typecheck::typed::CallCapability::StagePolymorphic
    );
    assert!(!bind.effects.writes.is_empty());
    let typecheck::BindBody::Expr(expr) = bind.body else {
        panic!("expected expression body")
    };
    assert!(matches!(
        typecheck::staging::normalize(&typed, expr, &typecheck::DepSubst::new()),
        typecheck::staging::NormalizeResult::RuntimeDependency(_)
    ));
}

#[test]
fn repeated_aggregate_seeds_remain_nested_values() {
    let source = format!(
        "nested Buffer(Buffer(Byte,2),2): [[1,2];2]\n{}",
        include_str!("fixtures/indexed_copy.gin")
    );
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    let row = ast::ConstValue::FixedArray(
        vec![ast::ConstValue::ONE, ast::ConstValue::Int(2.into())].into(),
    );
    assert_eq!(
        evaluator
            .call(typecheck::DefId(Intern::from_ref("nested")), &[])
            .unwrap(),
        ast::ConstValue::FixedArray(vec![row.clone(), row].into())
    );
}
