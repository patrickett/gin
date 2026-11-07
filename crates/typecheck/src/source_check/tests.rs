use super::*;

fn run(text: &str) -> Result<CheckedSourceFile> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, crate::Limits::default())
}

#[test]
fn layouts_are_acyclic_even_when_unused_or_generic() {
    for source in [
        "Loop has next Loop\n",
        "Left has next Right\nRight has next Left\n",
        "Grow(element) has next Grow(Grow(element))\n",
    ] {
        assert_eq!(run(source), Err(SourceCheckError::Type), "{source}");
    }
    assert!(run("Leaf(element) has item element\nTree(element) has leaf Leaf(element)\n").is_ok());
}

#[test]
fn lazy_layouts_still_validate_unused_immediate_scalar_contracts() {
    for text in [
        "Bad(element) has payload element, upper in 0 to 16 and >= payload\n",
        "Bad has lower in 8 to 10, upper in 0 to 3 and >= lower\n",
        concat!(
            "Bad(element) has payload element, upper in 0 to 16 and >= payload\n",
            "Outer(element) has nested Bad(element)\n",
        ),
    ] {
        assert_eq!(run(text), Err(SourceCheckError::Type));
    }
}

#[test]
fn type_application_arity_and_depth_are_bounded() {
    for ty in ["Box()", "Box(element, element)"] {
        let source =
            format!("Box(element) has item element\nf(element Type, x {ty}) {ty}: return x\n");
        assert_eq!(run(&source), Err(SourceCheckError::Type));
    }
    let mut ty = "element".to_owned();
    for _ in 0..66 {
        ty = format!("Box({ty})");
    }
    let source = format!("Box(element) has item element\nf(element Type, x {ty}) {ty}: return x\n");
    assert_eq!(run(&source), Err(SourceCheckError::Budget));
}

