use super::*;
use parser::query::SourceParseExt;

fn fixture() -> crate::TypedFileAst {
    let source = concat!(
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\n",
        "inner(flag Word) Word:\n    local Word: 7\n    while flag is 1 do\n        iteration: flag\n    return local\n",
        "outer(flag Word) Word:\n    retained Word: 9\n    return inner(flag)\n",
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed
}

#[test]
fn failures_capture_nested_calls_before_restoring_scopes_and_heap() {
    let typed = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let mut state = EvalState::with_fuel(Fuel::new(32));
    let entry = crate::DefId::named(Intern::from_ref("outer"));
    let diagnostic = evaluator
        .call_with_state(entry, &[ConstValue::ONE], &mut state)
        .unwrap_err();
    assert_eq!(
        diagnostic.code.slug().as_str(),
        "compile-time-fuel-exhausted"
    );
    let frames = evaluator.take_failure_frames();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.function.as_ref().unwrap().0.as_str())
            .collect::<Vec<_>>(),
        ["inner", "outer"]
    );
    assert!(
        matches!(typed.exprs.kind[frames[1].expression.unwrap().index()],
        TypedExprKind::FnCall { target, .. } if target.0.as_str() == "inner")
    );
    for (frame, name, expected) in [(&frames[0], "local", 7), (&frames[1], "retained", 9)] {
        let local = frame
            .locals
            .iter()
            .find(|local| local.name.as_str() == name)
            .unwrap();
        assert_eq!(local.value, Ok(ConstValue::Int(expected.into())));
        assert!(!local.is_reference);
        assert!(frame.expression.is_some());
    }
    assert!(evaluator.locals.is_empty());
    assert!(evaluator.frames.is_empty());
    assert!(evaluator.returned.is_none());
    state.fuel = Fuel::default();
    assert_eq!(
        evaluator.call_with_state(entry, &[ConstValue::ZERO], &mut state),
        Ok(ConstValue::Int(7.into()))
    );
    assert!(evaluator.failure_frames().is_empty());
    assert_eq!(
        frames[0]
            .locals
            .iter()
            .find(|local| local.name.as_str() == "local")
            .unwrap()
            .value,
        Ok(ConstValue::Int(7.into()))
    );
}

#[test]
fn cross_program_frames_keep_their_file_identity() {
    let mut caller = fixture();
    let mut callee = caller.clone();
    callee.file_id = crate::FileId(1);
    caller
        .defs
        .remove(&crate::DefId::named(Intern::from_ref("inner")));
    callee
        .defs
        .remove(&crate::DefId::named(Intern::from_ref("outer")));
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(caller.resolved_program(), CompileTimeHost::new(&target))
        .with_programs([callee.resolved_program()])
        .with_fuel(Fuel::new(32));
    let entry = crate::DefId::named(Intern::from_ref("outer"));
    assert!(evaluator.call(entry, &[ConstValue::ONE]).is_err());
    assert_eq!(
        evaluator
            .failure_frames()
            .iter()
            .map(|frame| frame.file)
            .collect::<Vec<_>>(),
        [crate::FileId(1), crate::FileId(0)]
    );
    assert_eq!(evaluator.program.file_id, crate::FileId(0));
}

#[test]
fn direct_expression_failures_have_an_anonymous_frame_without_stale_frames() {
    let mut typed = fixture();
    let expression = typed
        .exprs
        .kind
        .iter()
        .position(
            |kind| matches!(kind, TypedExprKind::Lit(Literal::Int(value)) if *value == 7.into()),
        )
        .unwrap();
    let expression = ExprId(u32::try_from(expression).unwrap());
    typed.exprs.kind[expression.index()] = TypedExprKind::InvalidOperator {
        op: ast::BinOp::Add,
        lhs: expression,
        rhs: expression,
        error: crate::operator::OperatorResolutionError::NoVisibleContract,
    };
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    assert!(matches!(
        evaluator.eval(expression),
        Ok(EvalOutcome::Trap(_))
    ));
    assert_eq!(evaluator.failure_frames().len(), 1);
    assert!(evaluator.failure_frames()[0].function.is_none());
    assert_eq!(evaluator.failure_frames()[0].expression, Some(expression));
    let success = typed
        .exprs
        .kind
        .iter()
        .position(
            |kind| matches!(kind, TypedExprKind::Lit(Literal::Int(value)) if *value == 9.into()),
        )
        .unwrap();
    assert!(matches!(
        evaluator.eval(ExprId(u32::try_from(success).unwrap())),
        Ok(EvalOutcome::Complete(_))
    ));
    assert!(evaluator.failure_frames().is_empty());
    assert!(matches!(
        evaluator.eval(expression),
        Ok(EvalOutcome::Trap(_))
    ));
    let entry = crate::DefId::named(Intern::from_ref("Word.finish"));
    assert!(evaluator.call(entry, &[]).is_err());
    assert!(evaluator.failure_frames().is_empty());
}

#[test]
fn frame_locals_freeze_references_and_respect_shadowing_after_heap_changes() {
    let typed = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let name = Intern::from_ref("value");
    let original = evaluator.heap.allocate(ConstValue::ONE);
    let referent = evaluator.heap.allocate(ConstValue::Int(7.into()));
    let invalid = referent.offset(1).unwrap();
    evaluator.locals = vec![
        HashMap::from([(name, LocalValue::Stored(original))]),
        HashMap::from([
            (name, LocalValue::Reference(referent)),
            (Intern::from_ref("unreadable"), LocalValue::Stored(invalid)),
        ]),
    ];

    assert!(evaluator.eval(ExprId(u32::MAX)).is_err());
    let frames = evaluator.take_failure_frames();
    assert_eq!(frames.len(), 1);
    assert!(evaluator.failure_frames().is_empty());
    evaluator.heap = EvalHeap::default();
    evaluator.locals.clear();

    let locals = &frames[0].locals;
    assert_eq!(locals.len(), 2);
    assert_eq!(locals[0].name.as_str(), "unreadable");
    assert!(locals[0].value.is_err());
    assert!(!locals[0].is_reference);
    assert_eq!(locals[1].name, name);
    assert_eq!(locals[1].value, Ok(ConstValue::Int(7.into())));
    assert!(locals[1].is_reference);
}
