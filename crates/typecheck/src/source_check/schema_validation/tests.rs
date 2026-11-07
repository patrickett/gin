use super::*;

const PACKETS: &str = concat!(
    "Box(element) has\n    #private\n    value element\n    lower in 0 to 9\n",
    "    upper in 0 to 9 and >= lower and <= 8\n",
    "Empty has\n",
    "Pair has left Box(Empty), right Box(Empty)\n",
    "Sum is Empty or Pair\n",
);

// Exercise the schema boundary directly, without unrelated module/body work.
fn prepared(source: &SourceFile) -> Checker<'_> {
    let mut checker = Checker::new(source, crate::Limits::default());
    checker.collect().unwrap();
    checker.prepare_layouts().unwrap();
    checker
        .module
        .type_names
        .sort_by(|left, right| left.0.cmp(&right.0));
    checker
}

fn assert_shells(checker: &Checker<'_>) {
    for layout in &checker.module.layouts {
        assert!(layout.schema.fields.is_empty());
        assert!(layout.schema.variants.is_empty());
        assert!(layout.constraints.is_empty());
    }
}

#[test]
fn validation_stages_ordered_fields_privacy_and_constraints_without_installing() {
    let source = parser::parse_strict(PACKETS).unwrap();
    let mut checker = prepared(&source);
    let schemas = checker.validate_source_schemas().unwrap();
    assert_shells(&checker);
    assert_eq!(
        schemas
            .entries
            .iter()
            .map(|entry| entry.schema.name.as_str())
            .collect::<Vec<_>>(),
        ["Box", "Empty", "Pair", "Sum"],
    );
    let boxed = &schemas.entries[0];
    assert_eq!(boxed.schema.parameters, ["element"]);
    assert_eq!(boxed.schema.fields.len(), 3);
    assert_eq!(boxed.schema.fields[0].name, "value");
    assert_eq!(boxed.schema.fields[0].ty, SourceType::Generic { index: 0 });
    assert!(boxed.schema.fields[0].private);
    assert!(!boxed.schema.fields[1].private);
    assert!(!boxed.schema.fields[2].private);
    assert_eq!(boxed.constraints.len(), 2);
    let relation = boxed.constraints[0].clone();
    assert_eq!(relation.left, 2);
    assert_eq!(relation.operator, BinaryOperator::GreaterOrEqual);
    assert!(matches!(relation.right, contracts::Term::Slot(1)));
    let bound = boxed.constraints[1].clone();
    assert_eq!(bound.left, 2);
    assert_eq!(bound.operator, BinaryOperator::LessOrEqual);
    assert!(matches!(bound.right, contracts::Term::Constant(value) if value == I256::from(8)));
    assert!(schemas.entries[1].schema.fields.is_empty());
    assert_eq!(schemas.entries[2].schema.fields.len(), 2);
    assert_eq!(schemas.entries[3].schema.variants.len(), 2);
    assert!(
        schemas.entries[1..]
            .iter()
            .all(|entry| entry.constraints.is_empty())
    );

    let origins: Vec<_> = checker
        .module
        .layouts
        .iter()
        .map(|layout| layout.origin.clone())
        .collect();
    let accounting = (checker.remaining, checker.flow_remaining);
    checker.compile_layouts(schemas);
    assert_eq!((checker.remaining, checker.flow_remaining), accounting);
    for (layout, origin) in checker.module.layouts.iter().zip(origins) {
        assert_eq!(layout.origin, origin);
    }
    checker.validate_layouts().unwrap();
}

#[test]
fn later_constraint_failure_leaves_even_completed_packets_uninstalled() {
    let source = parser::parse_strict(concat!(
        "First has value in 0 to 9 and >= 1\n",
        "Second has value in 0 to 9 and >= absent\n",
    ))
    .unwrap();
    let mut checker = prepared(&source);
    assert_eq!(
        checker
            .validate_source_schemas()
            .err()
            .map(|failure| failure.error),
        Some(SourceCheckError::UnknownName),
    );
    assert_shells(&checker);
    assert_eq!(checker.proof.next_identity, 0);
    assert!(checker.proof.names.is_empty());
}

