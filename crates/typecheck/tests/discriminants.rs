//! `=` discriminants and the always-wrap rule for `or` lists, each with negative controls.
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee, CheckedSourceFile, Discriminant,
    Evaluated, Limits, SourceCheckError, SourceCheckFailure, check_file_diagnostic, evaluate_value,
};
use i256::I256;

fn run(text: &str) -> Result<CheckedSourceFile, SourceCheckFailure> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}: {d:?}"));
    check_file_diagnostic(&file, Limits::default())
}

fn accepts(text: &str) -> CheckedSourceFile {
    run(text).unwrap_or_else(|failure| panic!("{text}\n{failure:?}"))
}

fn refuses(text: &str, error: SourceCheckError) -> String {
    let failure = run(text).expect_err(text);
    assert_eq!(failure.error, error, "{text}: {failure:?}");
    failure.detail.unwrap_or_default()
}

const EXIT: &str = "\
#unrestricted
ExitCode is Success   = 0
         or FirstFlaw = 1
         or SecFlaw   = 2
";

const LEVEL: &str = "\
#unrestricted
LogLevel is Debug = 'debug'
         or Info  = 'info'
         or Warn  = 'warn'
";

fn value_of(file: &CheckedSourceFile, name: &str) -> Option<Discriminant> {
    file.aggregates.iter().find(|a| a.name == name).unwrap().discriminant.clone()
}

#[test]
fn integer_and_text_values_are_recorded_on_each_variant() {
    let file = accepts(EXIT);
    assert_eq!(value_of(&file, "ExitCode.SecFlaw"), Some(Discriminant::Integer(I256::from(2))));
    assert_eq!(value_of(&file, "ExitCode"), None);
    let file = accepts(LEVEL);
    assert_eq!(value_of(&file, "LogLevel.Info"), Some(Discriminant::Text("info".to_owned())));
    accepts("#unrestricted\nSigned is Low = -5 or High = 5\n");
}

#[test]
fn a_union_with_payloads_has_no_values() {
    let detail = refuses("#unrestricted\nInt is in 0 to 9\nMixed is A = 1 or B(Int)\n", SourceCheckError::Type);
    assert!(detail.contains("payload"), "{detail}");
}

#[test]
fn values_are_all_or_none() {
    let detail = refuses("Partial is A = 1 or B\n", SourceCheckError::Type);
    assert!(detail.contains("every variant"), "{detail}");
}

#[test]
fn one_literal_kind_per_union() {
    let detail = refuses("Both is A = 1 or B = 'b'\n", SourceCheckError::Type);
    assert!(detail.contains("all integers or all text"), "{detail}");
}

#[test]
fn values_are_literals() {
    let detail = refuses("limit := 3\nE is A = limit or B = 4\n", SourceCheckError::Type);
    assert!(detail.contains("literal"), "{detail}");
    refuses("E is A = 1 + 1 or B = 4\n", SourceCheckError::Type);
}

#[test]
fn values_are_unique() {
    refuses("E is A = 1 or B = 1\n", SourceCheckError::Duplicate);
    refuses("E is A = 'x' or B = 'x'\n", SourceCheckError::Duplicate);
    // Byte for byte: these differ.
    accepts("E is A = 'x' or B = 'X'\n");
}

#[test]
fn a_valued_variant_cannot_share_a_type_name() {
    let detail = refuses("Debug has n in 0 to 1\nLevel is Debug = 1 or Info = 2\n", SourceCheckError::Type);
    assert!(detail.contains("another name"), "{detail}");
}

#[test]
fn value_reads_the_active_variants_literal() {
    let text = format!("{EXIT}status(code ExitCode) in 0 to 2: return code.value\nfixed in 2 to 2: ExitCode.SecFlaw.value\n");
    accepts(&text);
    // The range is exactly the values: 3 is not one of them.
    let narrow = format!("{EXIT}status(code ExitCode) in 0 to 1: return code.value\n");
    assert!(run(&narrow).is_err());
    // A variant written out folds to its own value.
    let wrong = format!("{EXIT}fixed in 1 to 1: ExitCode.SecFlaw.value\n");
    assert!(run(&wrong).is_err());
}

