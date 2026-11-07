use ast::{DeclarationKind, Expr, Item, SourceFile, SyntaxNodeKind, TypeBody, TypeMember};

const SUM_TYPES: &str = include_str!("../../test-fixtures/fixtures/type_sum_generic_payloads.gin");
const STORAGE: &str =
    include_str!("../../test-fixtures/fixtures/box_storage_growth_copy_append.gin");

// These fixtures also contain unsupported ranges, ownership, and control flow. Select complete
// supported declarations from the original bytes, rather than rewriting them into lookalikes.
fn snippet<'a>(fixture: &'a str, start: &str, end: &str) -> &'a str {
    let start = fixture.find(start).expect("fixture start marker");
    let length = fixture[start..].find(end).expect("fixture end marker");
    assert!(length > 0);
    &fixture[start..start + length]
}

fn declaration(file: &SourceFile, index: usize) -> &ast::TypeDeclaration {
    let Item::Type(declaration) = &file.items[index].value else {
        panic!("expected real fixture type declaration");
    };
    declaration
}

#[test]
fn real_sum_type_snippet_retains_or_calls_extensions_and_queries() {
    let source = snippet(SUM_TYPES, "Error is Empty or Full\n", "Holder has\n");
    let file = parser::parse_strict(source).expect("supported sum type fixture snippet");
    assert_eq!(file.text.as_ref(), source);
    let outline: Vec<_> = file
        .declaration_outline()
        .map(|entry| (entry.name, entry.kind, entry.name_span.extract(source)))
        .collect();
    assert_eq!(
        outline,
        [
            ("Error", DeclarationKind::Type, "Error"),
            ("Attempt", DeclarationKind::Type, "Attempt"),
            ("Attempt", DeclarationKind::Type, "Attempt"),
        ]
    );
    let TypeBody::Is(expression) = &declaration(&file, 1).body.value else {
        panic!("expected written sum expression");
    };
    let Expr::Binary { left, right, .. } = &expression.value else {
        panic!("expected written or");
    };
    assert!(matches!(left.value, Expr::Call { .. }));
    assert!(matches!(right.value, Expr::Call { .. }));
    let parameters = declaration(&file, 1).parameters.as_ref().unwrap();
    assert_eq!(parameters.value[0].value.name.value, "element");
    let hit = file
        .node_at_offset(source.find("Block(element)").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Block")));
    assert_eq!(hit.declarations.len(), 1);
    assert_eq!(hit.declarations[0].name, "Attempt");
    let hit = file.node_at_offset(source.find("return").unwrap()).unwrap();
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Attempt", "finish"]
    );
}

#[test]
fn real_generic_has_snippet_retains_fields_method_and_receiver() {
    let source = snippet(
        SUM_TYPES,
        "Block(element) has\n",
        "Error is Empty or Full\n",
    );
    let file = parser::parse_strict(source).expect("supported generic has fixture snippet");
    assert_eq!(file.items.len(), 1);
    let declaration = declaration(&file, 0);
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("expected fields and method");
    };
    let names: Vec<_> = members
        .iter()
        .map(|member| {
            let TypeMember::Binding(binding) = &member.value else {
                panic!("supported member must not recover");
            };
            binding.name.value.as_str()
        })
        .collect();
    assert_eq!(names, ["value", "count", "finish"]);
    let TypeMember::Binding(method) = &members[2].value else {
        unreachable!();
    };
    assert_eq!(
        method.parameters.as_ref().unwrap().value[0]
            .value
            .name
            .value,
        "self"
    );
    assert!(matches!(
        method.value.as_ref().unwrap().expression.value,
        Expr::Block(_)
    ));
    let hit = file
        .node_at_offset(source.find("Count(element)").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Count")));
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Block", "count"]
    );
    assert_eq!(
        file.declaration_outline()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Block"]
    );
}

#[test]
fn real_inline_method_line_retains_source_body_and_outline() {
    let source = STORAGE
        .split_inclusive('\n')
        .find(|line| line.starts_with("Marker has "))
        .expect("real inline Marker declaration");
    let file = parser::parse_strict(source).expect("supported inline method fixture line");
    assert_eq!(source, "Marker has release(self): return\n");
    let hit = file.node_at_offset(source.find("self").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("self")));
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Marker", "release"]
    );
    let outline: Vec<_> = file.declaration_outline().collect();
    assert_eq!(outline.len(), 1);
    assert_eq!(outline[0].name, "Marker");
    assert_eq!(outline[0].span.extract(source), source.trim_end());
}
