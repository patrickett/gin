use ast::{ConstValue, Expr};
use internment::Intern;
use typecheck::const_value_to_expr;

#[test]
fn empty_tag_constants_preserve_their_qualified_path() {
    let value = ConstValue::Tag {
        name: Intern::from_ref("Ready"),
        qual_path: Some("network.State".into()),
        args: Vec::new().into(),
    };
    let Expr::TagCall(call) = const_value_to_expr(&value) else {
        panic!("qualified tag requires a qualified constructor");
    };
    assert_eq!(call.name, Intern::from_ref("Ready"));
    assert_eq!(
        call.qual_path.unwrap().to_qualified_string(),
        "network.State"
    );
    assert!(call.args.is_empty());
}

#[test]
fn unqualified_empty_tag_constants_remain_anonymous_tags() {
    let name = Intern::from_ref("Ready");
    let value = ConstValue::Tag {
        name,
        qual_path: None,
        args: Vec::new().into(),
    };
    assert!(matches!(const_value_to_expr(&value), Expr::AnonymousTag(actual) if actual == name));
}

#[test]
fn qualified_tag_constants_keep_their_arguments() {
    let value = ConstValue::Tag {
        name: Intern::from_ref("Ready"),
        qual_path: Some("network.State".into()),
        args: vec![ConstValue::Int(7.into())].into(),
    };
    let Expr::TagCall(call) = const_value_to_expr(&value) else {
        panic!("expected constructor");
    };
    assert_eq!(
        call.qual_path.unwrap().to_qualified_string(),
        "network.State"
    );
    assert_eq!(call.args.len(), 1);
    assert_eq!(
        call.args[0].const_value,
        Some(Box::new(ConstValue::Int(7.into())))
    );
}
