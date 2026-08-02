//! Traits provided by unified declarations must resolve trait names in scope.

use typecheck::TypedFileAst;

mod support;
use support::transform_source;

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

#[test]
fn provided_trait_to_string_ok_when_declared_in_same_file() {
    let source = "\
ToString has to_string String

Bool is True or False
Bool has ToString
    ToString.to_string: when self then 'true' else 'false'
";
    let typed = transform_source(source);
    let unknown = unknown_symbol_names(&typed);
    assert!(
        unknown.is_empty(),
        "ToString is declared in this file, should not be UnknownSymbol: {unknown:?}"
    );
}
