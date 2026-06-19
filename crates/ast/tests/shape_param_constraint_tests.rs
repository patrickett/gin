//! Tests for parameter-level shape and trait constraints.
//!
//! The Gin language supports several constraint mechanisms:
//!
//! - **Inline `has`** on parameters: `process(x has Trait(field: Pattern))`
//! - **`Type.Trait(...)`** provided-trait implementation declarations
//! - **Blanket impls**: `x.Trait(...)` at the top level

mod support;

use support::transform_source;

/// Inline `has` on a parameter (`process(x has Trait(...))`) is now parseable.
#[test]
fn inline_has_on_param_parses() {
    let src = "\
Copy has (can_copy Bool)
Bool is True or False

process(x has Copy(can_copy: True)):
    42
";
    let typed = transform_source(src);
    // The `process` def should exist with the inline has constraint.
    let has_def = typed
        .defs
        .contains_key(&typecheck::DefId(internment::Intern::new(
            "process".to_string(),
        )));
    assert!(
        has_def,
        "process should parse with inline `x has Copy(...)`"
    );
    // No parse warnings expected for the new inline syntax.
    assert!(
        typed.parse_warnings.is_empty(),
        "no parse warnings expected for inline has syntax: {:?}",
        typed.parse_warnings
    );
}

/// `Type.Trait(...)` is the standard way to provide a trait implementation.
#[test]
fn provided_trait_on_declaration() {
    let src = "\
Copy has (can_copy Bool)
Bool is True or False

UniqueId has (id Int)
UniqueId.Copy(can_copy: False)
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

/// Blanket impl (`x.Trait(...)`) at top level is also well-supported.
#[test]
fn blanket_impl_parses() {
    let src = "\
Copy has (can_copy Bool)
Bool is True or False

x.Copy(can_copy: True)
";
    let typed = transform_source(src);
    // Blanket impls are stored on the AST during parsing and consumed by the
    // registry in typecheck.  At minimum, parsing + transform must not crash.
    let _flaws = typed.all_flaws();
}

/// Bind-level `and has` chain: `process(x) has Copy(...) and Sized(...):`
#[test]
fn bind_level_and_has_constraints() {
    let src = "\
Copy has (can_copy Bool)
Sized has (size Int)
Bool is True or False
Int is in 1...400

process(x) has Copy(can_copy: True) and Sized(size: 8):
    42
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();
    // The important thing is no crash. Both constraints should be parsed.
    let has_def = typed
        .defs
        .contains_key(&typecheck::DefId(internment::Intern::new(
            "process".to_string(),
        )));
    assert!(
        has_def,
        "process should parse with bind-level and-has constraints"
    );
    let _ = flaws;
}
