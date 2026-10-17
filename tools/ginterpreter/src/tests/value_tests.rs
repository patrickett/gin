use super::*;
use ast::{HashFloat, ty::Ty};
use std::sync::Arc;

fn inspect(value: ConstValue) -> ValueInspection {
    EvaluatedValue::from_constant(value).inspection(InspectionLimits::default())
}

#[test]
fn scalar_values_are_lossless_and_serializable() {
    for (value, kind, scalar, display) in [
        (
            ConstValue::Proposition(true),
            ValueKind::Proposition,
            "true",
            "holds",
        ),
        (
            ConstValue::Proposition(false),
            ValueKind::Proposition,
            "false",
            "does not hold",
        ),
        (
            ConstValue::Int(u128::MAX.into()),
            ValueKind::Integer,
            "340282366920938463463374607431768211455",
            "340282366920938463463374607431768211455",
        ),
        (
            ConstValue::Float(HashFloat(1.5)),
            ValueKind::Float,
            "1.5",
            "1.5",
        ),
        (
            ConstValue::Float(HashFloat(f64::INFINITY)),
            ValueKind::Float,
            "inf",
            "inf",
        ),
        (
            ConstValue::Text("héllo".to_owned()),
            ValueKind::Text,
            "héllo",
            "héllo",
        ),
    ] {
        let inspected = inspect(value);
        assert_eq!(inspected.nodes[0].kind, kind);
        assert_eq!(inspected.nodes[0].value.as_deref(), Some(scalar));
        assert_eq!(inspected.display, display);
        assert!(!inspected.truncated);
        let json = serde_json::to_value(inspected).unwrap();
        assert_eq!(json["nodes"][0]["value"], scalar);
    }
}

#[test]
fn structured_values_preserve_child_order_and_field_names() {
    let value = ConstValue::Record {
        fields: vec![
            ("second".to_owned().into(), ConstValue::Int(2.into())),
            (
                "first".to_owned().into(),
                ConstValue::Tag {
                    name: "Some".to_owned().into(),
                    qual_path: None,
                    args: vec![ConstValue::Text("a\n\"b".to_owned())].into(),
                },
            ),
        ]
        .into(),
    };
    let inspected = inspect(value.clone());
    assert_eq!(inspected.display, "{second: 2, first: Some(\"a\\n\\\"b\")}");
    assert_eq!(
        inspected.nodes[0].children[0].name.as_deref(),
        Some("second")
    );
    assert_eq!(
        inspected.nodes[0].children[1].name.as_deref(),
        Some("first")
    );
    assert_eq!(inspected, inspect(value));
    assert!(!inspected.truncated);
}

#[test]
fn sequence_repeats_keep_symbolic_counts_without_expansion() {
    let value = ConstValue::SequenceRepeat {
        seed: Box::new(ConstValue::Int(7.into())),
        count: Box::new(NormalExpr::Value(ConstValue::Int(u128::MAX.into()))),
    };
    let inspected = inspect(value);
    assert_eq!(inspected.nodes.len(), 3);
    assert_eq!(inspected.nodes[0].kind, ValueKind::SequenceRepeat);
    assert_eq!(
        inspected.nodes[2].value.as_deref(),
        Some("340282366920938463463374607431768211455")
    );
    assert!(!inspected.truncated);
}

#[test]
fn aggregate_kinds_and_nominal_identity_are_distinct() {
    let items: Arc<[ConstValue]> = vec![ConstValue::ONE, ConstValue::ZERO].into();
    for (value, kind) in [
        (
            ConstValue::ResultRow(Arc::clone(&items)),
            ValueKind::ResultRow,
        ),
        (
            ConstValue::FixedArray(Arc::clone(&items)),
            ValueKind::FixedArray,
        ),
        (ConstValue::Sequence(items), ValueKind::Sequence),
        (
            ConstValue::Tag {
                name: "Ready".to_owned().into(),
                qual_path: Some("package.child".to_owned()),
                args: Arc::from([]),
            },
            ValueKind::Tag,
        ),
        (
            ConstValue::ResultAlternative {
                owner: ast::ResultFamilyOwner::LocalCallable(ast::BinderId::new(
                    0,
                    ast::BinderOwner::Expression(1),
                )),
                label: "Found".to_owned().into(),
            },
            ValueKind::ResultAlternative,
        ),
        (
            ConstValue::Domain(Arc::new(
                ast::integer::IntegerDomain::bounded(0.into(), 255.into()).unwrap(),
            )),
            ValueKind::Domain,
        ),
    ] {
        let result = EvaluatedValue::from_constant(value.clone());
        let inspected = result.inspection(InspectionLimits::default());
        assert_eq!(inspected.nodes[0].kind, kind);
        assert_eq!(result.storage, RuntimeStorage::Constant(value));
        if kind == ValueKind::Tag {
            assert_eq!(
                inspected.nodes[0].value.as_deref(),
                Some("package.child.Ready")
            );
            assert!(!inspected.truncated);
        }
    }
}

