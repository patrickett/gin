mod support;

use ast::ty::{NamedTypeInstance, TypeId};
use internment::Intern;
use parser::cursor::TokenCursor;
use support::flaw_codes_of;
use typecheck::analysis::TyOwnershipExt;
use typecheck::transform::{TransformCtx, transform};
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

#[test]
fn non_singleton_anonymous_integer_moves_at_owned_call() {
    let diagnostics = flaw_codes_of(
        r#"source() is in 0 to 10: return 0
take(value x): return 0
main:
	value: source()
	take(value)
	take(value)
	return 0"#,
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn exact_anonymous_integer_can_be_rematerialized_at_owned_calls() {
    let typed = transform(
        &TokenCursor::parse_source(
            "take(value x): return 0\nmain:\n\tvalue: 7\n\ttake(value)\n\ttake(value)\n\treturn 0",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let diagnostics: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.to_string())
        .collect();
    let BindBody::Body { exprs, .. } = &typed.defs[&DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected main body");
    };

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
    for call in &exprs[1..=2] {
        let TypedExprKind::FnCall {
            args: Some(arguments),
            ..
        } = &typed.exprs.kind[call.as_usize()]
        else {
            panic!("expected owned call");
        };
        let TypedExprKind::Rematerialize {
            constant: ast::ConstValue::Int(value),
            ..
        } = &typed.exprs.kind[arguments[0].as_usize()]
        else {
            panic!("expected integer rematerialization");
        };
        assert_eq!(*value, 7.into());
    }
}

#[test]
fn equal_singleton_join_can_be_rematerialized() {
    let typed = transform(
        &TokenCursor::parse_source(
            r#"Bool is True or False
coin() Bool: return 0
take(value x): return 0
main:
	value: when coin() is True then 7 else 7
	take(value)
	take(value)
	return 0"#,
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let arguments: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::FnCall {
                target,
                args: Some(arguments),
                ..
            } if target.0.as_str().starts_with("take$specialized$") => Some(arguments[0]),
            _ => None,
        })
        .collect();

    assert_eq!(arguments.len(), 2);
    assert!(arguments.iter().all(|argument| matches!(
        typed.exprs.kind[argument.as_usize()],
        TypedExprKind::Rematerialize { .. }
    )));
}

#[test]
fn flow_singleton_can_be_rematerialized_inside_the_proven_branch() {
    let typed = transform(
        &TokenCursor::parse_source(
            r#"Bool is True or False
coin() Bool: return 0
take(value x): return 0
main:
	value: when coin() is True then 1 else 2
	when value is 1 then
		take(value)
		take(value)
		return
	return 0"#,
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let arguments: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::FnCall {
                target,
                args: Some(arguments),
                ..
            } if target.0.as_str().starts_with("take$specialized$") => Some(arguments[0]),
            _ => None,
        })
        .collect();

    assert_eq!(arguments.len(), 2);
    assert!(arguments.iter().all(|argument| matches!(
        typed.exprs.kind[argument.as_usize()],
        TypedExprKind::Rematerialize { .. }
    )));
}

#[test]
fn folded_control_expression_can_be_rematerialized() {
    let typed = transform(
        &TokenCursor::parse_source(
            "Bool is True or False\ntake(value x): return 0\nmain:\n\tvalue: when True is True then 7 else 8\n\ttake(value)\n\ttake(value)\n\treturn 0",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let arguments: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::FnCall {
                target,
                args: Some(arguments),
                ..
            } if target.0.as_str().starts_with("take$specialized$") => Some(arguments[0]),
            _ => None,
        })
        .collect();

    assert_eq!(arguments.len(), 2);
    assert!(arguments.iter().all(|argument| matches!(
        typed.exprs.kind[argument.as_usize()],
        TypedExprKind::Rematerialize { .. }
    )));
}

#[test]
fn nominal_singleton_is_never_rematerializable() {
    let diagnostics = flaw_codes_of(
        "Only is in 1 to 1\ntake(value x): return 0\nmain(value Only):\n\ttake(value)\n\ttake(value)\n\treturn 0",
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn different_singleton_join_still_moves() {
    let diagnostics = flaw_codes_of(
        r#"Bool is True or False
coin() Bool: return 0
take(value x): return 0
main:
	value: when coin() is True then 1 else 2
	take(value)
	take(value)
	return 0"#,
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn symbolic_anonymous_integer_is_not_rematerializable() {
    let mut typed = transform(
        &TokenCursor::parse_source("main: 7"),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(expression) = typed.defs[&DefId::named(Intern::from_ref("main"))].body
    else {
        panic!("expected expression body");
    };
    typed.exprs.facts.const_facts.const_value[expression.as_usize()] = None;
    typed.exprs.facts.semantic_facts.integer_knowledge[expression.as_usize()] = Some(
        ast::integer::IntegerKnowledge::Exact(ast::integer::CanonicalIntegerExpr::Symbolic(
            ast::NormalExpr::Var(Intern::from_ref("value")),
        )),
    );

    assert!(!typecheck::analysis::expr_is_rematerializable(
        &typed, expression
    ));
}

#[test]
fn payload_alternative_still_moves() {
    let diagnostics = flaw_codes_of(
        "Payload has\n    value @Int\n    finish(self):\n    \treturn\nOption(element) is Some(element) or None\nOption(element) has\n    finish(self):\n    \treturn\ntake(value x): return 0\nmain:\n\tvalue: Some(Payload(value: @7))\n\ttake(value)\n\ttake(value)\n\treturn 0",
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn moving_named_result_field_twice_is_rejected() {
    let diagnostics = flaw_codes_of(
        "Int is in 0 to 255\nPayload has\n    pointer @Int\n    finish(self):\n    \treturn\npair() left Payload, right Payload:\n\treturn Payload(pointer: @1), Payload(pointer: @2)\ndrop(value Payload): return 0\nmain:\n\tresult: pair()\n\tdrop(result.left)\n\tdrop(result.left)\n\treturn 0",
    );

    assert!(
        diagnostics.contains(&"type-use-of-moved-value".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn moving_named_result_field_preserves_its_sibling() {
    let diagnostics = flaw_codes_of(
        "Int is in 0 to 255\nPayload has\n    pointer @Int\n    finish(self):\n    \treturn\npair() left Payload, right Payload:\n\treturn Payload(pointer: @1), Payload(pointer: @2)\ndrop(value Payload): return 0\nmain:\n\tresult: pair()\n\tdrop(result.left)\n\tdrop(result.right)\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn conversion_consumes_a_nonrematerializable_anonymous_integer() {
    let diagnostics = flaw_codes_of(
        r#"Bool is True or False
Tiny is in 0 to 5
coin() Bool: return 0
main:
	value: when coin() is True then 1 else 2
	first: value as Tiny
	second: value as Tiny
	return 0"#,
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn conversion_rematerializes_an_exact_anonymous_integer() {
    let typed = transform(
        &TokenCursor::parse_source(
            "Tiny is in 0 to 7\nmain:\n\tvalue: 7\n\tfirst: value as Tiny\n\tsecond: value as Tiny\n\treturn 0",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let casts: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::Cast { expr, .. } => Some(*expr),
            _ => None,
        })
        .collect();

    assert_eq!(casts.len(), 2);
    assert!(casts.iter().all(|expr| matches!(
        typed.exprs.kind[expr.as_usize()],
        TypedExprKind::Rematerialize { .. }
    )));
    assert!(
        !typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug().contains("move"))
    );
}

#[test]
fn by_value_pattern_binding_moves_only_the_matching_path() {
    let diagnostics = flaw_codes_of(
        "Payload has\n    value @Int\n    finish(self):\n    \treturn\nOption(element) is Some(element) or None\nOption(element) has\n    finish(self):\n    \treturn\ndrop(value Option(Payload)): return 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(payload) then drop(value)\n\t\tNone then 0\n\treturn 0",
    );

    assert_eq!(
        diagnostics
            .iter()
            .filter(|code| code.as_str() == "type-use-of-moved-value")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn reference_pattern_binding_preserves_the_subject() {
    let diagnostics = flaw_codes_of(
        "Payload has\n    value @Int\n    finish(self):\n    \treturn\nOption(element) is Some(element) or None\nOption(element) has\n    finish(self):\n    \treturn\ndrop(value Option(Payload)): return 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(ref payload) then drop(value)\n\t\tNone then 0\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn mutable_pattern_binding_preserves_the_subject() {
    let diagnostics = flaw_codes_of(
        "Payload has\n    value @Int\n    finish(self):\n    \treturn\nOption(element) is Some(element) or None\nOption(element) has\n    finish(self):\n    \treturn\ndrop(value Option(Payload)): return 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(mut payload) then drop(value)\n\t\tNone then 0\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn nonbinding_pattern_preserves_the_subject() {
    let diagnostics = flaw_codes_of(
        "Payload has\n    value @Int\n    finish(self):\n    \treturn\nOption(element) is Some(element) or None\nOption(element) has\n    finish(self):\n    \treturn\ndrop(value Option(Payload)): return 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(_) then drop(value)\n\t\tNone then 0\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn structural_records_are_discardable_but_nominal_records_are_not() {
    let anonymous = Ty::Record {
        name: Intern::from_ref(""),
        fields: vec![
            (
                Intern::from_ref("0"),
                Box::new(Ty::anonymous_integer_for_width(8, false)),
            ),
            (
                Intern::from_ref("1"),
                Box::new(Ty::anonymous_integer_for_width(8, false)),
            ),
        ],
        resolved_params: None,
    };
    let nominal = Ty::Named {
        instance: NamedTypeInstance::new(TypeId {
            file: 0,
            name: Intern::from_ref("Pair"),
        }),
        name: Intern::from_ref("Pair"),
    };

    assert!(anonymous.is_structurally_discardable());
    assert!(!nominal.is_structurally_discardable());
}
