use ast::DeclareValue;
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_package_asts;

#[test]
fn explicit_target_query_selects_pointer_type_across_files() {
    for (target, expected) in [
        ("x86_64-unknown-linux-gnu", "Wide"),
        ("aarch64-apple-darwin", "Wide"),
        ("wasm32-unknown-unknown", "Small"),
    ] {
        let entry = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
        let mut asts = [
            TokenCursor::parse_source("platform := #target.arch\n"),
            TokenCursor::parse_source(
                "PointerSize is when platform is\n    'x86_64' or 'aarch64' then Wide\n    else Small\n",
            ),
        ];
        let diagnostics = prepare_package_asts(&mut asts, &entry);
        assert!(diagnostics.iter().all(Vec::is_empty), "{diagnostics:?}");
        let declaration = &asts[1].tags[&Intern::from_ref("PointerSize")];
        let DeclareValue::Alias(alias) = &declaration.value else {
            panic!("expected a selected alias: {:?}", declaration.value);
        };
        assert!(matches!(&alias.value, ast::Expr::AnonymousTag(name) if name.as_str() == expected));
    }
}

#[test]
fn target_name_does_not_override_an_ordinary_constant() {
    let entry = CompileTarget::Concrete(TargetTriple::parse("wasm32-unknown-unknown").unwrap());
    let mut asts = [TokenCursor::parse_source(
        "target := [arch: 'x86_64']\nPointerSize is when target.arch is\n    'x86_64' then Wide\n    else Small\n",
    )];
    prepare_package_asts(&mut asts, &entry);
    let declaration = &asts[0].tags[&Intern::from_ref("PointerSize")];
    assert!(
        matches!(&declaration.value, DeclareValue::Alias(alias) if matches!(&alias.value, ast::Expr::AnonymousTag(name) if name.as_str() == "Wide"))
    );
}
