//! Tests for [`TypedFileAst::classify_hover`] — the classification step
//! that determines *what kind of thing* is at the cursor position.
//!
//! Each test asserts that a cursor position produces a specific [`HoverTarget`]
//! variant, independently of the rendering logic.

mod support;
use support::transform_source;

use ast::HoverDoc;
use ast::UnionVariant;
use ast::source::SourceExt;
use internment::Intern;
use typecheck::HoverTarget;
use typecheck::ty::Ty;

fn classify(source: &str, line: u32, character: u32) -> Option<HoverTarget> {
    let typed = transform_source(source);
    let byte_offset = source.position_to_byte_offset(line, character)?;
    let word = source.symbol_at_byte_offset(byte_offset)?;
    typed.classify_hover(source, byte_offset, &word, None)
}

fn classify_at_needle(source: &str, needle: &str) -> Option<HoverTarget> {
    let typed = transform_source(source);
    let byte_offset = source.find(needle)?;
    let word = source.symbol_at_byte_offset(byte_offset)?;
    typed.classify_hover(source, byte_offset, &word, None)
}

#[test]
fn self_in_has_method_body_classified_as_selfref() {
    let source = "Region has\n    reset(self) Region: self\n";
    let target = classify_at_needle(source, "self\n").expect("classify body self");
    match target {
        HoverTarget::SelfRef { modifier, .. } => {
            assert_eq!(modifier.as_str(), "self");
        }
        other => panic!("expected SelfRef hover, got {other:?}"),
    }
}

#[test]
fn ref_and_mut_self_in_has_method_body_include_modifier() {
    let cases = [
        ("Region has\n    reset(ref self) Region: self\n", "ref self"),
        ("Region has\n    reset(mut self) Region: self\n", "mut self"),
    ];

    for (source, expected_name) in cases {
        let target = classify_at_needle(source, "self\n").expect("classify body self");
        match target {
            HoverTarget::SelfRef { modifier, .. } => {
                assert_eq!(modifier.as_str(), expected_name);
            }
            other => panic!("expected SelfRef hover, got {other:?}"),
        }
    }
}

#[test]
fn self_in_associated_has_method_reports_specific_diagnostic_inline_body() {
    let source = "Region has
    allocate(size Int) Int: self
";
    let typed = transform_source(source);
    let codes: Vec<_> = typed
        .all_flaws()
        .iter()
        .map(|(_, diagnostic)| diagnostic.code.slug())
        .collect();
    assert!(
        codes.contains(&"no-self-in-associated-method"),
        "expected no-self-in-associated-method in inline body, got {codes:?}"
    );
}

#[test]
fn self_in_associated_has_method_reports_specific_diagnostic_block_body() {
    // Block body style: method body is indented after colon
    let source = "Region has
    allocate(size Int) Int:
        self
";
    let typed = transform_source(source);
    let codes: Vec<_> = typed
        .all_flaws()
        .iter()
        .map(|(_, diagnostic)| diagnostic.code.slug())
        .collect();
    assert!(
        codes.contains(&"no-self-in-associated-method"),
        "expected no-self-in-associated-method in block body, got {codes:?}"
    );
}

