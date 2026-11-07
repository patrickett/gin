//! Parenthesized sequence syntax and both positional projections share owned places.
use ast::{Expr, Expression, Item, SourceFile, Statement};
use typecheck::{
    CheckedSourceInstanceHead, CheckedSourceModule, Limits, SourceCheckError, SourceType,
    check_file, check_module,
};

const LIBRARY: &str = include_str!("../examples/sequences.gin");
const CLIENT: &str = include_str!("../examples/sequences-client.gin");

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid sequence source");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn library() -> CheckedSourceModule {
    check("library.sequences", LIBRARY, &[]).expect("body-checked sequence library")
}

#[test]
fn parenthesized_values_types_and_both_projections_preserve_payload_identity() {
    let library = library();
    check("app.client", CLIENT, &[&library]).unwrap();
    let client = CLIENT
        .replace("and = second", "and = first")
        .replace("return left_number", "return right_number");
    check("app.other", &client, &[&library]).unwrap();
    let bare = CLIENT
        .replace("from_values, swap,", "from_values, swap_bare,")
        .replace("swap(Scalar,", "swap_bare(Scalar,");
    check("app.bare", &bare, &[&library]).unwrap();
}

#[test]
fn heterogeneous_sequences_keep_each_position_type_and_origin() {
    let library = library();
    for (returned, expected) in [("left_number", "second"), ("right_number", "first")] {
        let client = format!(
            "use library.sequences swap_mixed
Left has number in 0 to 16
Right has number in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = {expected}:
    pair: swap_mixed(Left, Right, (Left(number: first), Right(number: second)))
    left: pair.0
    right: pair.[1]
    Right(left_number) := left
    Left(right_number) := right
    return {returned}
"
        );
        check("app.mixed", &client, &[&library]).unwrap();
    }
}

