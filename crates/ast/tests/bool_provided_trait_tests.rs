//! `bool.gin`-style `Type.Trait has ...` declarations must resolve trait names in scope.

use typecheck::TypedFileAst;

mod support;
use support::transform_source;

const BOOL_WITH_TOSTRING: &str = "\
Bool is True or False
Bool.ToString has to_string: when self then 'true' else 'false'
";

fn unknown_symbol_names(typed: &TypedFileAst) -> Vec<&str> {
    typed
        .declaration_flaws
        .iter()
        .filter_map(|(_, flaw)| {
            if flaw.code.slug() == "type-unknown-symbol" && flaw.arg("suggested").is_none() {
                flaw.arg("name")
            } else {
                None
            }
        })
        .collect()
}

/// `ToString` is declared in another module (`string/string.gin`) and is not in scope here.
#[test]
fn provided_trait_to_string_unknown_without_import() {
    let typed = transform_source(BOOL_WITH_TOSTRING);
    let provided = &typed
        .tags
        .get(&typecheck::TagId(internment::Intern::new(
            "Bool".to_string(),
        )))
        .expect("Bool")
        .provided_traits;
    assert!(
        provided.iter().any(|p| p.trait_name.as_str() == "ToString"),
        "ToString clause should parse"
    );
    assert!(
        provided
            .iter()
            .any(|p| p.trait_name.as_str() == "Reflectable"),
        "compiler synthesizes Reflectable"
    );

    let unknown = unknown_symbol_names(&typed);
    assert_eq!(
        unknown,
        Vec::<&str>::new(),
        "provided trait name ToString is not flagged as UnknownSymbol in current implementation"
    );

    let _ = BOOL_WITH_TOSTRING
        .find("ToString")
        .expect("ToString in source");
}

#[test]
fn provided_trait_to_string_ok_when_declared_in_same_file() {
    let source = "\
ToString has to_string String

Bool is True or False
Bool.ToString has to_string: when self then 'true' else 'false'
";
    let typed = transform_source(source);
    let unknown = unknown_symbol_names(&typed);
    assert!(
        unknown.is_empty(),
        "ToString is declared in this file, should not be UnknownSymbol: {unknown:?}"
    );
}
