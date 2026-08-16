use ast::BinderOwner;
use ast::normal_expr::BinderId;
use ast::prelude::OperatorRole;
use ast::ty::Ty;
use internment::Intern;
use typecheck::typed::{CallCapability, DefId, FunctionEffects, TypedCallableSignature};

fn signature(
    name: &str,
    lhs: Ty,
    rhs: Ty,
    ret: Ty,
    role: OperatorRole,
) -> (DefId, TypedCallableSignature) {
    let sig = TypedCallableSignature {
        dependent_binder: BinderId::new(0, BinderOwner::Definition(Intern::new(name.to_string()))),
        return_type: ret,
        params: vec![
            (Intern::new("lhs".to_string()), lhs),
            (Intern::new("rhs".to_string()), rhs),
        ],
        param_kinds: vec![],
        param_conventions: vec![],
        param_groups: vec![None, None],
        param_refinements: vec![None, None],
        groups: vec![],
        return_group: None,
        effects: FunctionEffects::default(),
        param_requirements: vec![],
        call_capability: CallCapability::StagePolymorphic,
        intrinsic: None,
        operator_role: Some(role),
    };
    (DefId(Intern::new(name.to_string())), sig)
}

#[test]
fn left_operator_takes_precedence_over_right() {
    let lhs = Ty::i64();
    let rhs = Ty::i64();
    let left_sig = signature("left", Ty::i64(), Ty::i64(), Ty::i64(), OperatorRole::Add);
    let right_sig = signature("right", Ty::u8(), Ty::i64(), Ty::i64(), OperatorRole::Add);

    let registry = typecheck::operator::OperatorRegistry::new(
        std::iter::once(left_sig),
        std::iter::once(right_sig),
    );

    let (resolved, _) = registry
        .resolve(OperatorRole::Add, &lhs, &rhs)
        .expect("left operator should be selected");
    assert_eq!(resolved.0.as_str(), "left");
}

#[test]
fn right_only_operators_are_ignored() {
    let lhs = Ty::i64();
    let rhs = Ty::u8();
    let right_sig = signature("right", Ty::u8(), Ty::i64(), Ty::i64(), OperatorRole::Add);

    let registry =
        typecheck::operator::OperatorRegistry::new(std::iter::empty(), std::iter::once(right_sig));

    let result = registry.resolve(OperatorRole::Add, &lhs, &rhs);
    assert!(matches!(
        result,
        Err(typecheck::operator::OperatorResolutionError::OperandDoesNotProvide)
    ));
}