#[test]
fn wrong_order_is_not_hidden_by_conserving_both_owners() {
    let target = "    return (second, first)\n\nswap_bare";
    assert_eq!(LIBRARY.matches(target).count(), 1);
    // Restrict to the homogeneous swap; reversing the heterogeneous result has
    // an immediate type error rather than an origin-only mismatch.
    let wrong = LIBRARY.replace(target, "    return (first, second)\n\nswap_bare");
    assert_ne!(wrong, LIBRARY);
    let wrong = check("library.sequences", &wrong, &[]).unwrap();
    assert_eq!(
        check("app.client", CLIENT, &[&wrong]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn sequence_metadata_is_structural_not_an_array_or_nominal_schema() {
    let library = library();
    let checked = library.checked();
    let mixed = checked
        .functions
        .iter()
        .find(|function| function.name == "swap_mixed")
        .unwrap();
    let SourceType::Sequence { id } = mixed.result else {
        panic!("sequence result");
    };
    let instance = &checked.instances[id as usize];
    assert_eq!(instance.head, CheckedSourceInstanceHead::Sequence);
    assert_eq!(
        instance.arguments,
        [
            SourceType::Generic { index: 1 },
            SourceType::Generic { index: 0 }
        ]
    );
    let unit = checked
        .functions
        .iter()
        .find(|function| function.name == "unit")
        .unwrap();
    assert_eq!(unit.result, SourceType::Unit);
    let singleton = checked
        .functions
        .iter()
        .find(|function| function.name == "singleton")
        .unwrap();
    let SourceType::Sequence { id } = singleton.result else {
        panic!("singleton sequence result");
    };
    assert_eq!(
        checked.instances[id as usize].arguments,
        [SourceType::Generic { index: 0 }]
    );
}

#[test]
fn singleton_sequence_is_not_ordinary_parenthesis_grouping() {
    let library = library();
    let client = "
use library.sequences singleton, take_one
Marker has
roundtrip(value Marker) Marker:
    one: singleton(Marker, value)
    return take_one(Marker, one)
";
    check("app.singleton", client, &[&library]).unwrap();
    for text in [
        "bad(element Type, value element) (element,): return (value)\n",
        "bad(element Type, value element) element: return (value,)\n",
    ] {
        assert_eq!(
            check("bad.group", text, &[]).unwrap_err(),
            SourceCheckError::Type
        );
    }
}

#[test]
fn dot_and_bracket_access_cannot_consume_the_same_position_twice() {
    for (first, second) in [(".0", ".[0]"), (".[0]", ".0"), (".1", ".[1]")] {
        let text = format!(
            "bad(element Type, values (element, element)) (element, element):
    first: values{first}
    second: values{second}
    return (first, second)
"
        );
        assert_eq!(
            check("bad.repeat", &text, &[]).unwrap_err(),
            SourceCheckError::UseAfterMove
        );
    }
}

#[test]
fn a_partial_sequence_cannot_be_used_whole_and_unmoved_children_cannot_disappear() {
    let partial = "
bad(element Type, values (element, element)) (element, element):
    first: values.0
    return values
";
    assert_eq!(
        check("bad.partial", partial, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
    let lost = "bad(element Type, values (element, element)) element: return values.[0]\n";
    assert_eq!(
        check("bad.lost", lost, &[]).unwrap_err(),
        SourceCheckError::Unconsumed
    );
}

#[test]
fn positional_bounds_and_call_arity_are_not_inferred_by_flattening() {
    for text in [
        "bad(element Type, values (element, element)) element: return values.2\n",
        "bad(element Type, values (element, element)) element: return values.[2]\n",
    ] {
        assert_eq!(
            check("bad.bounds", text, &[]).unwrap_err(),
            SourceCheckError::Type
        );
    }
    let library = library();
    let wrong_call = "
use library.sequences swap
Marker has
bad(first Marker, second Marker) (Marker, Marker):
    return swap(Marker, first, second)
";
    assert_eq!(
        check("bad.call", wrong_call, &[&library]).unwrap_err(),
        SourceCheckError::Type
    );
}

#[test]
fn empty_sequence_is_unit_but_a_singleton_unit_sequence_is_not() {
    let text = "
accept(value ()) (): return value
run: return accept(())
";
    check("app.unit", text, &[]).unwrap();
    for text in [
        "accept(value ()) (): return value\nbad: return accept()\n",
        "bad (): return ((),)\n",
    ] {
        assert_eq!(
            check("bad.unit", text, &[]).unwrap_err(),
            SourceCheckError::Type
        );
    }
}

#[test]
fn the_two_projection_forms_also_agree_on_existing_fixed_arrays() {
    let text = "
swap(element Type, values [element; 2]) [element; 2]:
    first: values.0
    second: values.[1]
    return [second, first]
";
    check("app.arrays", text, &[]).unwrap();
    let repeat = text.replace("values.[1]", "values.[0]");
    assert_eq!(
        check("bad.arrays", &repeat, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn nested_owned_sequences_use_explicit_local_intermediates() {
    let text = "
rebuild(element Type, values ((element, element), element)) ((element, element), element):
    pair: values.0
    other: values.[1]
    first: pair.0
    second: pair.[1]
    return ((first, second), other)
";
    check("app.nested", text, &[]).unwrap();
    let chain = "
bad(element Type, values ((element,),)) element:
    return values.0.0
";
    assert_eq!(
        check("bad.nested_base", chain, &[]).unwrap_err(),
        SourceCheckError::Unsupported
    );
}

#[test]
fn an_unknown_owned_position_is_not_guessed_from_a_bounded_interval() {
    let text = "
bad(element Type, values (element, element), index in 0 to 1) element:
    return values.[index]
";
    assert_eq!(
        check("bad.unknown", text, &[]).unwrap_err(),
        SourceCheckError::Unsupported
    );
}

#[test]
fn bracket_arrays_and_parenthesized_sequences_are_distinct_types() {
    for text in [
        concat!(
            "bad(element Type, first element, second element) (element, element):\n",
            "    return [first, second]\n",
        ),
        concat!(
            "bad(element Type, first element, second element) [element; 2]:\n",
            "    return (first, second)\n",
        ),
    ] {
        assert_eq!(
            check("bad.distinct", text, &[]).unwrap_err(),
            SourceCheckError::Type
        );
    }
}

#[test]
fn unit_does_not_dispose_of_owned_or_scalar_containers() {
    for text in [
        "Marker has\nbad:\n    value: (Marker,)\n    return ()\n",
        "bad:\n    pair: (1, 2)\n    first: pair.0\n    again: pair.[0]\n    return ()\n",
    ] {
        assert_eq!(
            check("bad.ending", text, &[]).unwrap_err(),
            SourceCheckError::Unconsumed
        );
    }
}

#[test]
fn generic_unpacking_explicitly_consumes_the_whole_container() {
    let library = library();
    let text = "
use library.sequences take_one
run: return take_one((), ((),))
";
    check("app.explicit_unpack", text, &[&library]).unwrap();
}

#[test]
fn private_grouped_sequence_fields_follow_the_same_checked_boundary() {
    let text = "
Secret(element) has
    #private
    value (element, element)
make(element Type, first element, second element) Secret(element):
    return Secret(value: (first, second))
take(element Type, secret Secret(element)) (element, element):
    Secret(value) := secret
    return value
";
    let library = check("library.private_sequences", text, &[]).unwrap();
    let imports = "use library.private_sequences Secret, make, take\nMarker has\n";
    let allowed = format!(
        "{imports}
roundtrip(first Marker, second Marker) (Marker, Marker):
    secret: make(Marker, first, second)
    return take(Marker, secret)
"
    );
    check("app.allowed", &allowed, &[&library]).unwrap();
    for body in [
        concat!(
            "bad(first Marker, second Marker) Secret(Marker):\n",
            "    return Secret(value: (first, second))\n",
        ),
        concat!(
            "bad(first Marker, second Marker) (Marker, Marker):\n",
            "    secret: make(Marker, first, second)\n",
            "    Secret(value) := secret\n",
            "    return value\n",
        ),
    ] {
        assert_eq!(
            check("app.private", &format!("{imports}{body}"), &[&library]).unwrap_err(),
            SourceCheckError::Privacy
        );
    }
}

fn returned_expression(source: &mut SourceFile) -> &mut Expression {
    let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
        panic!("function binding");
    };
    let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
        panic!("return block");
    };
    let Statement::Return(returned) = &mut block.statements[0].value else {
        panic!("return statement");
    };
    returned.value.as_mut().expect("return value")
}

#[test]
fn manual_asts_cannot_erase_or_reclassify_written_sequence_elements() {
    let mut erased = parser::parse_strict("f: return (1, 2)\n").unwrap();
    let Expr::Sequence(sequence) = &mut returned_expression(&mut erased).value else {
        panic!("sequence");
    };
    sequence.elements.clear();
    sequence.separators.clear();
    assert_eq!(
        check_file(&erased, Limits::default()),
        Err(SourceCheckError::Malformed)
    );

    let mut grouped =
        parser::parse_strict("Marker has\nf Marker: return (Marker,)\n").unwrap();
    let returned = returned_expression(&mut grouped);
    let Expr::Sequence(sequence) = &returned.value else {
        panic!("singleton sequence");
    };
    returned.value = Expr::Group(Box::new(sequence.elements[0].clone()));
    assert_eq!(
        check_file(&grouped, Limits::default()),
        Err(SourceCheckError::Malformed)
    );

    let mut reordered = parser::parse_strict("f() ((), ()): return ((), ())\n").unwrap();
    let Expr::Sequence(sequence) = &mut returned_expression(&mut reordered).value else {
        panic!("sequence");
    };
    sequence.elements.swap(0, 1);
    assert_eq!(
        check_file(&reordered, Limits::default()),
        Err(SourceCheckError::Malformed)
    );
}
