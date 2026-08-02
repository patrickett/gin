//! Tests for parameter-level shape and trait constraints.
//!
//! Unified declarations with provided traits.

mod support;

use support::transform_source;

#[test]
fn provided_trait_on_declaration() {
    let src = "\
Copy has can_copy Bool
Bool is True or False

UniqueId has Copy
    id Int
    Copy.can_copy: False
";
    let typed = transform_source(src);
    let tag_id = typecheck::TagId(internment::Intern::new("UniqueId".to_string()));
    let tag = typed
        .tags
        .get(&tag_id)
        .expect("UniqueId tag should be declared");
    let has_copy = tag
        .provided_traits
        .iter()
        .any(|pt| pt.trait_name.as_str() == "Copy");
    assert!(
        has_copy,
        "UniqueId should provide Copy; traits: {:?}",
        tag.provided_traits
    );
}
