use ast::DeclarationKind;

#[test]
fn incomplete_main_body_keeps_top_level_declarations_visible() {
    let source = "find_user(id Int) Int: return id\nmain:\n    user := find_user(\n";
    let output = parser::parse(source).expect("incomplete source is recoverable");
    assert!(output.has_errors());
    let declarations: Vec<_> = output.file.declaration_outline().collect();
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>(),
        ["find_user", "main"]
    );
    for (declaration, item) in declarations.iter().zip(&output.file.items) {
        assert_eq!(declaration.kind, DeclarationKind::Binding);
        assert_eq!(declaration.span, output.file.span_table.get(item.span_id));
        assert_eq!(declaration.name_span.extract(source), declaration.name);
    }
}

#[test]
fn duplicates_and_private_sections_keep_source_order_without_markers_or_expressions() {
    let source = "same := 1\nprivate\nsame() := 2\nsame Int\nsame()\n";
    let output = parser::parse(source).expect("parse completes");
    assert!(!output.has_errors());
    let declarations: Vec<_> = output.file.declaration_outline().collect();
    assert_eq!(declarations.len(), 3);
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>(),
        ["same", "same", "same"]
    );
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.span.extract(source))
            .collect::<Vec<_>>(),
        ["same := 1", "same() := 2", "same Int"]
    );
    assert!(
        declarations
            .windows(2)
            .all(|pair| pair[0].span.end() <= pair[1].span.start())
    );
    for declaration in declarations {
        assert_eq!(declaration.kind, DeclarationKind::Binding);
        assert_eq!(declaration.name_span.extract(source), "same");
    }
}

#[test]
fn malformed_top_level_items_are_skipped_without_hiding_later_declarations() {
    let source = "first := 1\n    bad := 2\nlater := 3\n";
    let output = parser::parse(source).expect("syntax errors are recoverable");
    assert!(output.has_errors());
    assert!(
        output
            .file
            .items
            .iter()
            .any(|item| item.value == ast::Item::Error)
    );
    assert_eq!(
        output
            .file
            .declaration_outline()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>(),
        ["first", "later"]
    );
}

#[test]
fn local_bindings_are_not_top_level_declarations() {
    let source = "main:\n    local := 1\n    return local\nlast := 2\n";
    let output = parser::parse(source).expect("parse completes");
    assert!(!output.has_errors());
    let declarations: Vec<_> = output.file.declaration_outline().collect();
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>(),
        ["main", "last"]
    );
    let ast::Item::Binding(main) = &output.file.items[0].value else {
        panic!("main is a binding");
    };
    assert!(std::ptr::eq(declarations[0].name, main.name.value.as_str()));
    assert_eq!(
        declarations[0].span.extract(source),
        "main:\n    local := 1\n    return local"
    );
}
