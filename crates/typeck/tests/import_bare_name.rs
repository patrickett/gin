use diagnostic::{DiagnosticCode, TypeSymptom};
use parser::parse_source_full;
use typeck::TyEnv;

#[test]
fn symbol_import_allows_true() {
    let source = "use core.true\n\nmain:\n    true\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    let has_undef = symptoms
        .iter()
        .any(|d| d.message.contains("undefined binding"));
    assert!(!has_undef, "unexpected undefined binding: {:?}", symptoms);

    let has_not_expr = symptoms
        .iter()
        .any(|d| matches!(&d.code, DiagnosticCode::Type(TypeSymptom::NotExpr { .. })));
    assert!(!has_not_expr, "unexpected NotExpr: {:?}", symptoms);
}

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

/// `use core.(true)` brings `true` into scope as a bundle member.
/// The name `true` should NOT get an "undefined binding" or "NotExpr"
/// diagnostic — it is recognised as an imported value symbol.
#[test]
fn bundle_member_import_does_not_report_not_expr() {
    let source = "use core.(true)\n\nmain:\n    true\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    // No "undefined binding" for `true`
    let has_undef = symptoms
        .iter()
        .any(|d| d.message.contains("undefined binding"));
    assert!(!has_undef, "unexpected undefined binding: {:?}", symptoms);

    // No "NotExpr" for `true` either (bundle members are value imports)
    let has_not_expr = symptoms
        .iter()
        .any(|d| matches!(&d.code, DiagnosticCode::Type(TypeSymptom::NotExpr { .. })));
    assert!(!has_not_expr, "unexpected NotExpr: {:?}", symptoms);
}

/// `use core.(true)` with multiple members works and each member is tracked
/// independently.
#[test]
fn bundle_member_import_multiple_members() {
    let source = "use core.(true, false)\n\nmain:\n    true\n    false\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    let has_undef = symptoms
        .iter()
        .any(|d| d.message.contains("undefined binding"));
    assert!(!has_undef, "unexpected undefined binding: {:?}", symptoms);

    let has_not_expr = symptoms
        .iter()
        .any(|d| matches!(&d.code, DiagnosticCode::Type(TypeSymptom::NotExpr { .. })));
    assert!(!has_not_expr, "unexpected NotExpr: {:?}", symptoms);
}

#[test]
fn module_prefix_not_imported_errors_for_core_true() {
    let source = "use core.true\n\nmain:\n    core.true\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert!(
        symptoms.iter().any(|d| {
            matches!(
                &d.code,
                DiagnosticCode::Type(TypeSymptom::UnknownBinding { name, .. })
                if name == "core.true"
            )
        }),
        "expected UnknownBinding for core.true: {:?}",
        symptoms
    );
}

#[test]
fn module_prefix_not_imported_errors_for_unknown_module() {
    let source = "main:\n    thing.fair\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert!(
        symptoms.iter().any(|d| {
            matches!(
                &d.code,
                DiagnosticCode::Type(TypeSymptom::UnknownBinding { name, .. })
                if name == "thing.fair"
            )
        }),
        "expected UnknownBinding for thing.fair: {:?}",
        symptoms
    );
}
