use internment::Intern;
use parser::parse_from_str;

#[test]
fn signature_help_includes_named_results() {
    let source = "pair() first Int, second Bit: return 1, True\nmain: pair(";
    let ast = parser::cursor::TokenCursor::parse_source(source);
    let help = crate::Backend::build_signature_help(
        source,
        &ast,
        tower_lsp::lsp_types::Position {
            line: 1,
            character: 10,
        },
    )
    .expect("signature help");
    assert_eq!(help.signatures[0].label, "pair() first Int, second Bit");
}

#[test]
fn dot_completion_union_variants() {
    use typecheck::completions::dot_completions_for_ty;
    use typecheck::transform::transform_file;

    let source = "Maybe(x) is Some(x) or None";
    let po = parse_from_str(source);
    let typed = transform_file(po.clone(), typecheck::FileId(0));

    let nominal_ty = typed
        .tag_types
        .get(&typecheck::TagId(Intern::<String>::from_ref("Maybe")))
        .expect("Expected Maybe to resolve to a union type");
    let ty = typed.type_registry.resolved_definition_for_type(nominal_ty);
    let items = dot_completions_for_ty(ty);

    assert_eq!(items.len(), 2, "Expected 2 variants for Maybe type");

    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"Some(x)"), "Expected 'Some(x)' variant");
    assert!(labels.contains(&"None"), "Expected 'None' variant");

    let some_item = items.iter().find(|i| i.label == "Some(x)").unwrap();
    assert_eq!(some_item.detail.as_ref().unwrap(), "Maybe.Some(x)");
}
