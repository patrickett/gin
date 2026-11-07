//! `#instruction` and `#register`: what the checker keeps and what it refuses.
use typecheck::{
    CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceFile, CheckedSourceModule, Limits,
    SourceCheckError, check_module,
};

const PRELUDE: &str = "\
#unrestricted
#wrap
Word is in 0 to 18446744073709551615

PutInputs has
    #register('x0')
    character Word

#unrestricted
Saved has
    #register('x0')
    result Word
    #register('lr')
    link Word

#instruction('bl $target')
branch_link(target, input Type, output Type, inputs input) output

put(character Word) Word:
    after := branch_link('putchar', PutInputs, Saved, PutInputs(character: character))
    return after.result
";

fn module(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.boundary", &source, &[], Limits::default())
}

fn checked(extra: &str) -> CheckedSourceModule {
    module(&format!("{PRELUDE}{extra}")).unwrap()
}

fn body<'a>(file: &'a CheckedSourceFile, name: &str) -> &'a CheckedSourceBody {
    let index = file.functions.iter().position(|f| f.name == name).unwrap();
    file.bodies[index].as_ref().expect("a recorded body")
}

fn rejects(text: &str, expected: SourceCheckError) {
    assert_eq!(module(text).err(), Some(expected), "{text}");
}

#[test]
fn the_attributes_reach_the_checked_file() {
    let module = checked("");
    let file = module.checked();
    let branch = file
        .functions
        .iter()
        .find(|f| f.name == "branch_link")
        .unwrap();
    assert_eq!(branch.instruction.as_deref(), Some("bl $target"));
    assert_eq!(branch.literals, ["target"]);
    let schema = |name: &str| file.aggregates.iter().find(|a| a.name == name).unwrap();
    assert_eq!(
        schema("PutInputs").fields[0].register.as_deref(),
        Some("x0")
    );
    assert_eq!(schema("Saved").fields[1].register.as_deref(), Some("lr"));
    assert!(!schema("Saved").fields[0].private);
}

#[test]
fn an_instruction_function_has_no_body_and_ordinary_functions_are_unmarked() {
    let module = checked("");
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "branch_link")
        .unwrap();
    assert!(file.bodies[index].is_none());
    assert!(
        file.functions
            .iter()
            .find(|f| f.name == "put")
            .unwrap()
            .instruction
            .is_none()
    );
}

#[test]
fn a_call_records_the_instruction_its_symbol_and_its_input() {
    let module = checked("");
    let body = body(module.checked(), "put");
    assert!(body.effects);
    let found = body.expressions.iter().find_map(|expr| match &expr.kind {
        CheckedSourceBodyKind::Instruction {
            text,
            symbols,
            input,
        } => Some((text.clone(), symbols.clone(), *input)),
        _ => None,
    });
    let (text, symbols, input) = found.expect("an instruction node");
    assert_eq!(text, "bl $target");
    assert_eq!(symbols, [("target".to_owned(), "putchar".to_owned())]);
    assert!(matches!(
        body.expressions[input as usize].kind,
        CheckedSourceBodyKind::Construct { .. }
    ));
}

#[test]
fn a_pure_function_has_no_effects() {
    let module = checked("three() in 0 to 5: return 3\n");
    assert!(!body(module.checked(), "three").effects);
}

/// Every input is known, but the call still has to happen.
#[test]
fn known_arguments_never_fold_an_effect_away() {
    let module = checked("hello() Word: return put(72)\n");
    let hello = body(module.checked(), "hello");
    assert!(hello.effects);
    assert!(
        hello
            .expressions
            .iter()
            .any(|expr| matches!(expr.kind, CheckedSourceBodyKind::Call { .. }))
    );
}

/// The result is a constant, but the effect in the statement before it is not.
#[test]
fn a_constant_result_never_folds_an_effect_away() {
    let module =
        checked("say() in 0 to 5:\n    put(72)\n    return 3\ncaller() in 0 to 5: return say()\n");
    let file = module.checked();
    assert!(body(file, "say").effects);
    let caller = body(file, "caller");
    assert!(caller.effects, "effects are transitive");
    assert!(
        caller
            .expressions
            .iter()
            .any(|expr| matches!(expr.kind, CheckedSourceBodyKind::Call { .. }))
    );
}

#[test]
fn inputs_are_consumed_by_the_instruction() {
    rejects(
        &format!(
            "{PRELUDE}twice(character Word) Word:\n    inputs := PutInputs(character: character)\n    \
             first := branch_link('putchar', PutInputs, Saved, inputs)\n    \
             second := branch_link('putchar', PutInputs, Saved, inputs)\n    return second.result\n"
        ),
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn misplaced_attributes_are_refused() {
    use SourceCheckError::InstructionTarget as Target;
    rejects(
        "#instruction('bl $target')\nf() in 0 to 1: return 0\n",
        Target,
    );
    rejects("#instruction('bl $target')\nS has\n", Target);
    rejects("#instruction('bl $target')\n", Target);
    rejects("#instruction('bl $target')\nf()\n", Target);
    rejects("S has\n    #register('{x0}')\n    a in 0 to 1\n", Target);
}

#[test]
fn a_register_is_placed_once_per_record() {
    rejects(
        "S has\n    #register('x0')\n    a in 0 to 1\n    #register('x0')\n    b in 0 to 1\n",
        SourceCheckError::Duplicate,
    );
}

#[test]
fn a_function_without_a_body_or_a_marker_is_still_missing_its_body() {
    rejects("f() in 0 to 1\n", SourceCheckError::MissingBody);
}

fn call(target: &str) -> String {
    format!(
        "{PRELUDE}go(other Word) Word:\n    \
         after := branch_link({target}, PutInputs, Saved, PutInputs(character: other))\n    \
         return after.result\n"
    )
}

#[test]
fn the_symbol_must_be_a_literal_plain_name() {
    use SourceCheckError::InstructionTarget as Target;
    assert!(module(&call("'puts'")).is_ok());
    rejects(&call("other"), Target);
    rejects(&call("72"), Target);
    rejects(&call("'two words'"), Target);
    rejects(&call("''"), Target);
}

/// Text is a compile-time value (see `compile_time_text.rs`), never a number or a machine
/// value: it cannot be returned as one or handed to a function that takes one.
#[test]
fn a_string_literal_is_text_and_nothing_else() {
    assert!(module("f() in 0 to 1: return 'a'\n").is_err());
    assert!(module(&format!("{PRELUDE}f() Word: return put('a')\n")).is_err());
    assert!(module("f() in 0 to 1:\n    x := 'a'\n    return 0\n").is_ok());
}

#[test]
fn an_unannotated_parameter_is_a_symbol_only_in_an_instruction_whose_text_places_it() {
    // Outside an instruction a bare parameter is a type parameter.
    assert!(module("f(a) in 0 to 1: return 0\n").is_ok());
    rejects(
        "#instruction('bl $target')\nf(a, b) in 0 to 1\n",
        SourceCheckError::InstructionTarget,
    );
}
