use ast::{BinderId, BinderOwner, ConstValue, DependentArgId, HashFloat, NormalExpr};
use internment::Intern;

#[test]
fn value_int_displays_as_number() {
    let expr = NormalExpr::Value(ConstValue::Int(42.into()));
    assert_eq!(expr.to_string(), "42");
}

#[test]
fn var_displays_as_name() {
    let expr = NormalExpr::Var(Intern::from_ref("n"));
    assert_eq!(expr.to_string(), "n");
}

#[test]
fn add_displays_parens() {
    let expr = NormalExpr::Add(
        Box::new(NormalExpr::Var(Intern::from_ref("n"))),
        Box::new(NormalExpr::Value(ConstValue::Int(1.into()))),
    );
    assert_eq!(expr.to_string(), "(n + 1)");
}

#[test]
fn sub_displays_parens() {
    let expr = NormalExpr::Sub(
        Box::new(NormalExpr::Var(Intern::from_ref("m"))),
        Box::new(NormalExpr::Value(ConstValue::Int(1.into()))),
    );
    assert_eq!(expr.to_string(), "(m - 1)");
}

#[test]
fn mul_displays_parens() {
    let expr = NormalExpr::Mul(
        Box::new(NormalExpr::Value(ConstValue::Int(2.into()))),
        Box::new(NormalExpr::Var(Intern::from_ref("n"))),
    );
    assert_eq!(expr.to_string(), "(2 * n)");
}

#[test]
fn value_float_displays() {
    let expr = NormalExpr::Value(ConstValue::Float(HashFloat(314.0 / 100.0)));
    assert_eq!(expr.to_string(), "3.14");
}

#[test]
fn value_string_displays() {
    let expr = NormalExpr::Value(ConstValue::String("hello".into()));
    assert_eq!(expr.to_string(), r#""hello""#);
}

#[test]
fn value_int_round_trips() {
    let expr = NormalExpr::Value(ConstValue::Int(3.into()));
    assert_eq!(expr.to_string(), "3");
}

#[test]
fn equality_and_hash() {
    use std::collections::HashSet;
    let a = NormalExpr::Var(Intern::from_ref("n"));
    let b = NormalExpr::Var(Intern::from_ref("n"));
    let c = NormalExpr::Var(Intern::from_ref("m"));
    assert_eq!(a, b);
    assert_ne!(a, c);

    let mut set = HashSet::new();
    set.insert(a.clone());
    set.insert(b.clone());
    assert_eq!(set.len(), 1);
    set.insert(c);
    assert_eq!(set.len(), 2);
}

#[test]
fn inferred_identity_controls_equality_and_hash() {
    use std::collections::HashSet;

    let binder = BinderId::new(7, BinderOwner::Definition(Intern::from_ref("Vector")));
    let a = NormalExpr::Inferred(DependentArgId::new(binder, 0));
    let same = NormalExpr::Inferred(DependentArgId::new(binder, 0));
    let other_slot = NormalExpr::Inferred(DependentArgId::new(binder, 1));
    let other_owner = NormalExpr::Inferred(DependentArgId::new(
        BinderId::new(7, BinderOwner::Expression(3)),
        0,
    ));

    assert_eq!(a, same);
    assert_ne!(a, other_slot);
    assert_ne!(a, other_owner);

    let set = HashSet::from([a, same, other_slot, other_owner]);
    assert_eq!(set.len(), 3);
}

#[test]
fn inferred_display_hides_identity() {
    let expr = NormalExpr::Inferred(DependentArgId::new(
        BinderId::new(19, BinderOwner::Expression(42)),
        3,
    ));
    assert_eq!(expr.to_string(), "?");
}

#[test]
fn nested_add_mul_expression() {
    let expr = NormalExpr::Add(
        Box::new(NormalExpr::Mul(
            Box::new(NormalExpr::Value(ConstValue::Int(2.into()))),
            Box::new(NormalExpr::Var(Intern::from_ref("n"))),
        )),
        Box::new(NormalExpr::Value(ConstValue::Int(1.into()))),
    );
    assert_eq!(expr.to_string(), "((2 * n) + 1)");
}
