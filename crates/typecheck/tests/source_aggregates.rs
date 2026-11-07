use typecheck::{
    CheckedSourceFile, CheckedSourceInstanceHead, Limits, SourceCheckError, SourceType, check_file,
};

const WINDOW: &str = include_str!("../examples/window-source.gin");
const PAIR: &str = "Pair(element) has\n    left element\n    right element\n";

fn run(text: &str) -> Result<CheckedSourceFile, typecheck::SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid new source syntax");
    let before = source.clone();
    let result = check_file(&source, Limits::default());
    assert_eq!(source, before);
    result
}

fn accepts(text: &str) -> CheckedSourceFile {
    run(text).unwrap_or_else(|error| panic!("{error:?}\n{text}"))
}

fn rejects(text: &str, expected: &str) {
    let error = run(text).expect_err("invalid body must not be verified");
    let expected = match expected {
        "Type" => SourceCheckError::Type,
        "Unproved" => SourceCheckError::Unproved,
        "UseAfterMove" => SourceCheckError::UseAfterMove,
        "Unconsumed" => SourceCheckError::Unconsumed,
        _ => panic!("unknown expected semantic diagnostic"),
    };
    assert_eq!(error, expected, "{text}");
}

#[test]
fn actual_window_bodies_are_verified_not_only_parsed() {
    let checked = accepts(WINDOW);
    let names: Vec<_> = checked.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["make_window", "rebuild_window", "take_window"]);
    for function in &checked.functions {
        assert!(!matches!(
            &function.result,
            SourceType::Unit | SourceType::Integer { .. }
        ));
    }
}

#[test]
fn aggregate_result_ids_resolve_to_owned_schemas_and_instances() {
    let checked = accepts(WINDOW);
    let schema = checked
        .aggregates
        .iter()
        .position(|s| s.name == "Window")
        .unwrap();
    let definition = &checked.aggregates[schema];
    assert_eq!(definition.parameters, ["element"]);
    let fields: Vec<_> = definition.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(fields, ["payload", "lower", "upper"]);
    assert!(definition.fields.iter().all(|field| field.private));
    assert_eq!(definition.fields[0].ty, SourceType::Generic { index: 0 });
    for function in &checked.functions {
        assert_eq!(function.type_parameters, ["element"]);
    }
    assert_eq!(checked.functions[0].parameters.len(), 3);
    assert_eq!(checked.functions[1].parameters.len(), 1);
    assert_eq!(checked.functions[2].parameters.len(), 1);
    for function in &checked.functions[..2] {
        let SourceType::Aggregate { id } = function.result else {
            panic!("nominal result");
        };
        let instance = &checked.instances[id as usize];
        assert_eq!(
            instance.head,
            CheckedSourceInstanceHead::Nominal {
                schema: schema as u32
            }
        );
        assert_eq!(instance.arguments, [SourceType::Generic { index: 0 }]);
    }
    for instance in &checked.instances {
        let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
            panic!("nominal instance");
        };
        let schema = &checked.aggregates[schema as usize];
        assert_eq!(instance.arguments.len(), schema.parameters.len());
        for argument in &instance.arguments {
            if let SourceType::Aggregate { id } = argument {
                assert!((*id as usize) < checked.instances.len());
            }
        }
    }
}

#[test]
fn unrelated_type_field_and_function_names_have_the_same_capabilities() {
    let renamed = WINDOW
        .replace("Window", "Envelope")
        .replace("window", "envelope")
        .replace("payload", "cargo")
        .replace("lower", "start")
        .replace("upper", "finish")
        .replace("element", "content");
    let original = accepts(WINDOW);
    let changed = accepts(&renamed);
    assert_eq!(original.functions.len(), changed.functions.len());
    for (left, right) in original.functions.iter().zip(&changed.functions) {
        assert_eq!(left.parameters.len(), right.parameters.len());
        assert_eq!(
            std::mem::discriminant(&left.result),
            std::mem::discriminant(&right.result)
        );
    }
}

#[test]
fn a_body_only_bad_field_relation_fails_its_proof() {
    let invalid = WINDOW.replacen(
        "return Window(payload: payload, lower: lower, upper: upper)",
        "return Window(payload: payload, lower: lower, upper: 0)",
        1,
    );
    rejects(&invalid, "Unproved");
}

