use ast::{Argument, Expr, Item};
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file, check_module};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid constructor syntax");
    let before = source.clone();
    let result = check_file(&source, Limits::default());
    assert_eq!(source, before);
    result
}

#[test]
fn the_parser_preserves_a_bare_argument_instead_of_inventing_a_named_one() {
    let source = parser::parse_strict("result: Taken(value, slot: Empty)\n").unwrap();
    let Item::Binding(binding) = &source.items[0].value else {
        panic!("result binding");
    };
    let Expr::Call { arguments, .. } = &binding.value.as_ref().unwrap().expression.value else {
        panic!("constructor expression");
    };
    let Argument::Positional(value) = &arguments.value[0].value else {
        panic!("bare written argument");
    };
    assert_eq!(value.value, Expr::Name("value".into()));
    assert_eq!(
        source.span_table.get(value.span_id).extract(&source.text),
        "value"
    );
    assert!(
        matches!(&arguments.value[1].value, Argument::Named { name, .. }
        if name.value == "slot")
    );
}

#[test]
fn shorthand_matches_field_names_not_argument_positions() {
    for arguments in ["right, left", "right: right, left", "right, left: left"] {
        let source = format!(
            "Pair has left in 0 to 16, right in 0 to 16
read(left in 0 to 16, right in 0 to 16) in 0 to 16 and = left:
    pair: Pair({arguments})
    Pair(first, second) := pair
    return first
"
        );
        check(&source).unwrap();
    }
}

#[test]
fn ordinary_function_arguments_remain_positional() {
    let source = "first(left in 0 to 16, right in 0 to 16) in 0 to 16: return left
observe(left in 0 to 16, right in 0 to 16) in 0 to 16 and = right:
    return first(right, left)
";
    check(source).unwrap();
}

#[test]
fn generic_inference_and_contextual_construction_use_the_same_shorthand() {
    let source = "Pair(element) has left element, right element
make(element Type, left element, right element) Pair(element):
    pair: Pair(right, left)
    return pair
";
    check(source).unwrap();
    check(&source.replace("pair:", "pair Pair(element):")).unwrap();
}

#[test]
fn reordered_shorthand_receives_the_expected_type_of_its_named_field() {
    let source = "Small has
Large has
Pair has left Small, right Large
make(left Small, right Large) Pair: return Pair(right, left)
";
    check(source).unwrap();
    let inferred = source.replace(
        "make(left Small, right Large) Pair: return Pair(right, left)",
        "make(left Small, right Large) Pair:\n    pair: Pair(right, left)\n    return pair",
    );
    assert_ne!(inferred, source);
    check(&inferred).unwrap();
}

#[test]
fn mixed_reordered_fields_propagate_context_to_a_nested_constructor() {
    let source = "Small has
Phantom(element) has
Pair(element) has left element, right Phantom(element)
make(left Small) Pair(Small): return Pair(right: Phantom, left)
";
    check(source).unwrap();
    let annotated = source.replace(
        "make(left Small) Pair(Small): return Pair(right: Phantom, left)",
        "make(left Small) Pair(Small):
    pair Pair(Small): Pair(right: Phantom, left)
    return pair",
    );
    assert_ne!(annotated, source);
    check(&annotated).unwrap();
}

#[test]
fn mixed_shorthand_and_named_arguments_preserve_variant_payloads() {
    let source = "Spent has
Taken(element) has value element, slot Spent
Result(element) is Gone or Taken(Taken(element))
take(element Type, value element) Result(element):
    return Taken(Taken(slot: Spent, value))
";
    check(source).unwrap();
}

#[test]
fn shorthand_preserves_field_refinement_obligations() {
    let source = "Bounds has
    lower in 0 to 16
    upper in 0 to 16 and >= lower
make(lower in 0 to 16, upper in 0 to 16 and >= lower) Bounds:
    return Bounds(upper, lower)
";
    check(source).unwrap();
    let invalid = source.replace("Bounds(upper, lower)", "Bounds(upper: lower, lower: upper)");
    assert_ne!(invalid, source);
    assert_eq!(check(&invalid).unwrap_err(), SourceCheckError::Unproved);
}

#[test]
fn duplicate_or_missing_field_assignments_reject_before_reading_payloads_twice() {
    for arguments in ["left, left", "left, left: left", "left"] {
        let source = format!(
            "Pair(element) has left element, right element
bad(element Type, left element, right element) Pair(element):
    return Pair({arguments})
"
        );
        assert_eq!(
            check(&source).unwrap_err(),
            SourceCheckError::Type,
            "{source}"
        );
    }
}

#[test]
fn unnamed_expressions_and_unknown_names_are_not_positional_constructor_fallbacks() {
    for argument in ["other", "0", "(value)"] {
        let source = format!(
            "Only has value in 0 to 16
bad(value in 0 to 16, other in 0 to 16) Only: return Only({argument})
"
        );
        assert_eq!(
            check(&source).unwrap_err(),
            SourceCheckError::Type,
            "{source}"
        );
    }
}

#[test]
fn shorthand_moves_an_owned_value_once_and_does_not_allow_duplication() {
    let source = "Full(element) has value element
forward(element Type, value element) Full(element):
    return Full(value)
";
    check(source).unwrap();
    let duplicated = "Pair(element) has left element, right element
bad(element Type, left element) Pair(element): return Pair(left, right: left)
";
    assert_eq!(
        check(duplicated).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn shorthand_does_not_bypass_imported_constructor_privacy() {
    let text = "Secret(element) has
    #private
    value element
make(element Type, value element) Secret(element): return Secret(value)
";
    let source = parser::parse_strict(text).unwrap();
    let library = check_module("library.secret", &source, &[], Limits::default()).unwrap();
    let client = parser::parse_strict(
        "use library.secret Secret
bad(element Type, value element) Secret(element): return Secret(value)
",
    )
    .unwrap();
    let error = check_module("client", &client, &[&library], Limits::default()).unwrap_err();
    assert_eq!(error, SourceCheckError::Privacy);
}

#[test]
fn a_shorthand_value_can_resolve_the_same_nullary_computation_as_its_long_form() {
    let source = "Token has
Product has value Token
value Token: return Token
make Product: return Product(value)
";
    check(source).unwrap();
    check(&source.replace("Product(value)", "Product(value: value)")).unwrap();
}

#[test]
fn bare_constructor_arguments_still_validate_their_source_spelling() {
    let text = "Full(element) has value element
make(element Type, value element) Full(element): return Full(value)
";
    let mut source = parser::parse_strict(text).unwrap();
    let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
        panic!("make binding");
    };
    let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
        panic!("body");
    };
    let ast::Statement::Return(ret) = &mut block.statements[0].value else {
        panic!("return");
    };
    let Expr::Call { arguments, .. } = &mut ret.value.as_mut().unwrap().value else {
        panic!("constructor");
    };
    let Argument::Positional(value) = &mut arguments.value[0].value else {
        panic!("bare argument");
    };
    value.value = Expr::Name("other".into());
    let before = source.clone();
    assert_eq!(
        check_file(&source, Limits::default()).unwrap_err(),
        SourceCheckError::Malformed
    );
    assert_eq!(source, before);
}
