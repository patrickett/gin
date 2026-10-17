mod support;

use internment::Intern;
use support::check_lexed_with;
use typecheck::{BindBody, DefId, TypedExprKind};

const PRELUDE: &str = test_fixtures::preludes::BYTE_PRELUDE;

#[test]
fn comma_returns_materialize_the_declared_array_element_type() {
    for result in ["Byte, Byte", "[Byte; 2]"] {
        let typed = check_lexed_with(PRELUDE, &format!("get() {result}: return 1, 2\n"));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId::named(Intern::from_ref("get"))];
        let BindBody::Body { ret: Some(id), .. } = bind.body else {
            panic!("expected return");
        };
        let ast::Ty::Array { elem, .. } = &bind.return_type else {
            panic!("expected array");
        };
        assert_eq!(typed.exprs.ty_of(id), Some(&bind.return_type));
        let TypedExprKind::ArrayLit(items) = typed.exprs.kind_of(id).unwrap() else {
            panic!("expected array literal");
        };
        assert_eq!(items.len(), 2);
        for item in items {
            assert_eq!(typed.exprs.ty_of(*item), Some(elem.as_ref()));
        }
    }
}

#[test]
fn conditional_comma_returns_use_the_array_contract() {
    let typed = check_lexed_with(
        PRELUDE,
        "get(flag Byte) [Byte; 2]:\n    when flag is 0 then\n        return 1, 2\n    return 3, 4\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(
        typed
            .exprs
            .kind
            .iter()
            .filter(|kind| matches!(kind, TypedExprKind::ArrayLit(_)))
            .count(),
        2
    );
}

#[test]
fn comma_array_returns_reject_wrong_cardinality() {
    let typed = check_lexed_with(PRELUDE, "get() [Byte; 2]: return 1, 2, 3\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-fixed-array-length-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn comma_array_returns_reject_out_of_range_elements() {
    let typed = check_lexed_with(PRELUDE, "get() [Byte; 2]: return 1, 256\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-integer-literal-out-of-range"),
        "{:?}",
        typed.all_flaws()
    );
}