#[test]
fn all_annotation_errors_precede_any_constraint_errors() {
    for (later, expected) in [
        ("Second has missing\n", SourceCheckError::Type),
        ("Second has value Unknown\n", SourceCheckError::Unsupported),
        (
            "Second has value in 0 to 9, value in 0 to 9\n",
            SourceCheckError::Duplicate,
        ),
    ] {
        let text = format!("First has value in 0 to 9 and >= absent\n{later}");
        let source = parser::parse_strict(&text).unwrap();
        let mut checker = prepared(&source);
        assert_eq!(
            checker
                .validate_source_schemas()
                .err()
                .map(|failure| failure.error),
            Some(expected),
        );
        assert_shells(&checker);
    }
}

#[test]
fn quota_exhaustion_in_repeated_field_validation_precedes_constraint_lookup() {
    let source = parser::parse_strict(
        "Record has\n    #private\n    value in 0 to 9 and >= absent\n",
    )
    .unwrap();
    let mut measured = prepared(&source);
    let start = measured.remaining;
    let schemas = measured.resolve_source_schemas().unwrap();
    let first_pass_work = start - measured.remaining;
    measured.charge(schemas.len()).unwrap();
    measured.source_fields(0).unwrap();
    let repeated_fields_work = start - measured.remaining;
    assert!(repeated_fields_work > first_pass_work + schemas.len() as u32);

    // A budget covering only the first pass must not skip the second field walk.
    for work in [
        first_pass_work,
        repeated_fields_work - 1,
        repeated_fields_work,
    ] {
        let mut checker = prepared(&source);
        checker.remaining = work;
        assert_eq!(
            checker
                .validate_source_schemas()
                .err()
                .map(|failure| failure.error),
            Some(SourceCheckError::Budget),
        );
        assert_shells(&checker);
    }
    let mut checker = prepared(&source);
    assert_eq!(
        checker
            .validate_source_schemas()
            .err()
            .map(|failure| failure.error),
        Some(SourceCheckError::UnknownName),
    );
    assert_shells(&checker);
}

#[test]
fn packets_install_and_feed_layout_consumers_without_source() {
    let source = parser::parse_strict(PACKETS).unwrap();
    let empty = parser::parse_strict("").unwrap();
    let mut producer = prepared(&source);
    let schemas = producer.validate_source_schemas().unwrap();
    // Transfer only source-free storage; no raw declaration or source survives
    // in the consumer. IDs stay in their original arena, with no reinterning.
    let mut consumer = Checker::new(&empty, crate::Limits::default());
    consumer.module.layouts = producer.module.layouts;
    consumer.module.instances = producer.module.instances;
    consumer.module.instance_index = producer.module.instance_index;
    assert!(consumer.module.schemas.is_empty());
    assert!(consumer.module.substitutions.is_empty());
    consumer.remaining = 0;
    consumer.flow_remaining = 0;
    schemas.install(&mut consumer.module.layouts);
    assert_eq!((consumer.remaining, consumer.flow_remaining), (0, 0));
    assert!(consumer.module.substitutions.is_empty());

    consumer.remaining = crate::Limits::default().work_max;
    let pair = consumer.instance(2, Vec::new()).unwrap();
    let SourceType::Aggregate { id } = pair else {
        panic!("record instance")
    };
    assert_eq!(
        consumer.instance_fields(id).unwrap(),
        [SourceType::Aggregate { id: 1 }; 2],
    );
    let sum = consumer.instance(3, Vec::new()).unwrap();
    let SourceType::Sum { id } = sum else {
        panic!("sum instance")
    };
    assert_eq!(
        consumer.sum_variants(id).unwrap(),
        [SourceType::Aggregate { id: 0 }, pair],
    );
    assert_eq!(consumer.schema_access(0), Err(SourceCheckError::Privacy));
    assert_eq!(consumer.schema_access(2), Ok(()));
}

#[test]
#[should_panic(expected = "assertion `left == right` failed")]
fn installation_rejects_a_different_number_of_shells() {
    let source = parser::parse_strict("Empty has\n").unwrap();
    let mut checker = prepared(&source);
    let schemas = checker.validate_source_schemas().unwrap();
    schemas.install(&mut []);
}
