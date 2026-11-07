use ast::{Expr, Item, SourceFile, SyntaxNodeKind, TypeBody, TypeMember};

fn declaration(file: &SourceFile, index: usize) -> &ast::TypeDeclaration {
    let Item::Type(declaration) = &file.items[index].value else {
        panic!("expected type declaration: {:?}", file.items[index]);
    };
    declaration
}

fn members(file: &SourceFile, index: usize) -> &[ast::Spanned<TypeMember>] {
    let TypeBody::Has(members) = &declaration(file, index).body.value else {
        panic!("expected has body");
    };
    members
}

#[test]
fn definitions_and_extensions_preserve_source_order_and_duplicates() {
    let source = "\
Outcome is Ready(Int) or Failed
Point has x Int, y Int
private
Point has
    x Int
    read(self) Int:
        local := self.x
        return local
    read(self) Int: return self.x
";
    let file = parser::parse_strict(source).expect("simple type declarations");
    assert_eq!(file.items.len(), 4);
    let TypeBody::Is(expression) = &declaration(&file, 0).body.value else {
        panic!("written type expression");
    };
    assert!(matches!(expression.value, Expr::Binary { .. }));
    assert_eq!(members(&file, 1).len(), 2);
    assert_eq!(members(&file, 3).len(), 3);
    let outline: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(outline, ["Outcome", "Point", "Point"]);
    let hit = file
        .node_at_offset(source.find("local :=").unwrap())
        .unwrap();
    let owners: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
    assert_eq!(owners, ["Point", "read", "local"]);
}

#[test]
fn is_composition_remains_a_written_expression_not_assembled_trait_metadata() {
    let file = parser::parse_strict("InOut is Input and Output\n").unwrap();
    let TypeBody::Is(expression) = &declaration(&file, 0).body.value else {
        panic!("written is expression");
    };
    let Expr::Binary {
        left,
        operator,
        right,
    } = &expression.value
    else {
        panic!("written composition");
    };
    assert_eq!(left.value, Expr::Name("Input".into()));
    assert_eq!(operator.value, ast::BinaryOperator::And);
    assert_eq!(right.value, Expr::Name("Output".into()));
    assert_eq!(
        file.span_table.get(operator.span_id).extract(&file.text),
        "and"
    );
    assert_eq!(file.declaration_outline().count(), 1);
}

#[test]
fn simple_generic_types_and_indented_aliases_keep_written_structure() {
    let source = "Box(element) has value element\nAlias() is\n    Box(Int)\nMarker has\n";
    let file = parser::parse_strict(source).expect("simple named type parameters");
    let parameters = declaration(&file, 0).parameters.as_ref().unwrap();
    assert_eq!(parameters.value[0].value.name.value, "element");
    assert!(
        declaration(&file, 1)
            .parameters
            .as_ref()
            .unwrap()
            .value
            .is_empty()
    );
    assert!(matches!(declaration(&file, 1).body.value, TypeBody::Is(_)));
    assert!(members(&file, 2).is_empty());
}

#[test]
fn empty_has_and_simple_member_signatures_are_not_invented_definitions() {
    for source in ["Numeric has", "Numeric has\n", "Numeric has\n    -- docs\n"] {
        let file = parser::parse_strict(source).expect("empty has is valid source");
        assert!(members(&file, 0).is_empty());
    }
    let source = "\
Factory(element, capacity Count) has
    value element, count Count
    create() Self
    size Count: compute_size(Self)
";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(members(&file, 0).len(), 4);
    let TypeMember::Binding(create) = &members(&file, 0)[2].value else {
        panic!("written method");
    };
    assert!(create.parameters.as_ref().unwrap().value.is_empty());
    assert!(create.value.is_none(), "no generated method body");
    let TypeMember::Binding(size) = &members(&file, 0)[3].value else {
        panic!("written member initializer");
    };
    assert!(size.parameters.is_none());
    assert!(size.value.is_some());
}

#[test]
fn incomplete_and_rejected_members_preserve_the_type_and_following_declarations() {
    let source = "\
Thing has
    first Int
    @bad
    second Int
    read(self) Int:
        value := receiver.
        return value
Alias is
later := 2
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(matches!(
        members(&output.file, 0)[1].value,
        TypeMember::Error
    ));
    assert_eq!(members(&output.file, 0).len(), 4);
    let TypeBody::Is(expression) = &declaration(&output.file, 1).body.value else {
        panic!("incomplete alias stays a declaration");
    };
    assert_eq!(expression.value, Expr::Missing);
    let outline: Vec<_> = output
        .file
        .declaration_outline()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(outline, ["Thing", "Alias", "later"]);
    assert!(parser::parse_strict(source).is_err());
}

#[test]
fn unsupported_type_forms_are_diagnosed_not_reinterpreted() {
    for source in [
        "Byte is in 0 to 255 and has Trait\n",
        "Thing has (value Int)\n",
        "Thing has\n    Trait.method: return 1\n",
        "Box(Element) has value Element\n",
        "Box(self) has\n",
        "Thing has value own{arena} Int\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}

#[test]
fn every_utf8_edit_prefix_has_navigable_type_syntax() {
    let source = "\
--- hé 🦀
Choice is First or Second
Box(element) has
    value element
    read(self) element:
        return self.value
";
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).expect("bounded edit");
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            for ancestor in hit.ancestors {
                assert!(ancestor.span.start() <= hit.node.span.start());
                assert!(hit.node.span.end() <= ancestor.span.end());
            }
        }
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
    }
}

#[test]
fn unsupported_member_forms_keep_following_members_and_top_level_items() {
    let source = "\
Thing has
    qualified.value Int
    other.value Int
    next Int
last := 2
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(members(&output.file, 0).len(), 3);
    assert_eq!(members(&output.file, 0)[0].value, TypeMember::Error);
    assert_eq!(members(&output.file, 0)[1].value, TypeMember::Error);
    let TypeMember::Binding(next) = &members(&output.file, 0)[2].value else {
        panic!("next member survives");
    };
    assert_eq!(next.name.value, "next");
    let outline: Vec<_> = output
        .file
        .declaration_outline()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(outline, ["Thing", "last"]);
}

#[test]
fn has_keyword_and_rejected_member_have_syntax_views() {
    let source = "Thing has\n    @bad\n";
    let output = parser::parse(source).unwrap();
    let keyword = output
        .file
        .node_at_offset(source.find("has").unwrap())
        .unwrap();
    assert!(matches!(keyword.node.kind, SyntaxNodeKind::TypeBody(_)));
    let error = output
        .file
        .node_at_offset(source.find("@bad").unwrap())
        .unwrap();
    assert!(matches!(
        error.node.kind,
        SyntaxNodeKind::TypeMember(member) if member.value == TypeMember::Error
    ));
    assert_eq!(error.declarations[0].name, "Thing");
}
