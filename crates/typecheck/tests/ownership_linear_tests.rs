use ast::ty::{NamedTypeInstance, TypeId};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::analysis::TyOwnershipExt;
use typecheck::transform::{TransformCtx, transform};
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn codes(source: &str) -> Vec<String> {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
    .all_flaws()
    .into_iter()
    .map(|(_, flaw)| flaw.code.slug().to_string())
    .collect()
}

#[test]
fn non_singleton_anonymous_integer_moves_at_owned_call() {
    let diagnostics = codes(
        "source() is in 0...10 extern\ntake(value x): 0\nmain:\n\tvalue: source()\n\ttake(value)\n\ttake(value)\n\treturn 0",
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
            "take(value x): 0\nmain:\n\tvalue: 7\n\ttake(value)\n\ttake(value)\n\treturn 0",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let diagnostics: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect();
    let BindBody::Body { exprs, .. } = &typed.defs[&DefId(Intern::from_ref("main"))].body else {
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
            "Bool is True or False\ncoin() Bool extern\ntake(value x): 0\nmain:\n\tvalue: when coin() is True then 7 else 7\n\ttake(value)\n\ttake(value)\n\treturn 0",
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
            } if target.0.as_str() == "take" => Some(arguments[0]),
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
            "Bool is True or False\ncoin() Bool extern\ntake(value x): 0\nmain:\n\tvalue: when coin() is True then 1 else 2\n\tif value is 1\n\t\ttake(value)\n\t\ttake(value)\n\treturn\n\treturn 0",
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
            } if target.0.as_str() == "take" => Some(arguments[0]),
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
            "Bool is True or False\ntake(value x): 0\nmain:\n\tvalue: when True is True then 7 else 8\n\ttake(value)\n\ttake(value)\n\treturn 0",
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
            } if target.0.as_str() == "take" => Some(arguments[0]),
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
    let diagnostics = codes(
        "Only is in 1...1\ntake(value x): 0\nmain(value Only):\n\ttake(value)\n\ttake(value)\n\treturn 0",
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn exact_anonymous_aggregate_still_moves() {
    let diagnostics =
        codes("take(value x): 0\nmain:\n\tvalue: (1, 2)\n\ttake(value)\n\ttake(value)\n\treturn 0");

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn different_singleton_join_still_moves() {
    let diagnostics = codes(
        "Bool is True or False\ncoin() Bool extern\ntake(value x): 0\nmain:\n\tvalue: when coin() is True then 1 else 2\n\ttake(value)\n\ttake(value)\n\treturn 0",
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
    let BindBody::Expr(expression) = typed.defs[&DefId(Intern::from_ref("main"))].body else {
        panic!("expected expression body");
    };
    typed.exprs.const_value[expression.as_usize()] = None;
    typed.exprs.integer_knowledge[expression.as_usize()] = Some(
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
    let diagnostics = codes(
        "Payload has value Pointer(Int)\nOption(Payload) is Some(Payload) or None\ntake(value x): 0\nmain:\n\tvalue: Some(Payload(value: @7))\n\ttake(value)\n\ttake(value)\n\treturn 0",
    );

    assert!(
        diagnostics.contains(&"type-use-after-move".to_string()),
        "{diagnostics:?}"
    );
}

#[test]
fn conversion_consumes_a_nonrematerializable_anonymous_integer() {
    let diagnostics = codes(
        "Bool is True or False\nTiny is in 0...5\ncoin() Bool extern\nmain:\n\tvalue: when coin() is True then 1 else 2\n\tfirst: value as Tiny\n\tsecond: value as Tiny\n\treturn 0",
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
            "Tiny is in 0...7\nmain:\n\tvalue: 7\n\tfirst: value as Tiny\n\tsecond: value as Tiny\n\treturn 0",
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
    let diagnostics = codes(
        "Payload has value Pointer(Int)\nOption(Payload) is Some(Payload) or None\ndrop(eat value Option(Payload)): 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(payload) then drop(eat value)\n\t\tNone then drop(eat value)\n\treturn 0",
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
    let diagnostics = codes(
        "Payload has value Pointer(Int)\nOption(Payload) is Some(Payload) or None\ndrop(eat value Option(Payload)): 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(ref payload) then drop(eat value)\n\t\tNone then drop(eat value)\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn mutable_pattern_binding_preserves_the_subject() {
    let diagnostics = codes(
        "Payload has value Pointer(Int)\nOption(Payload) is Some(Payload) or None\ndrop(eat value Option(Payload)): 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(mut payload) then drop(eat value)\n\t\tNone then drop(eat value)\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn nonbinding_pattern_preserves_the_subject() {
    let diagnostics = codes(
        "Payload has value Pointer(Int)\nOption(Payload) is Some(Payload) or None\ndrop(eat value Option(Payload)): 0\ninspect(value Option(Payload)):\n\twhen value is\n\t\tSome(_) then drop(eat value)\n\t\tNone then drop(eat value)\n\treturn 0",
    );

    assert!(
        !diagnostics.iter().any(|code| code.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn anonymous_products_are_discardable_but_nominal_records_are_not() {
    let anonymous = Ty::Tuple(vec![
        Ty::anonymous_integer_for_width(8, false),
        Ty::anonymous_integer_for_width(8, false),
    ]);
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