#[test]
fn constructor_argument_order_does_not_replace_actual_field_facts() {
    let reordered = WINDOW.replace(
        "Window(payload: payload, lower: lower, upper: upper)",
        "Window(upper: upper, payload: payload, lower: lower)",
    );
    accepts(&reordered);
    let invalid = reordered.replacen("Window(upper: upper,", "Window(upper: 0,", 1);
    rejects(&invalid, "Unproved");
}

#[test]
fn generic_inputs_cannot_be_abandoned() {
    rejects(
        "forget(element Type, payload element): return\n",
        "Unconsumed",
    );
    accepts("identity(element Type, payload element) element: return payload\n");
}

#[test]
fn packing_does_not_duplicate_a_generic_payload() {
    let source = format!(
        "{PAIR}duplicate(element Type, payload element) Pair(element):
    return Pair(left: payload, right: payload)
"
    );
    rejects(&source, "UseAfterMove");
}

#[test]
fn an_aggregate_alias_transfers_instead_of_copying_the_owner() {
    let source = format!(
        "{PAIR}forward(element Type, pair Pair(element)) Pair(element):
    moved: pair
    return moved
"
    );
    accepts(&source);
    let invalid = source.replace("return moved", "return pair");
    rejects(&invalid, "UseAfterMove");
}

#[test]
fn two_owned_fields_can_be_unpacked_and_repacked_in_a_different_order() {
    let source = format!(
        "{PAIR}swap(element Type, pair Pair(element)) Pair(element):
    Pair(left, right) := pair
    return Pair(left: right, right: left)
"
    );
    accepts(&source);
    rejects(
        &source.replace("right: left", "right: right"),
        "UseAfterMove",
    );
}

#[test]
fn double_unpacking_and_returning_a_consumed_root_fail_at_the_use() {
    let source = format!(
        "{PAIR}bad(element Type, pair Pair(element)) Pair(element):
    Pair(left, right) := pair
    Pair(again_left, again_right) := pair
    return Pair(left: left, right: right)
"
    );
    rejects(&source, "UseAfterMove");
    let invalid = source.replace(
        "    Pair(again_left, again_right) := pair\n    return Pair(left: left, right: right)",
        "    return pair",
    );
    rejects(&invalid, "UseAfterMove");
}

#[test]
fn unpacked_children_keep_their_obligations() {
    let source = format!(
        "{PAIR}forget(element Type, pair Pair(element)):
    Pair(left, right) := pair
    return
"
    );
    rejects(&source, "Unconsumed");
}

#[test]
fn record_type_identity_is_not_just_field_shape() {
    let source = "\
First(element) has item element
Second(element) has item element
wrong(element Type, value First(element)) Second(element): return value
";
    rejects(source, "Type");
}

#[test]
fn nested_aggregate_payloads_survive_sequential_unpack_and_rebuild() {
    let source = format!(
        "{WINDOW}
nested(element Type, value Window(Window(element))) Window(Window(element)):
    Window(inner, lower, upper) := value
    Window(payload, inner_lower, inner_upper) := inner
    inside: Window(payload: payload, lower: inner_lower, upper: inner_upper)
    return Window(payload: inside, lower: lower, upper: upper)
"
    );
    accepts(&source);
}

#[test]
fn same_named_fields_on_distinct_inputs_do_not_share_proof_identity() {
    let source = format!(
        "{WINDOW}
cross(element Type, first Window(element), second Window(element)) Window(element):
    Window(first_payload, first_lower, first_upper) := first
    Window(second_payload, second_lower, second_upper) := second
    result: Window(payload: first_payload, lower: first_lower, upper: second_upper)
    return result
"
    );
    rejects(&source, "Unproved");
}

#[test]
fn empty_nominal_resources_are_not_discardable() {
    accepts("Marker has\nforward(marker Marker) Marker: return marker\n");
    accepts("Marker has\nfinish(marker Marker):\n    Marker() := marker\n    return\n");
    rejects("Marker has\nforget(marker Marker): return\n", "Unconsumed");
    rejects(
        "Marker has\nforget():\n    local: Marker()\n    return\n",
        "Unconsumed",
    );
    rejects(
        "Marker has\nforget():\n    Marker()\n    return\n",
        "Unconsumed",
    );
    rejects(
        "Marker has\nforge(element Type, payload element):\n    Marker() := payload\n    return\n",
        "Type",
    );
}

