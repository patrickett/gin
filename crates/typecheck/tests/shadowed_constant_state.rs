mod support;

use ast::ConstValue;
use internment::Intern;
use support::check_no_flaws;
use typecheck::{TypedExprKind, TypedFileAst};

fn branch_return(typed: &TypedFileAst) -> typecheck::ExprId {
    typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Return { value, .. } => *value,
            _ => None,
        })
        .unwrap()
}

#[test]
fn disjoint_dynamic_binding_does_not_inherit_a_later_constant() {
    let typed = check_no_flaws(&format!(
        "choose(flag is in 0 to 1, value is in 0 to 255):\n    when flag = 1 then\n        x := value\n        return x\n    x := 0\n    return x\n"
    ));
    assert_eq!(
        typed.exprs.const_value_of(branch_return(&typed)),
        Some(&None)
    );
}

#[test]
fn disjoint_constant_bindings_preserve_both_values() {
    let typed = check_no_flaws(&format!(
        "choose(flag is in 0 to 1, value is in 0 to 255):\n    when flag = 1 then\n        x := 7\n        return x\n    x := 0\n    return x\n"
    ));
    assert_eq!(
        typed.exprs.const_value_of(branch_return(&typed)),
        Some(&Some(ConstValue::Int(7_i128.into())))
    );
    let bind = &typed.defs[&typecheck::DefId::named(Intern::from_ref("choose"))];
    let typecheck::BindBody::Body { ret: Some(ret), .. } = bind.body else {
        panic!("expected block return")
    };
    assert_eq!(
        typed.exprs.const_value_of(ret),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn local_callable_parameter_does_not_inherit_an_unrelated_capture() {
    let typed = check_no_flaws(
        "outer(value is in 0 to 255):\n    captured := 0\n    local(x is in 0 to 255): return x\n    return local(value)\n",
    );
    let body = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Bind { name, body, .. } if *name == Intern::from_ref("local") => {
                Some(*body)
            }
            _ => None,
        })
        .expect("local callable body");
    assert_eq!(typed.exprs.const_value_of(body), Some(&None));
    let captured_place = typed
        .places
        .iter()
        .position(|place| place.name == Intern::from_ref("captured"))
        .expect("captured x place");
    assert_ne!(
        typed
            .exprs
            .place_of(body)
            .flatten()
            .map(|place| place.0 as usize),
        Some(captured_place)
    );
}

#[test]
fn local_callable_parameter_preserves_captures_and_outer_binding() {
    let typed = check_no_flaws(
        "outer():\n    x := 0\n    y := 7\n    local(argument is in 0 to 255): return y\n    return x\n",
    );
    let body = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Bind { name, body, .. } if *name == Intern::from_ref("local") => {
                Some(*body)
            }
            _ => None,
        })
        .expect("local callable body");
    assert_eq!(
        typed.exprs.const_value_of(body),
        Some(&Some(ConstValue::Int(7_i128.into())))
    );
    let bind = &typed.defs[&typecheck::DefId::named(Intern::from_ref("outer"))];
    let typecheck::BindBody::Body { ret: Some(ret), .. } = bind.body else {
        panic!("outer return")
    };
    assert_eq!(
        typed.exprs.const_value_of(ret),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}