#[test]
fn has_block_body_full_region() {
    let source = r#"Region has
    start Int
    end Int

    new(start Int, end Int) Self: Self(start, end)
"#;
    let typed = transform_source(source);
    let has_parse_error = typed.all_flaws().iter().any(|(_, d)| {
        d.code.slug() == "parse-expected-expression" || d.code.slug() == "parse-expected-return"
    });
    assert!(
        !has_parse_error,
        "expected no parse errors, got: {:?}",
        typed
            .all_flaws()
            .iter()
            .map(|(_, d)| (d.code.slug(), &d.message))
            .collect::<Vec<_>>()
    );

    let has_self_error = typed
        .all_flaws()
        .iter()
        .any(|(_, d)| d.code.slug() == "type-unknown-symbol" && d.message.contains("Self"));

    assert!(
        !has_self_error,
        "Self should resolve in this context: {:?}",
        typed
            .all_flaws()
            .iter()
            .map(|(_, d)| (d.code.slug(), &d.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn definition_name_classified() {
    let source = "two := 1 + 1\n";
    // Cursor on 'two' in the definition
    let target = classify(source, 0, 0).expect("classify on definition name");
    assert!(
        matches!(target, HoverTarget::Definition(_)),
        "expected Definition, got {target:?}"
    );
}

#[test]
fn function_name_classified() {
    let source = "foo() Int := 42\n";
    let target = classify(source, 0, 0).expect("classify on function name");
    assert!(
        matches!(target, HoverTarget::Definition(_)),
        "expected Definition for function name, got {target:?}"
    );
}

#[test]
fn tag_name_classified() {
    let source = "Bool is True or False\n";
    let target = classify(source, 0, 0).expect("classify on tag name");
    assert!(
        matches!(target, HoverTarget::TagDecl(_)),
        "expected TagDecl, got {target:?}"
    );
}

#[test]
fn tag_body_classified() {
    let source = "Small is in 0...100\n";
    // Cursor on '100' (the tag body, not the name 'Small')
    let target = classify(source, 0, 17).expect("classify on tag body");
    assert!(
        matches!(target, HoverTarget::TagAtByte(_)),
        "expected TagAtByte for tag body, got {target:?}"
    );
}

#[test]
fn expr_bind_classified() {
    let source = "main:\n    x := 1\n    return 0\n";
    // Cursor on 'x' in `x := 1`
    let target = classify(source, 1, 4).expect("classify on local bind");
    assert!(
        matches!(target, HoverTarget::Expr(_)),
        "expected Expr for local bind name, got {target:?}"
    );
}

#[test]
fn expr_fn_call_classified() {
    let source = "foo() Int := 42\nmain:\n    foo()\n    return 0\n";
    // Cursor on 'foo()' call in main
    let target = classify(source, 2, 4).expect("classify on fn call");
    assert!(
        matches!(target, HoverTarget::Expr(_)),
        "expected Expr for fn call, got {target:?}"
    );
}

#[test]
fn number_literal_classified() {
    let source = "main:\n    return 42\n";
    let target = classify(source, 1, 11).expect("classify on number literal");
    assert!(
        matches!(target, HoverTarget::Expr(_)),
        "expected Expr for number literal, got {target:?}"
    );
}

#[test]
fn def_name_wins_over_expr() {
    // When cursor is on a definition name, it must be classified as Definition,
    // not as Expr (which would also match the same position).
    let source = "two := 1 + 1\nmain:\n    return two\n";
    let target = classify(source, 0, 0).expect("classify on definition name");
    assert!(
        matches!(target, HoverTarget::Definition(_)),
        "definition name must win over overlapping Expr, got {target:?}"
    );
}

#[test]
fn tag_name_wins_over_expr() {
    // When cursor is on a tag declaration name, it must be classified as TagDecl,
    // not as Expr.
    let source = "Bool is True or False\n";
    let target = classify(source, 0, 0).expect("classify on tag name");
    assert!(
        matches!(target, HoverTarget::TagDecl(_)),
        "tag name must win over overlapping Expr, got {target:?}"
    );
}

/// Cursor on a non-final segment of an import path should classify as ModulePath.
#[test]
fn module_path_classified() {
    let source = "use core.maybe.Some\n\nmain:\n    return 0\n";
    let typed = transform_source(source);
    // Cursor on 'maybe' in 'core.maybe.Some'
    let byte_offset = source.position_to_byte_offset(0, 9).unwrap();
    let word = source.symbol_at_byte_offset(byte_offset).unwrap();
    let target = typed.classify_hover(source, byte_offset, &word, None);
    assert!(
        matches!(target, Some(HoverTarget::ModulePath(_))),
        "expected ModulePath for 'maybe' segment, got {target:?}"
    );
}

/// Cursor on the final segment of an import path should NOT classify as ModulePath.
#[test]
fn module_path_not_classified_for_final_segment() {
    let source = "use core.maybe.Some\n\nmain:\n    return 0\n";
    let typed = transform_source(source);
    // Cursor on 'Some' (final segment of import)
    let byte_offset = source.position_to_byte_offset(0, 15).unwrap();
    let word = source.symbol_at_byte_offset(byte_offset).unwrap();
    let target = typed.classify_hover(source, byte_offset, &word, None);
    // Should NOT be ModulePath (it's the item, not a module path segment)
    assert!(
        !matches!(target, Some(HoverTarget::ModulePath(_))),
        "expected non-ModulePath for final segment 'Some', got {target:?}"
    );
}

/// Cursor on 'core' (root of a multi-segment import path) should classify as ModulePath.
#[test]
fn module_path_root_classified() {
    let source = "use core.maybe.Some\n\nmain:\n    return 0\n";
    let typed = transform_source(source);
    // Cursor on 'core' in 'core.maybe.Some'
    let byte_offset = source.position_to_byte_offset(0, 4).unwrap();
    let word = source.symbol_at_byte_offset(byte_offset).unwrap();
    let target = typed.classify_hover(source, byte_offset, &word, None);
    assert!(
        matches!(target, Some(HoverTarget::ModulePath(_))),
        "expected ModulePath for root segment 'core', got {target:?}"
    );
}

// ── Hover doc rendering (shared markdown helpers) ────────────────────────

#[test]
fn hover_doc_matches_tag_golden_shape() {
    let md = HoverDoc::new()
        .gin("Bool is True or False")
        .ty_display("Bool")
        .prose("A boolean type.")
        .size_align_from_ty(
            &Ty::Union {
                name: Intern::from_ref("Bool"),
                variants: vec![
                    UnionVariant::new(Intern::from_ref("True"), vec![]),
                    UnionVariant::new(Intern::from_ref("False"), vec![]),
                ],
                literal_values: None,
                resolved_params: None,
            },
            &(),
            &(),
        )
        .render();

    assert_eq!(
        md,
        "\
```gin\nBool is True or False\n```\n---\n\n`Bool`\n---\n\nA boolean type."
    );
}
