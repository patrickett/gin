use ast::{BindValue, DeclareValue, Expr, HasMember, HasMemberBody};
use parser::query::SourceParseExt;

#[test]
fn newline_separated_constructor_bind_keeps_the_following_return() {
    let source = "Pool has\n    reserve(self mut, c Count(x)) Allocation(x):\n        a Allocation(x): Allocation(\n            address: typed_pointer\n            capacity: c\n            initialized: 0 as Count(x)\n        )\n        return a\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let declare = parsed.ast.tags.values().next().unwrap();
    let DeclareValue::Has(members) = &declare.value else {
        panic!("expected members")
    };
    let HasMember::Function(function) = &members[0] else {
        panic!("expected reserve")
    };
    let Some(HasMemberBody::Overrideable(BindValue::Body { exprs, ret })) = &function.body else {
        panic!("expected body")
    };
    assert_eq!(exprs.len(), 1);
    let Expr::Bind(bind) = &exprs[0].value else {
        panic!("expected allocation bind")
    };
    assert_eq!(bind.name.as_str(), "a");
    let BindValue::Expr(value) = &bind.value else {
        panic!("expected constructor")
    };
    let Expr::TagCall(call) = &value.value else {
        panic!("expected Allocation")
    };
    assert_eq!(call.args.len(), 3);
    assert_eq!(ret.values.len(), 1);
    let Expr::FnCall(value) = &ret.values[0].value else {
        panic!("expected returned allocation")
    };
    assert_eq!(value.path.root.as_str(), "a");
}

#[test]
fn multiline_when_continuations_leave_following_binds_in_the_method() {
    for condition in [
        "first is True",
        "first is True and second is True",
        "not first is True",
        "(first is True)",
    ] {
        let source = format!(
            "Pool has\n    reserve(self mut) Int:\n        size: when {condition}\n            then 1\n            else 0\n        next: size + 1\n        return next\n"
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{source}\n{:?}",
            parsed.symptoms
        );
        let declare = parsed.ast.tags.values().next().unwrap();
        let DeclareValue::Has(members) = &declare.value else {
            panic!("expected members")
        };
        let HasMember::Function(function) = &members[0] else {
            panic!("expected reserve")
        };
        let Some(HasMemberBody::Overrideable(BindValue::Body { exprs, ret })) = &function.body
        else {
            panic!("expected body")
        };
        assert_eq!(exprs.len(), 2, "{source}");
        assert_eq!(ret.values.len(), 1);
        let Expr::FnCall(value) = &ret.values[0].value else {
            panic!("expected returned next bind")
        };
        assert_eq!(value.path.root.as_str(), "next");
    }
}
