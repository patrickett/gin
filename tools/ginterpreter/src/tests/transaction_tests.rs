use super::*;
use parser::query::SourceParseExt;
use std::cell::Cell;
use typecheck::{Address, BindBody, CompileTimeHost, DefId, ExprId, TypedExprKind, TypedFileAst};

fn fixture() -> (TypedFileAst, DefId, ExprId) {
    let parsed = concat!(
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\n",
        "outer:\n    value Word: 1\n    value.finish\n    value:: 7\n    return 42\n",
    )
    .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let entry = *typed
        .defs
        .keys()
        .find(|entry| entry.0.as_str() == "outer")
        .unwrap();
    let BindBody::Body { ret: Some(ret), .. } = typed.defs[&entry].body else {
        panic!("expected block return");
    };
    (typed, entry, ret)
}

fn seeded_state() -> (EvalState, Address, Address) {
    let mut state = EvalState::with_fuel(Fuel::new(17));
    let preserved = state.heap.allocate(ConstValue::ONE);
    let next = state.heap.clone().allocate(ConstValue::ZERO);
    (state, preserved, next)
}

fn assert_unchanged(state: &Mutex<EvalState>, preserved: Address, next: Address) {
    let state = state.lock().unwrap();
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::ONE));
    assert_eq!(state.heap.clone().allocate(ConstValue::ZERO), next);
    assert_eq!(state.fuel, Fuel::new(17));
}

fn assert_completed(outcome: Execution) {
    let Execution::Completed(value) = outcome else {
        panic!("expected successful evaluation");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(42));
}

fn assert_captured_value(failure: &ExecutionFailure, entry: DefId) {
    let frame = &failure.frames[0];
    assert_eq!(frame.function, Some(entry));
    let local = frame
        .locals
        .iter()
        .find(|local| local.name.as_str() == "value")
        .unwrap();
    assert_eq!(local.value, Ok(ConstValue::Int(7.into())));
}

#[test]
fn successful_calls_commit_allocations_and_writes() {
    let (typed, entry, _) = fixture();
    let target = flask::CompileTarget::Library;
    let (initial, preserved, next) = seeded_state();
    let state = Mutex::new(initial);
    let stopped = AtomicBool::new(false);
    for _ in 0..2 {
        let before = state
            .lock()
            .unwrap()
            .heap
            .clone()
            .allocate(ConstValue::ZERO);
        let evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        assert_completed(execute_call(evaluator, entry, &stopped, &state));
        assert_eq!(
            state.lock().unwrap().heap.freeze(before),
            Ok(ConstValue::Int(7.into()))
        );
    }
    let state = state.lock().unwrap();
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::ONE));
    assert_eq!(state.heap.freeze(next), Ok(ConstValue::Int(7.into())));
    assert_ne!(state.heap.clone().allocate(ConstValue::ZERO), next);
    assert_ne!(state.fuel, Fuel::new(17));
}

#[test]
fn traps_discard_mutations_and_allow_retry() {
    let (mut typed, entry, ret) = fixture();
    let original = typed.exprs.kind[ret.index()].clone();
    typed.exprs.kind[ret.index()] = TypedExprKind::InvalidOperator {
        op: ast::BinOp::Add,
        lhs: ret,
        rhs: ret,
        error: typecheck::operator::OperatorResolutionError::NoVisibleContract,
    };
    let target = flask::CompileTarget::Library;
    let (initial, preserved, next) = seeded_state();
    let mut uncommitted = initial.clone();
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    assert_eq!(
        evaluator
            .call_with_state(entry, &[], &mut uncommitted)
            .unwrap_err()
            .code
            .slug()
            .as_str(),
        "compile-time-invalid-operator"
    );
    assert_eq!(uncommitted.heap.freeze(next), Ok(ConstValue::Int(7.into())));

    let state = Mutex::new(initial);
    let stopped = AtomicBool::new(false);
    let outcome = execute_call(evaluator, entry, &stopped, &state);
    let Execution::Failed(diagnostic) = outcome else {
        panic!("expected a trap");
    };
    assert_eq!(
        diagnostic.diagnostic.code.slug().as_str(),
        "compile-time-invalid-operator"
    );
    assert_unchanged(&state, preserved, next);
    assert_captured_value(&diagnostic, entry);
    typed.exprs.kind[ret.index()] = original;
    let evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    assert_completed(execute_call(evaluator, entry, &stopped, &state));
    assert_eq!(
        state.lock().unwrap().heap.freeze(next),
        Ok(ConstValue::Int(7.into()))
    );
}

