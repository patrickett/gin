//! `#todo` is an explicit hole: the checker accepts it where a value of a known type is
//! expected, says so in a note, and records a `Hole` node for a consumer that can report it.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceFile, Limits, SourceCheckError, SourceCheckNoteKind,
    check_file,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    check_file(&parser::parse_strict(text).expect("parses"), Limits::default())
}

#[test]
fn a_returned_hole_is_accepted_noted_and_recorded() {
    let checked = check("f(a in 0 to 9) in 0 to 9: return #todo\n").unwrap();
    let notes: Vec<_> = checked.notes.iter().filter(|n| n.kind == SourceCheckNoteKind::Hole).collect();
    assert_eq!(notes.len(), 1, "{:?}", checked.notes);
    let body = checked.bodies[0].as_ref().expect("a body is recorded");
    let hole = body
        .expressions
        .iter()
        .find(|e| matches!(e.kind, CheckedSourceBodyKind::Hole))
        .expect("a Hole node");
    // The hole stands for the function's result, so it has that type.
    assert_eq!(format!("{:?}", hole.ty), format!("{:?}", checked.functions[0].result));
}

#[test]
fn a_hole_after_a_guard_keeps_the_guard() {
    let checked = check("f(a in 0 to 9) in 0 to 9:\n    when a < 5 then return 1\n    return #todo\n").unwrap();
    assert!(checked.notes.iter().any(|n| n.kind == SourceCheckNoteKind::Hole));
}

#[test]
fn a_hole_where_no_type_is_expected_is_still_refused() {
    // `a + #todo` has no expected type for the hole; the checker does not guess one.
    assert_eq!(
        check("f(a in 0 to 9) in 0 to 18: return a + #todo\n").unwrap_err(),
        SourceCheckError::Unsupported
    );
}

#[test]
fn a_hole_does_not_hide_other_errors() {
    // The rest of the function is still checked: this value is outside its result domain.
    let error = check("f(a in 0 to 9) in 0 to 3:\n    when a < 5 then return a\n    return #todo\n").unwrap_err();
    assert_ne!(error, SourceCheckError::Unsupported);
}
