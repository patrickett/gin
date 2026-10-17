mod support;

use ast::Ty;
use internment::Intern;
use support::check_unflawed_with;
use typecheck::{CompileTimeHost, DefId, EvalOutcome, Evaluator, TypedExprKind};

const TYPES: &str = test_fixtures::preludes::WORD_PRELUDE;

#[test]
fn holes_take_the_expected_return_local_and_argument_types() {
    for body in [
        "unfinished Word: #todo\n",
        "unfinished(value Word) Word: return #todo\n",
        "unfinished Word:\n    local Word: #todo\n    return local\n",
        "identity(value Word) Word: return value\nunfinished Word: identity(#todo)\n",
        "unfinished() result Word: return #todo\n",
    ] {
        let typed = check_unflawed_with(TYPES, body);
        assert!(
            typed.all_flaws().is_empty(),
            "{body}\n{:?}",
            typed.all_flaws()
        );
        let holes = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .filter(|(_, kind)| matches!(kind, TypedExprKind::Todo))
            .collect::<Vec<_>>();
        assert!(!holes.is_empty(), "{body}");
        for (index, _) in holes {
            assert!(
                matches!(&typed.exprs.ty[index], Ty::Named { name, .. } if name.as_str() == "Word")
                    || matches!(&typed.exprs.ty[index], Ty::Sequence(_)),
                "{:?}",
                typed.exprs.ty[index]
            );
        }
    }
}

#[test]
fn reaching_a_hole_traps_with_a_frame_instead_of_fabricating_a_value() {
    let typed = check_unflawed_with(TYPES, "unfinished Word: #todo\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let entry = DefId::named(Intern::from_ref("unfinished"));
    let diagnostic = evaluator.call(entry, &[]).unwrap_err();
    assert_eq!(
        diagnostic.code.slug().as_str(),
        "compile-time-unfilled-hole"
    );
    assert_eq!(evaluator.failure_frames()[0].function, Some(entry));
    let expression = evaluator.failure_frames()[0].expression.unwrap();
    assert!(matches!(
        typed.exprs.kind_of(expression),
        Some(TypedExprKind::Todo)
    ));
}

#[test]
fn an_unconstrained_hole_remains_unknown_and_never_becomes_a_constant() {
    let typed = check_unflawed_with(TYPES, "#todo\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let index = typed
        .exprs
        .kind
        .iter()
        .position(|kind| matches!(kind, TypedExprKind::Todo))
        .unwrap();
    assert_eq!(typed.exprs.ty[index], Ty::Opaque(Intern::from_ref("#todo")));
    assert_eq!(typed.exprs.facts.const_facts.const_value[index], None);
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    assert!(matches!(
        evaluator.eval(typecheck::ExprId(index as u32)),
        Ok(EvalOutcome::Trap(_))
    ));
}

#[test]
fn statement_and_method_holes_accept_incomplete_bodies() {
    for body in [
        "unfinished Word:\n    #todo\n    return 42\n",
        "Word has\n    unfinished(self ref) Word: return #todo\n",
    ] {
        let typed = check_unflawed_with(TYPES, body);
        assert!(
            typed.all_flaws().is_empty(),
            "{body}\n{:?}",
            typed.all_flaws()
        );
        assert!(
            typed
                .exprs
                .kind
                .iter()
                .any(|kind| matches!(kind, TypedExprKind::Todo))
        );
    }
}

#[test]
fn surrounding_type_errors_still_fail_with_holes_present() {
    let typed = check_unflawed_with(TYPES, "unfinished Word: #todo\nbroken Word: 256\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.code.slug() == "type-integer-literal-out-of-range"),
        "{:?}",
        typed.all_flaws()
    );
}