#[test]
fn field_shape_errors_do_not_silently_drop_arguments() {
    for initializer in [
        "Pair(left: left)",
        "Pair(left: left, right: right, extra: 0)",
        "Pair(left: left, left: right)",
    ] {
        let source = format!(
            "{PAIR}bad(element Type, left element, right element) Pair(element):
    return {initializer}
"
        );
        rejects(&source, "Type");
    }
}

#[test]
fn facts_from_a_checked_function_do_not_become_another_functions_assumptions() {
    let source = format!(
        "{WINDOW}
unconstrained(element Type, payload element, lower in 0 to 16, upper in 0 to 16) Window(element):
    return Window(payload: payload, lower: lower, upper: upper)
"
    );
    rejects(&source, "Unproved");
}

#[test]
fn repeated_generic_arguments_must_agree() {
    let source = format!(
        "{PAIR}bad(a Type, b Type, left a, right b) Pair(a):
    return Pair(left: left, right: right)
"
    );
    rejects(&source, "Type");
}

#[test]
fn all_three_child_permutations_preserve_ownership_and_duplicates_reject() {
    let names = ["a", "b", "c"];
    let mut accepted = 0;
    let mut rejected = 0;
    for first in 0..3 {
        for second in 0..3 {
            for third in 0..3 {
                let source = format!(
                    "Triple(element) has
    first element
    second element
    third element
permute(element Type, triple Triple(element)) Triple(element):
    Triple(a, b, c) := triple
    return Triple(first: {}, second: {}, third: {})
",
                    names[first], names[second], names[third]
                );
                if first != second && first != third && second != third {
                    accepts(&source);
                    accepted += 1;
                } else {
                    rejects(&source, "UseAfterMove");
                    rejected += 1;
                }
            }
        }
    }
    assert_eq!(accepted, 6);
    assert_eq!(rejected, 21);
}

#[test]
fn contradictory_entry_relations_cannot_make_a_body_valid() {
    rejects(
        "bad(lower in 0 to 16, upper in 0 to 16 and >= lower and < lower) in 0 to 16:
    return upper
",
        "Type",
    );
    let consistent = WINDOW.replacen(
        "upper in 0 to 16 and >= lower) Window(element):",
        "upper in 0 to 16 and >= lower and <= lower) Window(element):",
        1,
    );
    accepts(&consistent);
}

#[test]
fn recursive_layouts_and_wrong_type_argument_counts_reject() {
    for source in [
        "Loop(element) has next Loop(element)\n",
        "First(element) has next Second(element)\nSecond(element) has next First(element)\n",
        "Pair(element) has item element\nwrong(value Pair) Pair: return value\n",
    ] {
        rejects(source, "Type");
    }
}

#[test]
fn an_owned_generic_payload_is_not_a_numeric_witness() {
    rejects(
        "Bad(element) has
    payload element
    upper in 0 to 16 and >= payload
",
        "Type",
    );
    rejects("forge(element Type) element: return 0\n", "Type");
}

#[test]
fn scalar_payloads_are_reusable_but_their_nominal_container_is_linear() {
    accepts(
        "Parcel(element) has payload element
reuse() in 0 to 0:
    parcel: Parcel(payload: 0)
    Parcel(value) := parcel
    alias: value
    return value
",
    );
    rejects(
        "Parcel(element) has payload element
forget():
    parcel: Parcel(payload: 0)
    return
",
        "Unconsumed",
    );
}

#[test]
fn ownership_work_has_a_fail_closed_boundary() {
    let text = "Marker has\nforward(marker Marker) Marker: return marker\n";
    let source = parser::parse_strict(text).unwrap();
    let mut first_success = None;
    for flow_work_max in 0..=128 {
        let result = check_file(
            &source,
            Limits {
                flow_work_max,
                ..Limits::default()
            },
        );
        match result {
            Ok(_) => {
                first_success.get_or_insert(flow_work_max);
            }
            Err(error) => assert_eq!(error, SourceCheckError::Budget),
        }
    }
    let boundary = first_success.expect("small aggregate must fit the measured bound");
    assert!(boundary > 0);
    assert_eq!(
        check_file(
            &source,
            Limits {
                flow_work_max: boundary - 1,
                ..Limits::default()
            },
        ),
        Err(SourceCheckError::Budget)
    );
}
