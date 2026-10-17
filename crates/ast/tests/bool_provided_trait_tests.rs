//! Traits provided by unified declarations must resolve trait names in scope.

mod support;
use support::{transform_source, unknown_symbol_names};

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
