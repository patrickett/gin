//! Failures name their culprit, and non-fatal findings come back as notes.
use typecheck::{
    CheckedSourceFile, Limits, SourceCheckError, SourceCheckFailure, SourceCheckNoteKind,
    check_file_diagnostic,
};

fn run(text: &str) -> Result<CheckedSourceFile, SourceCheckFailure> {
    let file = parser::parse_strict(text).expect("strict diagnostic fixture");
    check_file_diagnostic(&file, Limits::default())
}

fn detail(text: &str, error: SourceCheckError) -> String {
    let failure = run(text).expect_err(text);
    assert_eq!(failure.error, error, "{text}");
    failure
        .detail
        .unwrap_or_else(|| panic!("no detail for {text}"))
}

#[test]
fn passing_a_linear_type_to_an_unrestricted_generic_names_both_types() {
    let text = "#unrestricted\nBox(element) has item element\nRes has v in 0 to 9\nuse_it(b Box(Res)): return\n";
    let message = detail(text, SourceCheckError::UnrestrictedArgument);
    assert!(
        message.contains("`Res`") && message.contains("`Box`"),
        "{message}"
    );
}

#[test]
fn an_unrestricted_record_names_the_field_that_holds_a_linear_type() {
    let text = "Res has v in 0 to 9\n#unrestricted\nHolder has\n    inner Res\n";
    let message = detail(text, SourceCheckError::UnrestrictedField);
    assert!(
        message.contains("`Holder`") && message.contains("`inner`"),
        "{message}"
    );
    assert!(message.contains("Res"), "{message}");
}

#[test]
fn a_linear_domain_without_a_terminal_names_the_domain() {
    let message = detail("FileId is in 0 to 255\n", SourceCheckError::MissingTerminal);
    assert!(message.contains("`FileId`"), "{message}");
}

#[test]
fn a_missing_trait_method_names_the_type_the_method_and_the_trait() {
    let text = "Closes has\n    close(self)\nCount is in 0 to 9\nCount has Closes\n";
    let message = detail(text, SourceCheckError::MissingBody);
    for name in ["`Count`", "`close`", "`Closes`"] {
        assert!(message.contains(name), "{message}");
    }
}

#[test]
fn receiver_and_operator_errors_name_the_method() {
    let misplaced = "#unrestricted\nP has\n    x in 0 to 1\n    m(a in 0 to 1, self): return\n";
    assert!(detail(misplaced, SourceCheckError::MethodReceiver).contains("`m`"));
    let operator = "#unrestricted\nP has\n    x in 0 to 1\n    #infix('**')\n    d(self, rhs Self) in 0 to 1: 0\n";
    let message = detail(operator, SourceCheckError::InfixOperator);
    assert!(
        message.contains("`d`") && message.contains("'**'"),
        "{message}"
    );
    let arity =
        "#unrestricted\nP has\n    x in 0 to 1\n    #infix('+')\n    a(self) in 0 to 1: 0\n";
    assert!(detail(arity, SourceCheckError::InfixOperator).contains("`a`"));
}

#[test]
fn an_error_without_a_known_culprit_has_no_detail() {
    let failure = run("f() in 0 to 1: return missing\n").unwrap_err();
    assert_eq!(failure.detail, None);
}

#[test]
fn a_do_nothing_terminal_on_an_unrestricted_type_is_a_located_note() {
    let source = "#unrestricted\nByte is in 0 to 255\nByte has\n    finish(self): return\n";
    let checked = run(source).unwrap();
    let [note] = &checked.notes[..] else {
        panic!("one note, got {:?}", checked.notes)
    };
    assert_eq!(note.kind, SourceCheckNoteKind::RedundantTerminal);
    assert!(note.detail.contains("`finish`") && note.detail.contains("`Byte`"));
    let span = note.span.expect("located");
    assert_eq!(span.extract(source), "finish(self): return");
    assert_eq!(note.file, Some(0));
}

#[test]
fn only_a_do_nothing_terminal_on_an_unrestricted_type_is_noted() {
    for source in [
        // Linear: the terminal is required, so it is not redundant.
        "FileId is in 0 to 255\nFileId has\n    close(self): return\n",
        // Unrestricted but the method does something.
        "#unrestricted\nByte is in 0 to 255\nByte has\n    value(self) in 0 to 255: self\n",
        // Unrestricted with extra parameters.
        "#unrestricted\nP has\n    x in 0 to 1\n    m(self, k in 0 to 1): return\n",
    ] {
        let checked = run(source).unwrap();
        assert!(checked.notes.is_empty(), "{source}: {:?}", checked.notes);
    }
}

#[test]
fn a_record_with_a_do_nothing_terminal_is_also_noted() {
    let checked = run("#unrestricted\nP has\n    x in 0 to 1\n    drop(self): return\n").unwrap();
    assert_eq!(checked.notes.len(), 1);
}
