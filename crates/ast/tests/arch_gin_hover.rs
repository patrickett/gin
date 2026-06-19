//! Hover for an `Architecture` string-literal const union (canonical gin_core arch fixture).

use ast::source::SourceExt;
use internment::Intern;
use typecheck::{TagId, TypedFileAst};

mod support;
use support::transform_with_markers;

const ARCH_GIN: &str = r#"--- Target CPU architectures for conditional compilation.
Architecture is 'x86_64'
             or 'arm64'
             or 'wasm32'
"#;

const ARCHITECTURE_HOVER: &str = "\
```gin\nArchitecture is 'x86_64'\n             or 'arm64'\n             or 'wasm32'\n```\n---\n\nTarget CPU architectures for conditional compilation.";

fn arch_gin_source() -> String {
    ARCH_GIN.to_owned()
}

fn hover_on_architecture_name(typed: &TypedFileAst, source: &str) -> String {
    let byte = source
        .find("Architecture is")
        .or_else(|| source.find("Architecture"))
        .expect("Architecture in source");
    let (line, character) = source.byte_offset_to_position(byte);
    typed
        .hover_at(source, line, character)
        .unwrap_or_else(|| panic!("hover on Architecture at line {line} col {character}"))
}

#[test]
fn arch_gin_architecture_resolves_to_string_literal_const_union() {
    let arch = arch_gin_source();
    let typed = transform_with_markers(&arch);
    let tag = typed
        .tags
        .get(&TagId(Intern::new("Architecture".to_string())))
        .expect("Architecture tag");
    assert_eq!(
        tag.declaration_text,
        "Architecture is 'x86_64'\n             or 'arm64'\n             or 'wasm32'"
    );
}

#[test]
fn arch_gin_architecture_hover_markdown() {
    let arch = arch_gin_source();
    let typed = transform_with_markers(&arch);
    let hover = hover_on_architecture_name(&typed, &arch);
    assert_eq!(hover, ARCHITECTURE_HOVER);
}

#[test]
fn arch_gin_string_literal_variant_hover() {
    let arch = arch_gin_source();
    let typed = transform_with_markers(&arch);

    let byte = arch.find("'x86_64'").expect("'x86_64' literal in fixture");
    let (line, character) = arch.byte_offset_to_position(byte);
    let hover = typed
        .hover_at(&arch, line, character)
        .expect("hover on 'x86_64' variant");

    assert_eq!(
        hover,
        "```gin\nArchitecture is \'x86_64\'\n             or \'arm64\'\n             or \'wasm32\'\n```\n---\n\nTarget CPU architectures for conditional compilation."
    );
}