struct CancellingHost<'a> {
    target: &'a flask::CompileTarget,
    checks: &'a Cell<u32>,
    cancel_on: u32,
    stopped: &'a AtomicBool,
    report_cancellation: bool,
}

impl EvalHost for CancellingHost<'_> {
    fn target(&self) -> &flask::CompileTarget {
        self.target
    }

    fn cancelled(&self) -> bool {
        let checks = self.checks.get() + 1;
        self.checks.set(checks);
        if checks == self.cancel_on {
            self.stopped.store(true, Ordering::SeqCst);
        }
        self.report_cancellation && self.stopped.load(Ordering::SeqCst)
    }
}

#[test]
fn cancellation_after_writes_discards_mutations_and_allows_retry() {
    let (typed, entry, _) = fixture();
    let target = flask::CompileTarget::Library;
    let checks = Cell::new(0);
    let stopped = AtomicBool::new(false);
    let host = CancellingHost {
        target: &target,
        checks: &checks,
        cancel_on: u32::MAX,
        stopped: &stopped,
        report_cancellation: true,
    };
    let (initial, preserved, next) = seeded_state();
    let mut uncommitted = initial.clone();
    let mut evaluator = Evaluator::new(typed.resolved_program(), host);
    assert_eq!(
        evaluator.call_with_state(entry, &[], &mut uncommitted),
        Ok(ConstValue::Int(42.into()))
    );
    let cancel_on = checks.get();
    for report_cancellation in [true, false] {
        checks.set(0);
        stopped.store(false, Ordering::SeqCst);
        let host = CancellingHost {
            target: &target,
            checks: &checks,
            cancel_on,
            stopped: &stopped,
            report_cancellation,
        };
        let mut uncommitted = initial.clone();
        let mut evaluator = Evaluator::new(typed.resolved_program(), host);
        let raw = evaluator.call_with_state(entry, &[], &mut uncommitted);
        assert_eq!(raw.is_err(), report_cancellation);
        assert_eq!(uncommitted.heap.freeze(next), Ok(ConstValue::Int(7.into())));
        checks.set(0);
        stopped.store(false, Ordering::SeqCst);
        let state = Mutex::new(initial.clone());
        let outcome = execute_call(evaluator, entry, &stopped, &state);
        let Execution::Cancelled(diagnostic) = outcome else {
            panic!("expected cancellation");
        };
        assert_eq!(
            diagnostic.diagnostic.code.slug().as_str(),
            "compile-time-cancelled"
        );
        if report_cancellation {
            assert_captured_value(&diagnostic, entry);
        }
        assert_unchanged(&state, preserved, next);
        stopped.store(false, Ordering::SeqCst);
        let evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        assert_completed(execute_call(evaluator, entry, &stopped, &state));
    }
}

#[test]
fn wide_integer_results_commit_mutations_without_display_rejection() {
    let (mut typed, entry, ret) = fixture();
    typed.exprs.kind[ret.index()] =
        TypedExprKind::Lit(ast::Literal::Int(9_223_372_036_854_775_808_u128.into()));
    let target = flask::CompileTarget::Library;
    let (initial, preserved, next) = seeded_state();
    let state = Mutex::new(initial);
    let stopped = AtomicBool::new(false);
    let evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let Execution::Completed(value) = execute_call(evaluator, entry, &stopped, &state) else {
        panic!("expected a completed wide integer");
    };
    assert_eq!(value.display(), "9223372036854775808");
    assert!(
        value
            .inspection(crate::InspectionLimits {
                depth_max: 1,
                nodes_max: 1,
                text_bytes_max: 8,
            })
            .truncated
    );
    let state = state.lock().unwrap();
    assert_eq!(state.heap.freeze(preserved), Ok(ConstValue::ONE));
    assert_eq!(state.heap.freeze(next), Ok(ConstValue::Int(7.into())));
    assert_ne!(state.fuel, Fuel::new(17));
}