#[test]
fn a_union_without_values_has_no_value() {
    let text = "#unrestricted\nColor is Red or Blue\nf(c Color) in 0 to 9: return c.value\n";
    assert!(run(text).is_err());
}

struct NoBodies;
impl BodyResolver for NoBodies {
    fn body(&self, _: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        None
    }
}

#[test]
fn an_integer_value_is_recorded_and_evaluates_per_variant() {
    let text = format!("{EXIT}status(code ExitCode) in 0 to 2: return code.value\n");
    let file = accepts(&text);
    let index = file.functions.iter().position(|f| f.name == "status").unwrap();
    let body = file.bodies[index].as_ref().expect("recorded");
    assert!(body.expressions.iter().any(|e| matches!(e.kind, CheckedSourceBodyKind::Match { .. })), "{body:?}");
    for (variant, expected) in [("ExitCode.Success", 0), ("ExitCode.FirstFlaw", 1), ("ExitCode.SecFlaw", 2)] {
        let schema = file.aggregates.iter().position(|a| a.name == variant).unwrap() as u32;
        let argument = Evaluated::Record { schema, fields: Vec::new() };
        let value = evaluate_value(body, &[argument], &NoBodies, &mut 10_000);
        assert_eq!(value, Some(Evaluated::Integer(I256::from(expected))), "{variant}");
    }
}

#[test]
fn a_text_value_folds_on_a_known_variant() {
    let text = format!("{LEVEL}name(): return LogLevel.Warn.value\n");
    let file = accepts(&text);
    let index = file.functions.iter().position(|f| f.name == "name").unwrap();
    let body = file.bodies[index].as_ref().expect("recorded");
    assert!(
        body.expressions.iter().any(|e| matches!(&e.kind, CheckedSourceBodyKind::Text(t) if t == "warn")),
        "{body:?}"
    );
}

// The always-wrap rule.

#[test]
fn a_bare_name_that_is_a_type_is_refused_with_the_wrapped_form() {
    let text = "User has name in 0 to 9\nAdmin has level in 0 to 9\nPerson is User or Admin\n";
    let detail = refuses(text, SourceCheckError::Type);
    assert!(detail.contains("`User(User)`"), "{detail}");
    // Wrapped, it is a variant that shares the type's name.
    accepts("User has name in 0 to 9\nAdmin has level in 0 to 9\nPerson is User(User) or Admin(Admin)\nmake(u User) Person: return Person.User(u)\n");
}

#[test]
fn a_type_applied_to_several_arguments_is_refused_with_advice() {
    let text = "#unrestricted\nInt is in 0 to 9\n#unrestricted\nPair(a, b) has first a, second b\nEither is Pair(Int, Int) or Nothing\n";
    let detail = refuses(text, SourceCheckError::Type);
    assert!(detail.contains("wrap it in a variant"), "{detail}");
}

#[test]
fn a_bare_name_that_is_no_type_is_a_variant_as_before() {
    accepts("#unrestricted\nColor is Red or Blue\nf(c Color) in 0 to 1: when c is\n    Red then 0\n    Blue then 1\n");
}

// The inner-union arm is withdrawn.

const INNER: &str = "\
#unrestricted
Failure is OutOfMemory or Fragmented

#unrestricted
Done has n in 0 to 9

#unrestricted
Result is Done or Failure

code(r Result) in 0 to 2: when r is
    Done then 0
    Failure(failure) then 1
";

#[test]
fn the_arm_that_names_an_inner_union_is_refused() {
    let failure = run(INNER).expect_err("withdrawn");
    assert_eq!(failure.error, SourceCheckError::Type);
}

#[test]
fn a_payload_union_is_matched_through_its_wrapper() {
    let text = "\
#unrestricted
Failure is OutOfMemory or Fragmented

#unrestricted
Result is Done(Failure) or Fine

code(r Result) in 0 to 2: when r is
    Done(failure) then 1
    Fine then 0
";
    accepts(text);
}
