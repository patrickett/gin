use super::*;
use parser::query::SourceParseExt;

fn fixture() -> (crate::TypedFileAst, ExprId) {
    let parsed = format!(
        "{}outer:\n    value Word: 1\n    return value\n",
        test_fixtures::preludes::WORD_PRELUDE
    )
    .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let index = typed
        .exprs
        .kind
        .iter()
        .position(
            |kind| matches!(kind, TypedExprKind::Bind { name, .. } if name.as_str() == "value"),
        )
        .unwrap();
    let binding = ExprId(u32::try_from(index).unwrap());
    (typed, binding)
}

#[test]
fn failed_iteration_restores_a_shadowed_binding() {
    let (typed, binding) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let value = Intern::from_ref("value");
    let original = evaluator.heap.allocate(ConstValue::Int(7.into()));
    evaluator
        .locals
        .push(HashMap::from([(value, LocalValue::Stored(original))]));
    let error = evaluator
        .eval_scoped_iteration(&[binding, ExprId(u32::MAX)])
        .unwrap_err();
    assert_eq!(
        error.code.slug().as_str(),
        "compile-time-invalid-expression"
    );
    let Some(LocalValue::Stored(restored)) = evaluator.local(value) else {
        panic!("missing binding")
    };
    assert_eq!(restored, original);
    assert_eq!(
        evaluator.heap.freeze(restored),
        Ok(ConstValue::Int(7.into()))
    );
    assert_eq!(evaluator.locals.len(), 1);
}

#[test]
fn failed_iteration_removes_new_bindings() {
    let (typed, binding) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    evaluator.locals.push(HashMap::new());
    assert!(
        evaluator
            .eval_scoped_iteration(&[binding, ExprId(u32::MAX)])
            .is_err()
    );
    assert!(evaluator.local(Intern::from_ref("value")).is_none());
    assert_eq!(evaluator.locals.len(), 1);
}

#[test]
fn successful_iteration_restores_bindings() {
    let (typed, binding) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let name = Intern::from_ref("value");
    let original = evaluator.heap.allocate(ConstValue::Int(7.into()));
    for previous in [Some(LocalValue::Stored(original)), None] {
        evaluator.locals = vec![
            previous
                .map(|value| HashMap::from([(name, value)]))
                .unwrap_or_default(),
        ];
        evaluator.eval_scoped_iteration(&[binding]).unwrap();
        match (previous, evaluator.local(name)) {
            (Some(LocalValue::Stored(expected)), Some(LocalValue::Stored(actual))) => {
                assert_eq!(actual, expected)
            }
            (None, None) => {}
            _ => panic!("iteration changed the caller binding"),
        }
    }
}

#[test]
fn discarded_statements_propagate_traps() {
    let (mut typed, binding) = fixture();
    typed.exprs.kind[binding.index()] = TypedExprKind::InvalidOperator {
        op: ast::BinOp::Add,
        lhs: binding,
        rhs: binding,
        error: crate::operator::OperatorResolutionError::NoVisibleContract,
    };
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let diagnostic = evaluator
        .eval_returning_sequence(&[binding], None)
        .err()
        .unwrap();
    assert_eq!(
        diagnostic.code.slug().as_str(),
        "compile-time-invalid-operator"
    );
    let diagnostic = evaluator.eval_scoped_iteration(&[binding]).unwrap_err();
    assert_eq!(
        diagnostic.code.slug().as_str(),
        "compile-time-invalid-operator"
    );
}

#[test]
fn cancellation_inside_a_statement_stops_the_enclosing_call() {
    struct CancellingHost<'a> {
        target: &'a flask::CompileTarget,
        checks: std::cell::Cell<u32>,
    }

    impl EvalHost for CancellingHost<'_> {
        fn target(&self) -> &flask::CompileTarget {
            self.target
        }

        fn cancelled(&self) -> bool {
            let checks = self.checks.get();
            self.checks.set(checks + 1);
            checks > 0
        }
    }

    let (typed, _) = fixture();
    let target = flask::CompileTarget::Library;
    let host = CancellingHost {
        target: &target,
        checks: std::cell::Cell::new(0),
    };
    let mut evaluator = Evaluator::new(typed.resolved_program(), host);
    let mut state = EvalState::default();
    let preserved = state.heap.allocate(ConstValue::ONE);
    let error = evaluator
        .call_with_state(
            crate::DefId::named(Intern::from_ref("outer")),
            &[],
            &mut state,
        )
        .unwrap_err();
    assert_eq!(error.code.slug().as_str(), "compile-time-cancelled");
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::ONE));
    assert!(evaluator.locals.is_empty());
    assert!(evaluator.returned.is_none());
}

#[test]
fn calls_use_and_return_the_supplied_evaluator_state() {
    let (typed, _) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let entry = crate::DefId::named(Intern::from_ref("outer"));
    let mut state = EvalState::with_fuel(Fuel::new(0));
    let preserved = state.heap.allocate(ConstValue::Int(7.into()));
    let error = evaluator
        .call_with_state(entry, &[], &mut state)
        .unwrap_err();
    assert_eq!(error.code.slug().as_str(), "compile-time-fuel-exhausted");
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::Int(7.into())));
    state.fuel = Fuel::default();
    assert_eq!(
        evaluator.call_with_state(entry, &[], &mut state),
        Ok(ConstValue::ONE)
    );
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::Int(7.into())));
}
