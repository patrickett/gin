//! Text that exists only at compile time: `Symbol.Putchar.to_string` folds to `'putchar'`.
//!
//! There is no name for the type of text, so nothing can be annotated, passed or stored as
//! text. A string literal is a text value, a function may return one (its result is
//! inferred, being unspellable), and an `#instruction` target must fold to a plain name.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, SourceType, check_module,
};

const TARGET: &str = "\
#unrestricted
#wrap
Word is in 0 to 18446744073709551615

#unrestricted
Saved has
    #register('x0')
    result Word
    #register('lr')
    link Word

#unrestricted
OneArgument has
    #register('x0')
    value Word

#instruction('bl $target')
branch_link(target, input Type, output Type, inputs input) output
";

const SYMBOL: &str = "\
#unrestricted
Symbol is Putchar
       or Labs

Symbol has
    to_string(self): when self is
        Putchar then 'putchar'
        Labs then 'labs'
";

fn module(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.text", &source, &[], Limits::default())
}

fn rejects(text: &str, expected: SourceCheckError) {
    assert_eq!(module(text).err(), Some(expected), "{text}");
}

fn call(target: &str) -> String {
    format!(
        "{TARGET}{SYMBOL}\
         put(character Word) Word:\n    \
         after := branch_link({target}, OneArgument, Saved, OneArgument(value: character))\n    \
         return after.result\n"
    )
}

fn symbol_of(text: &str, function: &str) -> Option<String> {
    let module = module(text).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == function)
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    body.expressions.iter().find_map(|expr| match &expr.kind {
        CheckedSourceBodyKind::Instruction { symbols, .. } => {
            symbols.first().map(|(_, symbol)| symbol.clone())
        }
        _ => None,
    })
}

#[test]
fn a_method_returns_text_and_its_result_is_inferred() {
    let module = module(SYMBOL).unwrap();
    let file = module.checked();
    let method = file
        .functions
        .iter()
        .find(|f| f.name == "Symbol.to_string")
        .unwrap();
    assert_eq!(method.result, SourceType::Text);
}

#[test]
fn a_variant_names_its_symbol_through_to_string() {
    assert_eq!(
        symbol_of(&call("Symbol.Putchar.to_string"), "put").as_deref(),
        Some("putchar")
    );
    assert_eq!(
        symbol_of(&call("Symbol.Labs.to_string"), "put").as_deref(),
        Some("labs")
    );
}

#[test]
fn the_call_form_with_parentheses_is_the_same() {
    let text = call("Symbol.Labs.to_string()");
    assert_eq!(symbol_of(&text, "put").as_deref(), Some("labs"));
}

#[test]
fn a_literal_is_still_a_target() {
    assert_eq!(
        symbol_of(&call("'putchar'"), "put").as_deref(),
        Some("putchar")
    );
}

#[test]
fn a_local_variant_folds_too() {
    let text = format!(
        "{TARGET}{SYMBOL}\
         put(character Word) Word:\n    \
         which := Symbol.Putchar\n    \
         after := branch_link(which.to_string, OneArgument, Saved, OneArgument(value: character))\n    \
         return after.result\n"
    );
    assert_eq!(symbol_of(&text, "put").as_deref(), Some("putchar"));
}

/// Which symbol is a run-time choice here, so there is nothing to put in the instruction.
#[test]
fn a_target_that_is_not_known_at_compile_time_is_refused() {
    let text = format!(
        "{TARGET}{SYMBOL}\
         put(which Symbol, character Word) Word:\n    \
         after := branch_link(which.to_string, OneArgument, Saved, OneArgument(value: character))\n    \
         return after.result\n"
    );
    rejects(&text, SourceCheckError::InstructionTarget);
}

#[test]
fn text_that_is_not_a_plain_name_is_refused_as_a_target_only() {
    let spaced = "\
#unrestricted
Symbol is Putchar or Labs

Symbol has
    to_string(self): when self is
        Putchar then 'two words'
        Labs then 'labs'
";
    module(spaced).unwrap();
    let text = format!(
        "{TARGET}{spaced}\
         put(character Word) Word:\n    \
         after := branch_link(Symbol.Putchar.to_string, OneArgument, Saved, \
         OneArgument(value: character))\n    return after.result\n"
    );
    rejects(&text, SourceCheckError::InstructionTarget);
}

/// `Text` names compile-time text (decided with the register records: a record may hold it, as a
/// compile-time-only record). A bare unannotated parameter is still a type parameter, not text.
#[test]
fn text_is_nameable_as_a_type() {
    assert!(module("Box has\n    value Text\n").is_ok());
    assert!(module("f(x Text) in 0 to 1: return 0\n").is_ok());
    assert!(module("Box has\n    value Other\n").is_err());
    // A bare name is a type parameter, not text.
    assert!(module("f(x) in 0 to 1: return 0\n").is_ok());
}

#[test]
fn text_is_not_a_number() {
    rejects("f() in 0 to 1: return 'a'\n", SourceCheckError::Type);
    let text = format!("{SYMBOL}f() in 0 to 9: return 1 + Symbol.Labs.to_string\n");
    rejects(&text, SourceCheckError::Type);
}

#[test]
fn the_arms_of_a_match_agree_on_text() {
    let mixed = "\
#unrestricted
Symbol is Putchar or Labs

Symbol has
    to_string(self): when self is
        Putchar then 'putchar'
        Labs then 1
";
    rejects(mixed, SourceCheckError::Type);
}

/// A function that returns text is never run: it has no machine form.
#[test]
fn a_text_function_has_a_recorded_body_and_a_text_result() {
    let module = module(SYMBOL).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "Symbol.to_string")
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    assert!(
        body.expressions
            .iter()
            .any(|expr| matches!(&expr.kind, CheckedSourceBodyKind::Text(text) if text == "labs"))
    );
}

#[test]
fn an_unterminated_or_escaped_literal_is_refused() {
    rejects(
        "f() in 0 to 1:\n    x := 'a\\nb'\n    return 0\n",
        SourceCheckError::Unsupported,
    );
}
