//! Type-level `PointerSize is when target.arch is …` prepare + resolve.

use ast::{BindValue, ConstValue, DeclareValue, TypeExpr};
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::analysis::when_declare_is_exhaustive;
use typecheck::{infer_when_declare_subject_ty, materialize_when_declare_subjects_from_package};
use typecheck::{prepare_file_ast, prepare_package_asts};

fn target_fixture() -> &'static str {
    r#"
Architecture is 'x86_64' or 'arm64' or 'wasm32'
Vendor is 'unknown'
OperatingSystem is 'unknown'
Default(value) has (default value)
Target has (arch Architecture, vendor Vendor, os OperatingSystem)
Target.Default(default: ( arch: 'x86_64', vendor: 'unknown', os: 'unknown', ))
target Target
"#
}

fn pointer_fixture() -> &'static str {
    pointer_fixture_with_else()
}

fn pointer_fixture_with_else() -> &'static str {
    r#"
use '../target/'.target
use BigInt, Int

PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
    else in 0...0
"#
}

fn pointer_fixture_without_else() -> &'static str {
    r#"
use '../target/'.target
use BigInt, Int

PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
"#
}

fn pointer_fixture_partial_without_else() -> &'static str {
    r#"
use '../target/'.target
use BigInt, Int

PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
"#
}

fn pointer_size_alias_nominal(ast: &ast::FileAst) -> Option<Intern<String>> {
    let decl = ast.tags.get(&Intern::from_ref("PointerSize"))?;
    match &decl.value {
        DeclareValue::Alias(sp) => match &sp.value {
            TypeExpr::Nominal(name, _) => Some(*name),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn pointer_size_resolves_bigint_for_x86_64_entry() {
    let triple = TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap();
    let entry = CompileTarget::Concrete(triple);
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture()),
    ];
    let _ = prepare_package_asts(&mut asts, &entry);
    assert_eq!(
        pointer_size_alias_nominal(&asts[1]),
        Some(Intern::from_ref("BigInt")),
        "x86_64 entry should select BigInt arm"
    );
}

#[test]
fn entry_merge_sets_target_arch_from_triple() {
    let triple = TargetTriple::parse("wasm32-unknown-unknown").unwrap();
    let entry = CompileTarget::Concrete(triple);
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture()),
    ];
    let _ = prepare_package_asts(&mut asts, &entry);
    let target = asts[0]
        .defs
        .get(&Intern::from_ref("target"))
        .expect("target bind");
    let BindValue::Expr(expr) = &target.value else {
        panic!("expected expr target, got {:?}", target.value);
    };
    let arch = match &expr.const_value {
        Some(ConstValue::Record { fields }) => fields
            .iter()
            .find(|(n, _)| n.as_str() == "arch")
            .map(|(_, v)| v.clone()),
        Some(ConstValue::Tag { args, .. }) => args.first().cloned(),
        other => panic!("target should fold to record/tag, got {other:?}"),
    };
    assert_eq!(
        arch,
        Some(ConstValue::String("wasm32".to_string())),
        "entry merge should override arch for wasm32 triple"
    );
}

#[test]
fn pointer_size_resolves_int_for_wasm32_entry() {
    let triple = TargetTriple::parse("wasm32-unknown-unknown").unwrap();
    let entry = CompileTarget::Concrete(triple);
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture()),
    ];
    let _ = prepare_package_asts(&mut asts, &entry);
    assert_eq!(
        pointer_size_alias_nominal(&asts[1]),
        Some(Intern::from_ref("Int")),
        "wasm32 entry should select Int arm"
    );
}

#[test]
fn pointer_size_exhaustive_subject_ty_covers_architecture() {
    let entry = CompileTarget::Library;
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture()),
    ];
    for a in &mut asts {
        let _ = prepare_file_ast(a, &entry);
    }
    let mut merged = ast::FileAst::default();
    for a in &asts {
        merged.merge_from(a.clone());
    }
    materialize_when_declare_subjects_from_package(&mut asts[1], &merged);
    let when = match &asts[1]
        .tags
        .get(&Intern::from_ref("PointerSize"))
        .unwrap()
        .value
    {
        DeclareValue::When(w) => w,
        v => panic!("expected When, got {v:?}"),
    };
    assert!(
        merged.tags.contains_key(&Intern::from_ref("Architecture")),
        "merged package should include Architecture from target.gin"
    );
    let subject_ty = infer_when_declare_subject_ty(&when.subject, &merged.tags);
    assert!(
        subject_ty.is_some(),
        "Architecture union should be inferred for target.arch; subject={:?}",
        when.subject.as_ref().map(|s| (&s.value, &s.const_value))
    );
    assert!(
        when_declare_is_exhaustive(subject_ty.as_ref(), &when.arms),
        "all Architecture literals should be covered by is arms"
    );
}

#[test]
fn pointer_size_exhaustive_without_else_has_no_missing_else() {
    let entry = CompileTarget::Library;
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture_without_else()),
    ];
    let diags = prepare_package_asts(&mut asts, &entry);
    let has_missing = diags
        .iter()
        .flatten()
        .any(|d| d.code.slug() == "type-missing-else-arm");
    assert!(
        !has_missing,
        "all Architecture literals are covered, so no else should be required: {diags:?}"
    );
}

#[test]
fn pointer_size_partial_without_else_reports_missing_else() {
    let entry = CompileTarget::Library;
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture_partial_without_else()),
    ];
    let diags = prepare_package_asts(&mut asts, &entry);
    let has_missing = diags
        .iter()
        .flatten()
        .any(|d| d.code.slug() == "type-missing-else-arm");
    assert!(
        has_missing,
        "missing wasm32 coverage should require else or another branch: {diags:?}"
    );
}

#[test]
fn pointer_size_unreachable_else_when_arch_fully_covered() {
    let entry = CompileTarget::Library;
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture_with_else()),
    ];
    let diags = prepare_package_asts(&mut asts, &entry);
    let has_unreachable = diags
        .iter()
        .flatten()
        .any(|d| d.code.slug() == "type-unreachable-else-arm");
    assert!(
        has_unreachable,
        "expected UnreachableElseArm when all Architecture literals are covered: {diags:?}"
    );
}

#[test]
fn pointer_size_declare_simplified_to_alias_after_prepare() {
    let entry = CompileTarget::Library;
    let mut asts = [
        TokenCursor::parse_source(target_fixture()),
        TokenCursor::parse_source(pointer_fixture()),
    ];
    let _ = prepare_package_asts(&mut asts, &entry);
    let pointer_ast = asts[1].clone();
    let decl = pointer_ast
        .tags
        .get(&Intern::from_ref("PointerSize"))
        .expect("PointerSize");
    assert!(
        matches!(&decl.value, DeclareValue::Alias(_)),
        "prepare should fold when to alias, got {:?}",
        decl.value
    );
}