#[test]
fn metatype_and_constructor_names_obey_resolution() {
    assert!(run("Type has\nf(element Type, x element) element: return x\n").is_err());
    assert!(run("Type is 0 to 16\nf(element Type, x element) element: return x\n").is_err());
    // The parser distinguishes parameter capitalization; manual AST consumers do not.
    let mut source = parser::parse_strict(
        "Box(element) has item element\nf(box Type, element Type, x Box(element)): return\n",
    )
    .unwrap();
    source.text = source.text.replacen("box Type", "Box Type", 1).into();
    let Item::Binding(function) = &mut source.items[1].value else {
        unreachable!()
    };
    function.parameters.as_mut().unwrap().value[0].name.value = "Box".to_owned();
    assert_eq!(
        check_file(&source, crate::Limits::default()),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn moved_aliases_and_discarded_values_remain_accounted() {
    assert_eq!(
        run("f(element Type, x element):\n  x\n  return\n"),
        Err(SourceCheckError::Unconsumed)
    );
    assert_eq!(
        run("f(element Type, x element) element:\n  y: x\n  z: y\n  return x\n"),
        Err(SourceCheckError::UseAfterMove)
    );
}

#[test]
fn entry_relations_use_identity_transitivity_and_consistency() {
    assert!(
        run(concat!(
            "f(a in 0 to 16, b in 0 to 16 and >= a, c in 0 to 16 and >= b)",
            " in 0 to 16 and >= a:\n  return c\n"
        ))
        .is_ok()
    );
    assert_eq!(
        run("f(a in 0 to 16, b in 0 to 16 and >= a, c in 0 to 16 and >= b and < a): return\n"),
        Err(SourceCheckError::Type)
    );
    assert!(run("f(a in 0 to 16, b in 0 to 16 and >= a) in 0 to 16 and <= b: return a\n").is_ok());
}

#[test]
fn aggregate_budget_boundaries_and_flow_are_fail_closed() {
    let source = parser::parse_strict(
        "Box(element) has item element\nf(element Type, x Box(element)) Box(element): return x\n",
    )
    .unwrap();
    let mut lower = 0;
    let mut upper = crate::Limits::default().work_max;
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        let limits = crate::Limits {
            work_max: middle,
            ..crate::Limits::default()
        };
        match check_file(&source, limits) {
            Ok(_) => upper = middle,
            Err(SourceCheckError::Budget) => lower = middle + 1,
            other => panic!("{other:?}"),
        }
    }
    assert!(lower > 0);
    for (work, success) in [(lower - 1, false), (lower, true)] {
        let result = check_file(
            &source,
            crate::Limits {
                work_max: work,
                ..crate::Limits::default()
            },
        );
        assert_eq!(result.is_ok(), success);
        if !success {
            assert_eq!(result, Err(SourceCheckError::Budget));
        }
    }
    assert_eq!(
        check_file(
            &source,
            crate::Limits {
                flow_work_max: 0,
                ..crate::Limits::default()
            }
        ),
        Err(SourceCheckError::Budget)
    );
}

#[test]
fn new_span_occurrences_are_validated_without_panics() {
    let pristine = parser::parse_strict(concat!(
        "Box(element) has\n  #private\n  item element\n",
        "f(element Type, x Box(element)) element:\n  Box(item) := x\n  return item\n",
    ))
    .unwrap();
    for occurrence in 0..7 {
        let mut source = pristine.clone();
        let Item::Type(declaration) = &mut source.items[0].value else {
            unreachable!()
        };
        let TypeBody::Has(fields) = &mut declaration.body.value else {
            unreachable!()
        };
        let TypeMember::Binding(field) = &mut fields[0].value else {
            unreachable!()
        };
        match occurrence {
            0 => declaration.parameters.as_mut().unwrap().span_id = SpanId::INVALID,
            1 => field.annotation.as_mut().unwrap().span_id = SpanId::INVALID,
            2 => field.attributes[0].span_id = SpanId::INVALID,
            _ => {
                let Item::Binding(function) = &mut source.items[1].value else {
                    unreachable!()
                };
                let Expr::Block(block) = &mut function.value.as_mut().unwrap().expression.value
                else {
                    unreachable!()
                };
                let Statement::Destructure(pattern) = &mut block.statements[0].value else {
                    unreachable!()
                };
                let pattern = &mut pattern.pattern;
                let fields = pattern.fields.as_mut().unwrap();
                match occurrence {
                    3 => fields.binders.value[0].span_id = SpanId::INVALID,
                    4 => fields.open = SpanId::INVALID,
                    5 => fields.close = SpanId::INVALID,
                    _ => pattern.constructor.span_id = SpanId::INVALID,
                }
            }
        }
        assert_eq!(
            check_file(&source, crate::Limits::default()),
            Err(SourceCheckError::Malformed),
            "{occurrence}"
        );
    }
}

#[test]
fn ordinary_calls_schedule_forward_edges_and_reject_cycles() {
    assert!(
        run(concat!(
            "forward(x in 0 to 16) in 0 to 16 and = x: return identity(identity(x))\n",
            "identity(x in 0 to 16) in 0 to 16: return x\n",
        ))
        .is_ok()
    );
    for source in [
        "f(x in 0 to 16) in 0 to 16: return f(x)\n",
        concat!(
            "f(x in 0 to 16) in 0 to 16: return g(x)\n",
            "g(x in 0 to 16) in 0 to 16: return f(x)\n",
        ),
    ] {
        assert_eq!(run(source), Err(SourceCheckError::Unsupported));
    }
    assert_eq!(
        run(concat!(
            "identity(x in 0 to 16) in 0 to 16: return x\n",
            "f(x in 0 to 16) in 0 to 16 and >= identity(x): return x\n",
        )),
        Err(SourceCheckError::Unsupported),
    );
}

fn module(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    let source = parser::parse_strict(text).unwrap();
    check_module(name, &source, dependencies, crate::Limits::default()).unwrap()
}

#[test]
fn diamond_imports_remap_nested_types_without_losing_origin() {
    let base = module(
        "base",
        concat!(
            "Box(element) has\n  #private\n  item element\n",
            "make(element Type, x element) Box(element): return Box(item: x)\n",
            "take(element Type, x Box(element)) element:\n",
            "  Box(item) := x\n  return item\n",
        ),
        &[],
    );
    let middle = module(
        "middle",
        concat!(
            "use base Box, make\n",
            "Wrap(element) has item element\n",
            "build(element Type, x element) Box(Wrap(element)):\n",
            "  return make(Wrap(element), Wrap(item: x))\n",
        ),
        &[&base],
    );
    let client = module(
        "client",
        concat!(
            "use middle Wrap, build\n",
            "use base Box, take\n",
            "Other has\n",
            "roundtrip(element Type, x element) element:\n",
            "  wrapped: take(Wrap(element), build(element, x))\n",
            "  Wrap(item) := wrapped\n",
            "  return item\n",
        ),
        &[&base, &middle],
    );
    assert_eq!(
        client.checked().functions[0].result,
        SourceType::Generic { index: 0 }
    );
}

#[test]
fn generic_calls_cannot_end_unreturned_opaque_children() {
    assert_eq!(
        run(concat!(
            "Pair(element) has first element, second element\n",
            "bad(element Type, pair Pair(element)) element:\n",
            "  Pair(first, second) := pair\n",
            "  return identity(element, first)\n",
            "identity(element Type, x element) element: return x\n",
        )),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn discarded_body_only_type_instances_do_not_enter_dependency_work() {
    let prefix = "Box(element) has item element\n";
    let signature = "identity(element Type, x element) element:\n";
    let short = module("library", &format!("{prefix}{signature}  return x\n"), &[]);
    let long = module(
        "library",
        &format!(
            concat!(
                "{}{}",
                "  outer: Box(item: Box(item: x))\n",
                "  Box(inner) := outer\n",
                "  Box(value) := inner\n",
                "  return value\n",
            ),
            prefix, signature
        ),
        &[],
    );
    // `identity` is generic, so its recorded body is a template that is instantiated from the
    // types of its nodes: unlike a body of a function that is not generic, it keeps the instances
    // only it names, and an importer carries them.
    assert!(short.checked().instances.is_empty());
    assert_eq!(long.checked().instances.len(), 2);
    let source = parser::parse_strict(concat!(
        "use library identity\n",
        "forward(element Type, x element) element: return identity(element, x)\n",
    ))
    .unwrap();
    let short_work = module_work(&source, &short);
    let long_work = module_work(&source, &long);
    assert!(short_work > 0);
    assert!(long_work >= short_work);
}

fn module_work(source: &SourceFile, dependency: &CheckedSourceModule) -> u32 {
    let mut lower = 0;
    let mut upper = crate::Limits::default().work_max;
    while lower < upper {
        let work_max = lower + (upper - lower) / 2;
        let limits = crate::Limits {
            work_max,
            ..crate::Limits::default()
        };
        match check_module("client", source, &[dependency], limits) {
            Ok(_) => upper = work_max,
            Err(SourceCheckError::Budget) => lower = work_max + 1,
            other => panic!("{other:?}"),
        }
    }
    lower
}

#[test]
fn imported_type_named_type_shadows_the_builtin_metakind() {
    let library = module("types", "Token has\nType has payload Token\n", &[]);
    let source =
        parser::parse_strict("use types Type\nforget(value Type): return\n").unwrap();
    assert!(matches!(
        check_module("client", &source, &[&library], crate::Limits::default()),
        Err(SourceCheckError::Unconsumed),
    ));
    assert_eq!(
        run("Token has\nType has payload Token\nforget(value Type): return\n"),
        Err(SourceCheckError::Unconsumed),
    );
    let identity = module(
        "client",
        "use types Type\nidentity(value Type) Type: return value\n",
        &[&library],
    );
    assert!(identity.checked().functions[0].type_parameters.is_empty());
    assert_eq!(identity.checked().functions[0].parameters.len(), 1);
}

#[test]
fn publication_uses_parameter_roles_resolved_before_later_shadowing() {
    let mut source =
        parser::parse_strict("f(item Type, value item) item: return value\n").unwrap();
    source.text = source.text.replace("item", "Type").into();
    let Item::Binding(binding) = &mut source.items[0].value else {
        unreachable!()
    };
    let parameters = binding.parameters.as_mut().unwrap();
    parameters.value[0].name.value = "Type".to_owned();
    for annotation in [&mut parameters.value[1].annotation, &mut binding.annotation] {
        let Annotation::Expression(expression) = &mut annotation.as_mut().unwrap().value else {
            unreachable!()
        };
        let Expr::Name(name) = &mut expression.value else {
            unreachable!()
        };
        *name = "Type".to_owned();
    }
    let checked = check_file(&source, crate::Limits::default()).unwrap();
    assert_eq!(checked.functions[0].type_parameters, ["Type"]);
    assert_eq!(
        checked.functions[0].parameters,
        [SourceType::Generic { index: 0 }]
    );
}

const SUM: &str = concat!(
    "Slot(element) is Empty or Full(element)\n",
);

#[test]
fn untouched_sum_children_publish_original_origins_without_variant_products() {
    use std::fmt::Write;

    for untouched in 0..=6 {
        for call in [false, true] {
            let mut text = format!("{SUM}Buffer(element) has target Slot(element)");
            for index in 0..untouched {
                write!(text, ", rest{index} Slot(element)").unwrap();
            }
            text.push_str(concat!(
                "\nrebuild(element Type, slot Slot(element)) Slot(element):\n",
                "  return when slot is\n",
                "    Empty then Empty\n",
                "    Full(value) then Full(value)\n",
                "forward(element Type, buffer Buffer(element)) Buffer(element):\n",
                "  Buffer(target",
            ));
            for index in 0..untouched {
                write!(text, ", rest{index}").unwrap();
            }
            text.push_str(") := buffer\n");
            if call {
                text.push_str("  selected: rebuild(element, target)\n");
            } else {
                text.push_str(concat!(
                    "  selected Slot(element): when target is\n",
                    "    Empty then Empty\n",
                    "    Full(value) then Full(value)\n",
                ));
            }
            text.push_str("  return Buffer(target: selected");
            for index in 0..untouched {
                write!(text, ", rest{index}").unwrap();
            }
            text.push_str(")\n");
            let source = parser::parse_strict(&text).unwrap();
            let module = check_module("lazy", &source, &[], crate::Limits::default()).unwrap();
            let summaries = module.summaries("forward");
            assert_eq!(summaries.len(), 2, "untouched={untouched}, call={call}");
            for summary in summaries {
                let root = summary.roots[0];
                assert_eq!(summary.inputs[root].children.len(), untouched + 1);
                assert!(summary.inputs.len() <= untouched + 3);
                for child in summary.inputs[root].children.iter().skip(1) {
                    assert!(matches!(summary.inputs[*child].ty, SourceType::Sum { .. }));
                    assert!(summary.inputs[*child].children.is_empty());
                    assert_eq!(
                        summary.dispositions[*child],
                        summaries::Disposition::Returned
                    );
                    assert_eq!(
                        summary
                            .outputs
                            .iter()
                            .filter(|o| o.input == Some(*child))
                            .count(),
                        1
                    );
                }
            }
        }
    }
}

#[test]
fn impossible_nested_guards_do_not_publish_speculative_sum_inspections() {
    use std::fmt::Write;

    let mut text = format!(
        "{SUM}{}",
        concat!(
            "Index has value in 0 to 1\n",
            "rebuild(element Type, slot Slot(element)) Slot(element):\n",
            "  return when slot is\n",
            "    Empty then Empty\n",
            "    Full(value) then Full(value)\n",
            "conditional(element Type, slot Slot(element), index Index) Slot(element):\n",
            "  Index(value) := index\n",
            "  return when value is\n",
            "    0 then slot\n",
            "    1 then rebuild(element, slot)\n",
            "Buffer(element) has target Slot(element)",
        )
    );
    for index in 0..6 {
        write!(text, ", rest{index} Slot(element)").unwrap();
    }
    text.push_str(
        "\nforward(element Type, buffer Buffer(element)) Buffer(element):\n  Buffer(target",
    );
    for index in 0..6 {
        write!(text, ", rest{index}").unwrap();
    }
    text.push_str(
        ") := buffer\n  return Buffer(target: conditional(element, target, Index(value: 0))",
    );
    for index in 0..6 {
        write!(
            text,
            ", rest{index}: conditional(element, rest{index}, Index(value: 0))"
        )
        .unwrap();
    }
    text.push_str(")\n");
    let source = parser::parse_strict(&text).unwrap();
    let module = check_module("lazy", &source, &[], crate::Limits::default()).unwrap();
    let summaries = module.summaries("forward");
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.inputs.len(), 8);
    for input in summary.inputs.iter().skip(1) {
        assert!(matches!(input.ty, SourceType::Sum { .. }));
        assert!(input.children.is_empty());
    }
}

#[test]
fn opaque_identity_calls_preserve_the_declared_sum_view() {
    let identity = "identity(element Type, value element) element: return value\n";
    let body = concat!(
        "f(element Type, slot Slot(element)) Slot(element):\n",
        "  return when identity(Slot(element), slot) is\n",
        "    Empty then Empty\n",
        "    Full(value) then Full(value)\n",
    );
    assert!(run(&format!("{SUM}{identity}{body}")).is_ok());
    // The known active Empty does not erase the declared Slot result's coverage.
    let missing = concat!(
        "f(element Type) Slot(element):\n",
        "  return when identity(Slot(element), Slot.Empty) is\n",
        "    Empty then Empty\n",
    );
    assert_eq!(
        run(&format!("{SUM}{identity}{missing}")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn nested_sum_worlds_account_for_each_outer_and_active_payload() {
    let declarations = concat!(
        "Pair(element) has left Slot(element), right Slot(element)\n",
        "rebuild(element Type, slot Slot(element)) Slot(element):\n",
        "  return when slot is\n",
        "    Empty then Empty\n",
        "    Full(value) then Full(value)\n",
    );
    let body = concat!(
        "f(element Type, pair Pair(element)) Pair(element):\n",
        "  Pair(left, right) := pair\n",
        "  return Pair(left: rebuild(element, left), right: rebuild(element, right))\n",
    );
    let result = run(&format!("{SUM}{declarations}{body}"));
    assert!(result.is_ok(), "{result:?}");
    let dropped = concat!(
        "f(element Type, left Slot(element), right Slot(element)) Slot(element):\n",
        "  return when left is\n",
        "    Empty then right\n",
        "    Full(value) then Full(value)\n",
    );
    assert_eq!(
        run(&format!("{SUM}{dropped}")),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn sum_payloads_do_not_fabricate_scalar_proofs() {
    let source = concat!(
        "Low has value in 0 to 3\n",
        "High has value in 8 to 10\n",
        "Choice is Low(Low) or High(High)\n",
        "f(choice Choice) out in 0 to 10 where out >= 8:\n",
        "  return when choice is\n",
        "    Low(low) then low.value\n",
        "    High(high) then high.value\n",
    );
    assert_eq!(run(source), Err(SourceCheckError::Unproved));
}

// `inferred_sum_fields_accept_a_distinct_subset_alias` was removed with the type-member
// unions: `Big is Small or C` made `Small` a member of `Big`, so a `Small` was a `Big` without
// a constructor. A variant is the union's own now (`Big is Narrow(Small) or C`), a `Small` is
// never a `Big`, and there is no subset coercion left to test.

#[test]
fn impossible_arm_witnesses_are_checked_but_do_not_publish_origins() {
    let factory = "empty(element Type) Slot(element): return Empty\n";
    for (body, error) in [
        ("missing", SourceCheckError::UnknownName),
        ("0", SourceCheckError::Type),
        ("Empty", SourceCheckError::Unconsumed),
    ] {
        let function = format!(
            "f(element Type) Slot(element):\n  return when empty(element) is\n    Empty then Empty\n    Full(value) then {body}\n"
        );
        assert_eq!(run(&format!("{SUM}{factory}{function}")), Err(error));
    }
    let source = parser::parse_strict(&format!(
        "{SUM}{factory}f(element Type) Slot(element):\n  return when empty(element) is\n    Empty then Empty\n    Full(value) then Full(value)\n"
    )).unwrap();
    let module = check_module("example", &source, &[], crate::Limits::default()).unwrap();
    let function = module
        .checked()
        .functions
        .iter()
        .find(|f| f.name == "f")
        .unwrap();
    assert!(matches!(function.result, SourceType::Sum { .. }));
    // A caller may consume the real Empty outcome. The hypothetical Full payload
    // is absent from the published summary and cannot acquire a fresh generic origin.
    let client = parser::parse_strict(concat!(
        "use example Slot, f\n",
        "g(element Type) Slot(element):\n",
        "  return when f(element) is\n",
        "    Empty then Empty\n",
        "    Full(value) then\n",
        "      return value\n",
    ))
    .unwrap();
    assert_eq!(
        check_module("client", &client, &[&module], crate::Limits::default()).unwrap_err(),
        SourceCheckError::Type,
    );
}

const INTEGER_CHOICE: &str = concat!(
    "choose(index in 0 to 1) in 0 to 1:\n",
    "  return when index is\n",
    "    0 then 0\n",
    "    1 then 1\n",
);

#[test]
fn finite_integer_cases_check_declared_coverage_without_interval_enumeration() {
    assert!(
        run(concat!(
            "f(index in -2 to 2) in -2 to 2:\n",
            "  return when index is\n",
            "    2 then index\n",
            "    -1 then index\n",
            "    0 then index\n",
            "    -2 then index\n",
            "    1 then index\n",
        ))
        .is_ok()
    );
    for (arms, error) in [
        ("0 then 0", SourceCheckError::Type),
        ("0 then 0\n    2 then 1", SourceCheckError::Type),
        (
            "0 then 0\n    1 then 1\n    2 then 1",
            SourceCheckError::Type,
        ),
        (
            "0 then 0\n    0x0 then 0\n    1 then 1",
            SourceCheckError::Duplicate,
        ),
        ("0 then 0\n    Empty then 1", SourceCheckError::Type),
    ] {
        let text = format!(
            "Empty has\nf(index in 0 to 1) in 0 to 1:\n  return when index is\n    {arms}\n"
        );
        assert_eq!(run(&text), Err(error), "{text}");
    }
    for number in [I256::MIN, I256::MAX] {
        let text = format!(
            "f(index in {number} to {number}) in {number} to {number}:\n  return when index is\n    {number} then index\n"
        );
        assert!(run(&text).is_ok(), "{text}");
    }
    let text = format!(
        "f(index in {} to {}) in 0 to 1:\n  return when index is\n    {} then 0\n    {} then 1\n",
        I256::MIN,
        I256::MAX,
        I256::MIN,
        I256::MAX
    );
    assert_eq!(run(&text), Err(SourceCheckError::Type));
}

#[test]
fn integer_case_guards_narrow_inputs_and_survive_aliases_and_repeated_calls() {
    let text = format!(
        "{INTEGER_CHOICE}f(index in 0 to 1) out in 0 to 1 and = index where out = index:\n  alias: index\n  a: choose(alias)\n  b: choose(index)\n  return a\n"
    );
    assert!(run(&text).is_ok());
    let source = parser::parse_strict(&text).unwrap();
    let module = check_module("numbers", &source, &[], crate::Limits::default()).unwrap();
    let summaries = module.summaries("f");
    assert_eq!(summaries.len(), 2);
    for summary in summaries {
        assert_eq!(summary.integer_guards.len(), 1);
        let (input, number) = summary.integer_guards[0];
        assert_eq!(input, summary.roots[0]);
        assert_eq!(summary.outputs[0].lower, number);
        assert_eq!(summary.outputs[0].upper, number);
    }
    let unrelated = format!(
        "{INTEGER_CHOICE}f(first in 0 to 1, second in 0 to 1) out in 0 to 1 where out = second:\n  a: choose(first)\n  b: choose(second)\n  return a\n"
    );
    assert_eq!(run(&unrelated), Err(SourceCheckError::Unproved));
}

#[test]
fn numeric_inactive_arms_are_isolated_but_still_statically_checked() {
    let zero = "zero() in 0 to 1: return 0\n";
    for (body, result) in [
        ("1", Ok(())),
        ("missing", Err(SourceCheckError::UnknownName)),
        ("2", Err(SourceCheckError::Type)),
    ] {
        let text = format!(
            "{zero}f() in 0 to 1:\n  return when zero() is\n    0 then 0\n    1 then {body}\n"
        );
        assert_eq!(run(&text).map(|_| ()), result, "{text}");
    }
    let text = format!("{zero}f() in 0 to 1:\n  return when zero() is\n    0 then 0\n");
    assert_eq!(run(&text), Err(SourceCheckError::Type));
    let source = parser::parse_strict(&format!(
        "{zero}f() in 0 to 1:\n  return when zero() is\n    0 then 0\n    1 then 1\n"
    ))
    .unwrap();
    let module = check_module("numbers", &source, &[], crate::Limits::default()).unwrap();
    let summaries = module.summaries("f");
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].integer_guards.is_empty());
    assert_eq!(summaries[0].outputs[0].lower, I256::ZERO);
}

#[test]
fn imported_integer_guards_retain_actual_parameter_identity() {
    let source = parser::parse_strict(INTEGER_CHOICE).unwrap();
    let library = check_module("numbers", &source, &[], crate::Limits::default()).unwrap();
    let wrapper_source = parser::parse_strict(concat!(
        "use numbers choose\n",
        "forward(index in 0 to 1) in 0 to 1: return choose(index)\n",
    ))
    .unwrap();
    let wrapper = check_module(
        "wrapper",
        &wrapper_source,
        &[&library],
        crate::Limits::default(),
    )
    .unwrap();
    let client = parser::parse_strict(concat!(
        "use wrapper forward\n",
        "f(index in 0 to 1) out in 0 to 1 where out = index:\n",
        "  alias: index\n",
        "  a: forward(alias)\n",
        "  b: forward(index)\n",
        "  return a\n",
    ))
    .unwrap();
    assert!(check_module("client", &client, &[&wrapper], crate::Limits::default()).is_ok());
    for argument in ["2", "index"] {
        let client = parser::parse_strict(&format!(
            "use wrapper forward\nf(index in 0 to 2) in 0 to 1: return forward({argument})\n"
        ))
        .unwrap();
        assert_eq!(
            check_module("client", &client, &[&wrapper], crate::Limits::default()).unwrap_err(),
            SourceCheckError::Type
        );
    }
}

#[test]
fn indexed_buffer_default_budget() {
    let limits = crate::Limits::default();
    let slots =
        parser::parse_strict(include_str!("../../examples/runtime-slots.gin")).unwrap();
    let slots = check_module("example.slots", &slots, &[], limits).unwrap();
    let buffer =
        parser::parse_strict(include_str!("../../examples/indexed-buffer.gin")).unwrap();
    let buffer = check_module("example.buffers", &buffer, &[&slots], limits).unwrap();
    assert_eq!(buffer.checked().functions.len(), 9);
}

#[test]
fn world_dependent_domain_annotations_are_not_cached_by_source_location() {
    assert_eq!(
        run(concat!(
            "f(index in 0 to 1) in 0 to 1:\n",
            "  bound: when index is\n",
            "    0 then 1\n",
            "    1 then 0\n",
            "  invalid in 0 to bound: 1\n",
            "  return invalid\n",
        )),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn integer_world_guard_and_outcome_budgets_have_exact_fail_closed_boundaries() {
    let text = format!(
        "{INTEGER_CHOICE}f(index in 0 to 1) out in 0 to 1 where out = index:\n  a: choose(index)\n  b: choose(index)\n  return a\n"
    );
    let source = parser::parse_strict(&text).unwrap();
    for flow in [false, true] {
        let mut low = 0;
        let mut high = if flow {
            crate::Limits::default().flow_work_max
        } else {
            crate::Limits::default().work_max
        };
        while low < high {
            let middle = low + (high - low) / 2;
            let mut limits = crate::Limits::default();
            if flow {
                limits.flow_work_max = middle;
            } else {
                limits.work_max = middle;
            }
            match check_file(&source, limits) {
                Ok(_) => high = middle,
                Err(SourceCheckError::Budget) => low = middle + 1,
                other => panic!("{other:?}"),
            }
        }
        assert!(low > 0);
        let mut limits = crate::Limits::default();
        if flow {
            limits.flow_work_max = low;
        } else {
            limits.work_max = low;
        }
        assert!(check_file(&source, limits).is_ok());
        if flow {
            limits.flow_work_max -= 1;
        } else {
            limits.work_max -= 1;
        }
        assert_eq!(check_file(&source, limits), Err(SourceCheckError::Budget));
    }
}

#[test]
fn computed_scalar_aliases_have_one_identity_without_inverting_their_expression() {
    assert!(
        run(concat!(
            "f(index in 0 to 1) out in -1 to 0 where out >= -1, out <= 0:\n",
            "  computed: -index\n",
            "  alias: computed\n",
            "  return when alias is\n",
            "    -1 then computed\n",
            "    0 then computed\n",
        ))
        .is_ok()
    );
    assert_eq!(
        run(concat!(
            "f(index in 0 to 1) out in 0 to 1 where out = 1:\n",
            "  computed: -index\n",
            "  return when computed is\n",
            "    -1 then index\n",
            "    0 then 1\n",
        )),
        Err(SourceCheckError::Unproved)
    );
}
