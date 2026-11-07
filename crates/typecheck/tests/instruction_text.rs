//! `#instruction` text is data: several instructions, and symbols placed by name.
//!
//! The text is the target library's own assembly, so it may hold anything but a quote or a
//! backslash, and `;` separates instructions. Each unannotated parameter is a symbol the
//! call supplies as compile-time text, and the text says where it goes with `$name`. The
//! parameter and the placeholder must agree, so a symbol is never dropped or invented.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, check_module,
};

const BASE: &str = "\
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
One has
    #register('x1')
    value Word
";

fn module(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.text", &source, &[], Limits::default())
}

fn rejects(text: &str, expected: SourceCheckError) {
    assert_eq!(module(text).err(), Some(expected), "{text}");
}

fn declare(attribute: &str, parameters: &str) -> String {
    format!(
        "{BASE}#instruction({attribute})\nrun({parameters}input Type, output Type, inputs input) output\n"
    )
}

fn call(declaration: &str, arguments: &str) -> String {
    format!(
        "{declaration}go(n Word) Word:\n    \
         after := run({arguments}One, Saved, One(value: n))\n    return after.result\n"
    )
}

fn symbols(text: &str) -> Vec<(String, String)> {
    let module = module(text).unwrap();
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == "go").unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    body.expressions
        .iter()
        .find_map(|expr| match &expr.kind {
            CheckedSourceBodyKind::Instruction { symbols, .. } => Some(symbols.clone()),
            _ => None,
        })
        .expect("an instruction node")
}

#[test]
fn the_text_may_hold_several_instructions_and_data() {
    let declaration = declare(
        "'sub sp, sp, #16; str x1, [sp]; bl $target; add sp, sp, #16'",
        "target, ",
    );
    assert_eq!(
        symbols(&call(&declaration, "'printf', ")),
        [("target".to_owned(), "printf".to_owned())]
    );
    module(&declare(
        "'adr x1, 1f; b 2f; 1: .ascii \"hi\"; .byte 10; 2:'",
        "",
    ))
    .unwrap();
}

#[test]
fn a_symbol_may_be_a_qualified_name() {
    let declaration = declare("'bl $target'", "target, ");
    assert_eq!(
        symbols(&call(&declaration, "'app.libc.compare', ")),
        [("target".to_owned(), "app.libc.compare".to_owned())]
    );
}

#[test]
fn several_symbols_are_placed_by_their_names() {
    let declaration = declare("'adr x3, $callback; bl $target'", "target, callback, ");
    let found = symbols(&call(&declaration, "'qsort', 'app.compare', "));
    assert_eq!(
        found,
        [
            ("target".to_owned(), "qsort".to_owned()),
            ("callback".to_owned(), "app.compare".to_owned())
        ]
    );
}

#[test]
fn a_block_with_no_symbol_takes_no_parameter() {
    let declaration = declare("'mov x0, x1'", "");
    module(&call(&declaration, "")).unwrap();
}

#[test]
fn a_placeholder_must_name_a_parameter() {
    rejects(
        &declare("'bl $missing'", "target, "),
        SourceCheckError::InstructionTarget,
    );
    rejects(
        &declare("'bl $target'", ""),
        SourceCheckError::InstructionTarget,
    );
}

#[test]
fn a_parameter_must_be_placed() {
    rejects(
        &declare("'mov x0, x1'", "target, "),
        SourceCheckError::InstructionTarget,
    );
    rejects(
        &declare("'bl $target'", "target, callback, "),
        SourceCheckError::InstructionTarget,
    );
}

/// Nothing but a named placeholder may use `$`: it is how the machine counts operands.
#[test]
fn a_stray_dollar_is_refused() {
    for attribute in ["'mov x0, $1'", "'mov x0, $$'", "'mov x0, ${0}'", "'bl $'"] {
        rejects(
            &declare(attribute, "target, "),
            SourceCheckError::InstructionTarget,
        );
    }
}

#[test]
fn the_text_has_no_quote_or_backslash_and_is_not_empty() {
    for attribute in ["''", "'a\\nb'"] {
        rejects(&declare(attribute, ""), SourceCheckError::InstructionTarget);
    }
}

#[test]
fn parameter_names_are_distinct() {
    rejects(
        &declare("'bl $target'", "target, target, "),
        SourceCheckError::Duplicate,
    );
}

#[test]
fn a_symbol_is_a_plain_name_with_dots() {
    let declaration = declare("'bl $target'", "target, ");
    for good in ["'printf'", "'app.libc.compare'", "'_x9'"] {
        module(&call(&declaration, &format!("{good}, "))).unwrap();
    }
    for bad in ["'two words'", "''", "'a;b'", "'a$b'", "'a\"b'"] {
        rejects(
            &call(&declaration, &format!("{bad}, ")),
            SourceCheckError::InstructionTarget,
        );
    }
}

/// `;` becomes a line break, so one inside quoted data would split it: refuse it, never
/// assemble something other than what was written.
#[test]
fn a_semicolon_inside_quoted_data_is_refused() {
    rejects(
        &declare("'b . + 8; .ascii \"a;b\"; .byte 0'", ""),
        SourceCheckError::InstructionTarget,
    );
    module(&declare("'b . + 8; .ascii \"ab\"; .byte 0, 0'", "")).unwrap();
}
