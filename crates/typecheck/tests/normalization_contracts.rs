use ast::{ConstValue, HashFloat};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::staging::{NormalExpr, NormalizeResult, normalize};
use typecheck::subst::DepSubst;
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

#[test]
fn a_known_local_value_does_not_cache_away_a_mutating_call() {
    let declarations =
        include_str!("../../test-fixtures/fixtures/normalization_statement_effects.gin");
    let source = format!(
        "{declarations}\ncheck Count:\n    state Count: 7\n    local Count:\n        grow(state)\n        7\n    state.finish\n    return local\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let call = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| {
            matches!(kind, TypedExprKind::FnCall { target, args: Some(_), .. }
            if target.0.as_str() == "grow")
            .then_some(typecheck::ExprId(index as u32))
        })
        .expect("mutating call");
    assert_eq!(typed.exprs.const_value_of(call), Some(&None));
    assert!(matches!(
        normalize(&typed, call, &DepSubst::new()),
        NormalizeResult::RuntimeDependency(_)
    ));
}

#[test]
fn bindings_with_runtime_statements_do_not_normalize_to_their_constant_return() {
    for (statement, runtime) in [("5", false), ("grow(value)", true)] {
        let declarations =
            include_str!("../../test-fixtures/fixtures/normalization_statement_effects.gin");
        let source = format!(
            "{declarations}\ncheck(value mut Count) Count:\n    local Count:\n        {statement}\n        7\n    return local\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let local = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .find_map(|(index, kind)| {
                matches!(kind, TypedExprKind::Bind { name, .. }
                if name.as_str() == "local")
                .then_some(typecheck::ExprId(index as u32))
            })
            .expect("local binding with statements");
        let cached = normalize(&typed, local, &DepSubst::new());
        typed.exprs.facts.const_facts.const_value.fill(None);
        let uncached = normalize(&typed, local, &DepSubst::new());
        let availability = typed.exprs.facts.semantic_facts.availability[local.as_usize()];
        let expected = if runtime {
            typecheck::typed::Availability::Runtime
        } else {
            typecheck::typed::Availability::CompileTime
        };
        assert_eq!(
            (
                matches!(cached, NormalizeResult::RuntimeDependency(_)),
                matches!(uncached, NormalizeResult::RuntimeDependency(_)),
                availability
            ),
            (runtime, runtime, expected),
            "{statement}: cached {cached:?}, uncached {uncached:?}"
        );
        if !runtime {
            assert_eq!(cached, NormalizeResult::Value(ConstValue::Int(7.into())));
            assert_eq!(uncached, cached);
        }
    }
}

#[test]
fn residual_constructors_keep_their_variant_when_arguments_are_symbolic() {
    let parsed = "Word is in 0 to 255\nWord has finish(self): return\nChoice is Left(Word) or Right(Word)\nChoice has finish(self): return\nleft(value Word) Choice: return Left(value)\nright(value Word) Choice: return Right(value)\nleft_again(value Word) Choice: return Left(value)\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed.exprs.facts.const_facts.const_value.fill(None);
    let mut substitutions = DepSubst::new();
    substitutions.consts.insert(
        Intern::from_ref("value"),
        ast::NormalExpr::Var(Intern::from_ref("symbol")),
    );
    let results: Vec<_> = ["left", "right", "left_again"]
        .into_iter()
        .map(|name| {
            let BindBody::Expr(root) = typed.defs[&DefId::named(Intern::from_ref(name))].body
            else {
                panic!("expression body")
            };
            let result = normalize(&typed, root, &substitutions);
            assert!(matches!(result, NormalizeResult::Residual(_)), "{result:?}");
            result
        })
        .collect();
    assert_ne!(results[0], results[1]);
    assert_eq!(results[0], results[2]);
    assert_eq!(results[0].as_dependent_normal_expr(), None);
    assert_eq!(results[1].as_dependent_normal_expr(), None);
}

#[test]
fn normalized_constructors_retain_the_owner_used_by_evaluation() {
    let parsed = "Word is in 0 to 255\nWord has finish(self): return\nFirst is Stop or Item(Word)\nFirst has finish(self): return\nSecond is Stop or Item(Word)\nSecond has finish(self): return\nfirst_stop First: Stop\nsecond_stop Second: Stop\nfirst_item First: Item(7)\nsecond_item Second: Item(7)\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed.exprs.facts.const_facts.const_value.fill(None);
    let target = flask::CompileTarget::Library;
    let mut values = Vec::new();
    for name in ["first_stop", "second_stop", "first_item", "second_item"] {
        let BindBody::Expr(root) = typed.defs[&DefId::named(Intern::from_ref(name))].body else {
            panic!("expression body")
        };
        let mut evaluator = typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        );
        let typecheck::eval::EvalOutcome::Complete(value) = evaluator.eval(root).unwrap() else {
            panic!("constructor value")
        };
        assert_eq!(
            normalize(&typed, root, &DepSubst::new()),
            NormalizeResult::Value(value.clone()),
            "{name}"
        );
        values.push(value);
    }
    assert_ne!(values[0], values[1]);
    assert_ne!(values[2], values[3]);
}

#[test]
fn residual_negation_keeps_the_operator_for_noninteger_constants() {
    let parsed = "negative: -1.5\ninteger: -7\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    let BindBody::Expr(negative) = typed.defs[&DefId::named(Intern::from_ref("negative"))].body
    else {
        panic!("expression body")
    };
    let TypedExprKind::Negate(operand) = typed.exprs.kind[negative.as_usize()] else {
        panic!("negation")
    };
    assert_eq!(
        typed.exprs.const_value_of(operand),
        Some(&Some(ConstValue::Float(HashFloat(1.5))))
    );
    typed.exprs.facts.const_facts.const_value[negative.as_usize()] = None;
    let result = normalize(&typed, negative, &DepSubst::new());
    assert_eq!(
        result,
        NormalizeResult::Residual(NormalExpr::Negate(Box::new(NormalExpr::Value(
            ConstValue::Float(HashFloat(1.5))
        ))))
    );
    assert_eq!(result.as_dependent_normal_expr(), None);

    let BindBody::Expr(integer) = typed.defs[&DefId::named(Intern::from_ref("integer"))].body
    else {
        panic!("expression body")
    };
    typed.exprs.facts.const_facts.const_value[integer.as_usize()] = None;
    assert_eq!(
        normalize(&typed, integer, &DepSubst::new()),
        NormalizeResult::Value(ConstValue::Int((-7).into()))
    );
}
