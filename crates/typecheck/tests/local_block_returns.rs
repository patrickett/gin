mod support;

use internment::Intern;
use support::check_with;
use typecheck::TypedExprKind;

const PRELUDE: &str = "Byte is in 0 to 255\nByte.finish(self):\n    return\n";

fn local_body(typed: &typecheck::TypedFileAst) -> typecheck::ExprId {
    typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Bind {
                name,
                body,
                unassigned: false,
                ..
            } if *name == Intern::from_ref("local") => Some(*body),
            _ => None,
        })
        .expect("initialized local binding")
}

#[test]
fn local_block_preserves_a_complete_inferred_return_row() {
    let typed = check_with(PRELUDE, "outer:\n    local: 1, 256\n    return local\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        matches!(typed.exprs.kind_of(local_body(&typed)), Some(TypedExprKind::ArrayLit(items)) if items.len() == 2)
    );
}

#[test]
fn local_block_materializes_every_declared_array_element() {
    let typed = check_with(
        PRELUDE,
        "outer [Byte; 2]:\n    local [Byte; 2]: 1, 2\n    return local\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let body = local_body(&typed);
    let Some(TypedExprKind::ArrayLit(items)) = typed.exprs.kind_of(body) else {
        panic!("complete array return");
    };
    let Some(ast::Ty::Array { elem, .. }) = typed.exprs.ty_of(body) else {
        panic!("array type");
    };
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(typed.exprs.ty_of(*item), Some(elem.as_ref()));
    }
}

#[test]
fn local_block_checks_the_second_returned_array_element() {
    let typed = check_with(
        PRELUDE,
        "outer [Byte; 2]:\n    local [Byte; 2]: 1, 256\n    return local\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-integer-literal-out-of-range"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_block_checks_returned_array_cardinality() {
    let typed = check_with(
        PRELUDE,
        "outer [Byte; 2]:\n    local [Byte; 2]: 1, 2, 3\n    return local\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-fixed-array-length-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}