#[test]
fn bounded_views_do_not_discard_the_original_value() {
    let text = "é🦀".repeat(2000);
    let value = ConstValue::Sequence(vec![ConstValue::Text(text.clone()); 1000].into());
    let result = EvaluatedValue::from_constant(value.clone());
    let limits = InspectionLimits {
        depth_max: 1,
        nodes_max: 3,
        text_bytes_max: 64,
    };
    let inspected = result.inspection(limits);
    assert_eq!(result.storage, RuntimeStorage::Constant(value));
    assert_eq!(inspected.nodes.len(), 3);
    assert_eq!(inspected.nodes[0].total_children, 1000);
    assert!(inspected.truncated);
    let bytes: usize = inspected
        .nodes
        .iter()
        .map(|node| {
            node.value.as_ref().map_or(0, String::len)
                + node
                    .children
                    .iter()
                    .filter_map(|child| child.name.as_ref())
                    .map(String::len)
                    .sum::<usize>()
        })
        .sum();
    assert!(bytes + inspected.type_name.len() + inspected.display.len() <= 64);
}

#[test]
fn depth_and_node_limits_apply_without_recursion() {
    let mut value = ConstValue::Int(7.into());
    for _ in 0..128 {
        value = ConstValue::Sequence(Arc::from([value]));
    }
    let result = EvaluatedValue::from_constant(value);
    let inspected = result.inspection(InspectionLimits {
        depth_max: 2,
        nodes_max: 99,
        text_bytes_max: 256,
    });
    assert_eq!(inspected.nodes.len(), 3);
    assert!(inspected.nodes[2].truncated);
    assert_eq!(inspected.display, "[[[…]]]");
    let empty = result.inspection(InspectionLimits {
        depth_max: 0,
        nodes_max: 0,
        text_bytes_max: 0,
    });
    assert_eq!(empty.nodes.len(), 1);
    assert_eq!(empty.display, "");
    assert!(empty.truncated);
}

#[test]
fn unit_requires_the_compiler_type() {
    let empty_record = ConstValue::Record {
        fields: Arc::from([]),
    };
    assert_eq!(
        inspect(empty_record.clone()).nodes[0].kind,
        ValueKind::Record
    );
    let unit = EvaluatedValue::new(empty_record, Some(Ty::Unit));
    let inspected = unit.inspection(InspectionLimits::default());
    assert_eq!(inspected.nodes[0].kind, ValueKind::Unit);
    assert_eq!(inspected.display, "completed (no value)");
    assert_eq!(inspected.type_name, "unit");
}

#[test]
fn text_limits_keep_valid_utf8_and_report_partial_views() {
    let value = EvaluatedValue::from_constant(ConstValue::Text("🦀é".repeat(2000)));
    for bytes_max in 0..20 {
        let inspected = value.inspection(InspectionLimits {
            depth_max: 1,
            nodes_max: 1,
            text_bytes_max: bytes_max,
        });
        assert_eq!(inspected.nodes[0].kind, ValueKind::Text);
        assert!(inspected.nodes[0].truncated);
        assert!(inspected.truncated);
        let scalar = inspected.nodes[0].value.as_deref().unwrap();
        assert!(scalar.len() + inspected.display.len() <= bytes_max as usize);
    }
}

#[test]
fn small_budgets_never_emit_partial_integer_scalars() {
    let result = EvaluatedValue::from_constant(ConstValue::Int(u128::MAX.into()));
    let inspected = result.inspection(InspectionLimits {
        depth_max: 1,
        nodes_max: 1,
        text_bytes_max: 8,
    });
    assert_eq!(inspected.nodes[0].kind, ValueKind::Integer);
    assert_eq!(inspected.nodes[0].value, None);
    assert!(inspected.truncated);
}
