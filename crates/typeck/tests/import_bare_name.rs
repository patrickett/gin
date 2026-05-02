use diagnostic::{DiagnosticCode, TypeSymptom};
use parser::parse_source_full;
use typeck::TyEnv;

/// `use core` brings the prefix `core` into scope; a bare `core` in value position is not an
/// undefined binding — it is reported as not a valid expression.
#[test]
fn bare_package_import_root_is_not_expr_not_unknown() {
    let source = "use core\n\nmain:\n    core\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);
    let has_undef = symptoms
        .iter()
        .any(|d| d.message.contains("undefined binding"));
    assert!(!has_undef, "unexpected: {:?}", symptoms);
    assert!(
        symptoms.iter().any(|d| {
            matches!(&d.code, DiagnosticCode::Type(TypeSymptom::NotExpr { .. }))
                && d.message == "'core' is not an expression"
        }),
        "expected type-not-expr flaw: {:?}",
        symptoms
    );
}
