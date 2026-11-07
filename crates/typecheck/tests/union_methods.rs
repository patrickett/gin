//! `Symbol has` beside `Symbol is Putchar or Labs`: methods on a union.
//!
//! A union's `has` block shares its name, like a domain's, and holds methods only: the union
//! has no fields. `self` is the union, so a method matches on it. A method with nothing to
//! pass is written without parentheses.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, check_module,
};

const SYMBOL: &str = "\
#unrestricted
Symbol is Putchar
       or Labs

Symbol has
    code(self) in 0 to 1: when self is
        Putchar then 0
        Labs then 1
";

fn module(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.methods", &source, &[], Limits::default())
}

fn rejects(text: &str, expected: SourceCheckError) {
    assert_eq!(module(text).err(), Some(expected), "{text}");
}

#[test]
fn a_union_has_methods() {
    let module = module(SYMBOL).unwrap();
    let file = module.checked();
    let method = file
        .functions
        .iter()
        .find(|f| f.name == "Symbol.code")
        .expect("the method is the function `Symbol.code`");
    assert_eq!(method.parameters.len(), 1, "`self` only");
}

#[test]
fn a_method_is_called_on_a_value_of_the_union_with_or_without_parentheses() {
    for call in ["symbol.code", "symbol.code()"] {
        let text = format!("{SYMBOL}by_value(symbol Symbol) in 0 to 1: return {call}\n");
        module(&text).unwrap();
    }
}

/// `Symbol.Labs.code`: the receiver is a variant written out, with nothing to pass.
#[test]
fn a_method_is_called_on_a_variant_with_or_without_parentheses() {
    for call in ["Symbol.Labs.code", "Symbol.Labs.code()"] {
        let text = format!("{SYMBOL}answer() in 0 to 1: return {call}\n");
        module(&text).unwrap();
    }
}

#[test]
fn a_method_on_a_known_variant_folds_at_compile_time() {
    let text = format!("{SYMBOL}answer() in 0 to 1: return Symbol.Labs.code\n");
    let module = module(&text).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "answer")
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    assert!(
        body.expressions
            .iter()
            .all(|expr| !matches!(expr.kind, CheckedSourceBodyKind::Call { .. })),
        "{:?}",
        body.expressions
    );
    assert!(body.expressions.iter().any(
        |expr| matches!(&expr.kind, CheckedSourceBodyKind::Integer(value) if *value == 1.into())
    ));
}

#[test]
fn a_method_may_take_more_than_self() {
    let text = "\
#unrestricted
Symbol is Putchar or Labs

Symbol has
    plus(self, by in 0 to 1) in 0 to 2: when self is
        Putchar then by
        Labs then by + 1

answer() in 0 to 2: return Symbol.Labs.plus(1)
";
    module(text).unwrap();
}

#[test]
fn a_method_is_called_on_a_local_variant() {
    let text =
        format!("{SYMBOL}answer() in 0 to 1:\n    labs := Symbol.Labs\n    return labs.code\n");
    module(&text).unwrap();
}

/// The union has no fields, so its `has` block holds methods and nothing else.
#[test]
fn a_union_has_no_fields() {
    let text = "\
Symbol is Putchar or Labs

Symbol has
    index in 0 to 1
";
    rejects(text, SourceCheckError::Unsupported);
}

#[test]
fn a_method_name_is_written_once() {
    let text = format!("{SYMBOL}    code(self) in 0 to 1: return 0\n");
    rejects(&text, SourceCheckError::Duplicate);
}

#[test]
fn a_union_still_needs_every_arm_in_its_methods() {
    let text = "\
#unrestricted
Symbol is Putchar or Labs

Symbol has
    code(self) in 0 to 1: when self is
        Putchar then 0
";
    rejects(text, SourceCheckError::Type);
}

/// A `has` block that names no union and has only methods is still a trait.
#[test]
fn a_methods_only_block_with_no_union_is_still_a_trait() {
    let text = "\
Shape has
    area(self) in 0 to 9: return 0
";
    module(text).unwrap();
}
