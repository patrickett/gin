use diagnostic::DiagnosticCode;
use diagnostic::code::{BindingCode, TypeCode};
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn validity_flaws(source: &str) -> usize {
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    typed
        .all_flaws()
        .into_iter()
        .filter(|(_, flaw)| {
            flaw.code
                == DiagnosticCode::Type(TypeCode::Binding(BindingCode::RebindViolatesValidity))
        })
        .count()
}

#[test]
fn function_results_and_choices_preserve_the_declared_integer_domain() {
    let source = "Signed is in -9223372036854775808 to 9223372036854775807\nread Signed: -4\nmain:\n    value is in -9223372036854775808 to 9223372036854775807: 0\n    value :: read\n    value :: when value < 0 then read else value\n    return value\n";
    assert_eq!(validity_flaws(source), 0);
}

#[test]
fn wider_function_results_cannot_replace_narrow_integer_storage() {
    let source = "Wide is in 0 to 65535\nread Wide: 256\nmain:\n    value is in 0 to 255: 0\n    value :: read\n    return value\n";
    assert!(validity_flaws(source) > 0);
}

#[test]
fn arithmetic_still_requires_proof_that_the_rebind_fits() {
    let source = "main:\n    value is in 0 to 255: 255\n    value :: value + 1\n    return value\n";
    assert!(validity_flaws(source) > 0);
}
