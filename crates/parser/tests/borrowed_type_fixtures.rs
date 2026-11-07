//! Small source-only slices, not claims that the complete legacy fixtures parse.
use ast::{Annotation, Convention, Expr, Item, SyntaxNodeKind, TypeBody, TypePrefixKind};

const BOX_STORAGE: &str = include_str!("../../test-fixtures/fixtures/box_storage_core_box.gin");

#[test]
fn box_storage_fixture_preserves_ordinary_parameter_conventions() {
    for (line, expected) in [
        (
            "read(value ref Resource) Byte: return value.value",
            Convention::Ref,
        ),
        ("write(value mut Resource, next Byte):", Convention::Mut),
        (
            "move_box(value own Box(Resource)) Box(Resource): return value",
            Convention::Own,
        ),
    ] {
        // Select an exact existing line; do not parse or rewrite the full fixture.
        let source = BOX_STORAGE
            .lines()
            .find(|candidate| *candidate == line)
            .unwrap();
        // Keep write's original colon: removing it makes this look like a value call.
        // Its multiline body is omitted, so only that missing body is recovered.
        let output = parser::parse(source).expect(source);
        assert_eq!(output.has_errors(), source.ends_with(':'));
        if source.ends_with(':') {
            assert_eq!(output.diagnostics.len(), 1);
            assert_eq!(output.diagnostics[0].message, "expected an expression");
        }
        let file = output.file;
        let Item::Binding(binding) = &file.items[0].value else {
            panic!("expected fixture function");
        };
        let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
        let convention = parameter.convention.as_ref().unwrap();
        assert_eq!(convention.value.keyword.value, expected);
        assert!(convention.value.group.is_none());
        let keyword = file.span_table.get(convention.value.keyword.span_id);
        assert_eq!(
            keyword.extract(source),
            match expected {
                Convention::Ref => "ref",
                Convention::Mut => "mut",
                Convention::Own => "own",
                Convention::End => "end",
            }
        );
        assert_eq!(file.span_table.get(convention.span_id), keyword);
        for offset in keyword.start()..keyword.end() {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::Convention(_)));
            assert_eq!(hit.node.span, keyword);
        }
    }
}

#[test]
fn codegen_alias_snippets_preserve_written_borrowed_types() {
    // Exact alias lines from crates/codegen/tests/test_reference_pointer.rs.
    // Only these source-level aliases are tested, not pointer semantics or lowering.
    for (source, target) in [
        ("RefWord is ref Word\n", "Word"),
        ("RefAlias(x) is ref x\n", "x"),
    ] {
        let file = parser::parse_strict(source).expect(source);
        let Item::Type(declaration) = &file.items[0].value else {
            panic!("expected alias");
        };
        let TypeBody::Is(expression) = &declaration.body.value else {
            panic!("expected alias expression");
        };
        let Expr::PrefixedType(borrowed) = &expression.value else {
            panic!("expected written ref prefix");
        };
        assert_eq!(borrowed.keyword.value, TypePrefixKind::Ref);
        assert!(borrowed.group.is_none());
        assert!(matches!(&borrowed.target.value, Expr::Name(name) if name == target));
        assert_eq!(
            file.span_table.get(expression.span_id).extract(source),
            format!("ref {target}")
        );
        check_keyword_role(&file, source.find("ref").unwrap(), false);
    }
}

#[test]
fn ownership_grouped_header_preserves_distinct_keyword_roles() {
    // Exact source from crates/parser/tests/ownership_parser_tests.rs.
    let source = "borrow(value ref{r} Entity) ref{r} Entity: return value\n";
    let file = parser::parse_strict(source).expect(source);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected function");
    };
    let convention = binding.parameters.as_ref().unwrap().value[0]
        .value
        .convention
        .as_ref()
        .unwrap();
    assert_eq!(convention.value.keyword.value, Convention::Ref);
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("expected return annotation");
    };
    let Expr::PrefixedType(borrowed) = &expression.value else {
        panic!("expected borrowed return type");
    };
    assert_eq!(borrowed.keyword.value, TypePrefixKind::Ref);
    for group in [
        convention.value.group.as_ref().unwrap(),
        borrowed.group.as_ref().unwrap(),
    ] {
        assert_eq!(file.span_table.get(group.span_id).extract(source), "{r}");
        assert_eq!(group.value.components.len(), 1);
        assert_eq!(group.value.components[0].value, "r");
        assert_eq!(file.span_table.get(group.value.open).extract(source), "{");
        assert_eq!(
            file.span_table
                .get(group.value.close.unwrap())
                .extract(source),
            "}"
        );
        for delimiter in [group.value.open, group.value.close.unwrap()] {
            let span = file.span_table.get(delimiter);
            let hit = file.node_at_offset(span.start()).unwrap();
            assert_eq!(hit.node.span, span);
            assert!(matches!(hit.node.kind, SyntaxNodeKind::GroupDelimiter(_)));
            assert!(hit.ancestors.iter().any(|ancestor| {
                matches!(ancestor.kind, SyntaxNodeKind::GroupPath(_))
                    && ancestor.span == file.span_table.get(group.span_id)
            }));
        }
        let component = &group.value.components[0];
        let span = file.span_table.get(component.span_id);
        let hit = file.node_at_offset(span.start()).unwrap();
        assert_eq!(hit.node.span, span);
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("r")));
    }
    let offsets: Vec<_> = source
        .match_indices("ref")
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(offsets.len(), 2);
    check_keyword_role(&file, offsets[0], true);
    check_keyword_role(&file, offsets[1], false);
}

#[test]
fn header_only_wrapper_preserves_nested_borrowed_types() {
    // Deliberately new header-only wrapper, not an existing fixture program.
    let source = "nested(value ref mut Item) ref Box(mut Item)\n";
    let file = parser::parse_strict(source).expect(source);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected header");
    };
    let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
    assert_eq!(
        parameter.convention.as_ref().unwrap().value.keyword.value,
        Convention::Ref
    );
    let Annotation::Expression(expression) = &parameter.annotation.as_ref().unwrap().value else {
        panic!("expected parameter annotation");
    };
    assert!(matches!(&expression.value, Expr::PrefixedType(value)
            if value.keyword.value == TypePrefixKind::Mut));
    for (index, (offset, _)) in source.match_indices("ref").enumerate() {
        check_keyword_role(&file, offset, index == 0);
    }
    for (offset, _) in source.match_indices("mut") {
        check_keyword_role(&file, offset, false);
    }
}

fn check_keyword_role(file: &ast::SourceFile, start: usize, convention: bool) {
    for offset in start..start + 3 {
        let hit = file.node_at_offset(offset).unwrap();
        assert_eq!(hit.node.span.start(), start);
        assert_eq!(hit.node.span.end(), start + 3);
        if convention {
            assert!(matches!(hit.node.kind, SyntaxNodeKind::Convention(_)));
        } else {
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::TypePrefixKeyword(_)
            ));
        }
    }
}
