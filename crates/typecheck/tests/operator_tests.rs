use ast::BinderOwner;
use ast::normal_expr::BinderId;
use ast::ty::Ty;
use internment::Intern;
use typecheck::typed::{
    CallCapability, DefId, DefinitionStage, FunctionEffects, TypedCallableSignature,
};

fn signature(
    name: &str,
    lhs: Ty,
    rhs: Ty,
    ret: Ty,
    symbol: &str,
) -> (DefId, TypedCallableSignature) {
    let sig = TypedCallableSignature {
        dependent_binder: BinderId::new(0, BinderOwner::Definition(Intern::new(name.to_string()))),
        return_type: ret,
        results: vec![],
        params: vec![
            (Intern::new("lhs".to_string()), lhs),
            (Intern::new("rhs".to_string()), rhs),
        ],
        param_kinds: vec![],
        param_conventions: vec![],
        param_groups: vec![None, None],
        param_refinements: vec![None, None],
        groups: vec![],
        result_groups: vec![],
        effects: FunctionEffects::default(),
        param_requirements: vec![],
        call_capability: CallCapability::StagePolymorphic,
        definition_stage: DefinitionStage::StagePolymorphic,
        infix_symbol: Some(Intern::new(symbol.to_string())),
    };
    (DefId(Intern::new(name.to_string())), sig)
}

#[test]
fn left_operator_takes_precedence_over_right() {
    let lhs = Ty::i64();
    let rhs = Ty::i64();
    let left_sig = signature("left", Ty::i64(), Ty::i64(), Ty::i64(), "+");
    let right_sig = signature("right", Ty::u8(), Ty::i64(), Ty::i64(), "+");

    let registry = typecheck::operator::OperatorRegistry::new(
        std::iter::once(left_sig),
        std::iter::once(right_sig),
    );

    let (resolved, _) = registry
        .resolve("+", &lhs, &rhs)
        .expect("left operator should be selected");
    assert_eq!(resolved.0.as_str(), "left");
}

#[test]
fn right_only_operators_are_ignored() {
    let lhs = Ty::i64();
    let rhs = Ty::u8();
    let right_sig = signature("right", Ty::u8(), Ty::i64(), Ty::i64(), "+");

    let registry =
        typecheck::operator::OperatorRegistry::new(std::iter::empty(), std::iter::once(right_sig));

    let result = registry.resolve("+", &lhs, &rhs);
    assert!(matches!(
        result,
        Err(typecheck::operator::OperatorResolutionError::OperandDoesNotProvide)
    ));
}

#[test]
fn literal_symbol_selects_the_registered_infix_operator() {
    let plus = signature("plus", Ty::i64(), Ty::i64(), Ty::i64(), "+");
    let minus = signature("minus", Ty::i64(), Ty::i64(), Ty::i64(), "-");
    let registry =
        typecheck::operator::OperatorRegistry::new(std::iter::once(plus), std::iter::once(minus));

    let (resolved, _) = registry
        .resolve("+", &Ty::i64(), &Ty::i64())
        .expect("plus should resolve");
    assert_eq!(resolved.0.as_str(), "plus");
}

#[test]
fn pointer_length_records_have_no_implicit_text_compatibility() {
    let record = |name: &str| Ty::Record {
        name: Intern::from_ref(name),
        fields: vec![
            (
                Intern::from_ref("ptr"),
                Box::new(Ty::Ptr {
                    inner: Box::new(Ty::u8()),
                }),
            ),
            (Intern::from_ref("len"), Box::new(Ty::i64())),
        ],
        resolved_params: None,
    };
    let expected = record("Expected");
    let operator = signature("join", expected.clone(), expected.clone(), expected, "+");
    let registry =
        typecheck::operator::OperatorRegistry::new(std::iter::once(operator), std::iter::empty());
    for actual in [
        record("Other"),
        Ty::Literal(ast::ConstValue::Text("bytes".into())),
    ] {
        assert!(registry.resolve("+", &actual, &actual).is_err());
    }
}
