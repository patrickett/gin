mod support;

use support::check_lexed;
use typecheck::{
    TypedFileAst,
    typed::{BindBody, TypedExprKind},
};

fn check_return(case: &str, typed: &TypedFileAst, compatible: bool) {
    let flaws = typed.all_flaws();
    if compatible {
        assert!(flaws.is_empty(), "case `{case}` failed: {flaws:?}");
    } else {
        assert!(
            flaws
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-return-mismatch"),
            "case `{case}` failed: {flaws:?}"
        );
    }
    let callee = typed
        .defs
        .values()
        .find(|bind| bind.name.as_str().ends_with("take"))
        .unwrap();
    let caller = typed
        .defs
        .values()
        .find(|bind| bind.name.as_str().ends_with("use_value"))
        .unwrap();
    let BindBody::Expr(body) = &caller.body else {
        panic!("case `{case}`: expected expression body")
    };
    let TypedExprKind::FnCall {
        substituted_ty: Some(actual),
        ..
    } = &typed.exprs.kind[body.index()]
    else {
        panic!("case `{case}`: expected a call with a substituted return type")
    };
    assert_eq!(actual, &callee.return_type);
    if compatible {
        assert_eq!(typed.exprs.ty[body.index()], callee.return_type);
    }
}

const CASES: &[(&str, &str, bool)] = &[
    (
        "expected_return_context_preserves_concrete_lengths",
        "#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\ntake(value Buffer(Byte, 4)) Buffer(Byte, 4): return value\nuse_value(value Buffer(Byte, 4)) Buffer(Byte, 4): return take(value)\n",
        true,
    ),
    (
        "expected_return_context_preserves_concrete_lengths",
        "#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\ntake(value Buffer(Byte, 4)) Buffer(Byte, 4): return value\nuse_value(value Buffer(Byte, 4)) Buffer(Byte, 8): return take(value)\n",
        false,
    ),
    (
        "expected_return_context_preserves_concrete_element_types",
        "Wide is in 0 to 65535\nWide has\n    finish(self):\n        return\nBox(element) has\n    value element\n    finish(self):\n        return\ntake(value Box(Byte)) Box(Byte): return value\nuse_value(value Box(Byte)) Box(Byte): return take(value)\n",
        true,
    ),
    (
        "expected_return_context_preserves_concrete_element_types",
        "Wide is in 0 to 65535\nWide has\n    finish(self):\n        return\nBox(element) has\n    value element\n    finish(self):\n        return\ntake(value Box(Byte)) Box(Byte): return value\nuse_value(value Box(Byte)) Box(Wide): return take(value)\n",
        false,
    ),
];

#[test]
fn expected_return_context_cases() {
    for &(name, body, compatible) in CASES {
        let source = format!("{}{body}", test_fixtures::preludes::BYTE_PRELUDE);
        check_return(name, &check_lexed(&source), compatible);
    }
}
