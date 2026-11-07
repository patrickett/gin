use ast::{
    Annotation, Binding, BindingConvention, Convention, Expr, Expression, Item, SourceFile,
    SyntaxNodeKind, TypeBody, TypeMember, TypePrefixKind,
};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("expected binding");
    };
    binding
}

fn annotation(binding: &Binding) -> &Expression {
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("expected expression annotation");
    };
    expression
}

fn assert_header(binding: &Binding, own: bool, storage: bool) {
    assert_eq!(
        binding.convention.as_ref().map(|keyword| keyword.value),
        own.then_some(BindingConvention::Own)
    );
    let expression = annotation(binding);
    if storage {
        let Expr::PrefixedType(prefix) = &expression.value else {
            panic!("expected storage prefix");
        };
        assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
        assert_eq!(prefix.target.value, Expr::Name("Item".into()));
    } else {
        assert_eq!(expression.value, Expr::Name("Item".into()));
    }
}

#[test]
fn first_own_selects_permission_only_in_ordinary_and_method_headers() {
    let source = concat!(
        "plain Item: value\nowned own Item: value\nstorage own own Item: value\n",
        "function() own Item: return value\n",
        "Thing has\n    field own Item\n    method() own own Item: return field\n",
        "Owner is own Item\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_header(binding(&file, 0), false, false);
    assert_header(binding(&file, 1), true, false);
    assert_header(binding(&file, 2), true, true);
    assert_header(binding(&file, 3), true, false);
    let Item::Type(thing) = &file.items[4].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &thing.body.value else {
        panic!("members")
    };
    for (index, own) in [false, true].into_iter().enumerate() {
        let TypeMember::Binding(member) = &members[index].value else {
            panic!("member")
        };
        assert_header(member, own, true);
        assert_eq!(member.parameters.is_some(), own);
    }
    let Item::Type(owner) = &file.items[5].value else {
        panic!("alias")
    };
    let TypeBody::Is(expression) = &owner.body.value else {
        panic!("alias expression")
    };
    assert!(matches!(expression.value, Expr::PrefixedType(_)));
}

#[test]
fn qualifier_only_headers_do_not_fabricate_annotations() {
    for source in ["x own", "f() own", "x own: value", "f() own: return value"] {
        let file = parser::parse_strict(source).expect(source);
        let binding = binding(&file, 0);
        assert_eq!(
            binding.convention.as_ref().unwrap().value,
            BindingConvention::Own
        );
        assert!(binding.annotation.is_none(), "{source}");
    }
}

#[test]
fn parameter_and_result_permissions_are_independently_written() {
    for (parameter, result) in [(false, false), (true, false), (false, true), (true, true)] {
        let parameter_text = if parameter { "own " } else { "" };
        let result_text = if result { "own " } else { "" };
        let source = format!("forward(tx {parameter_text}Item) {result_text}Item: return tx\n");
        let file = parser::parse_strict(&source).unwrap();
        let function = binding(&file, 0);
        assert_header(function, result, false);
        let parameters = &function.parameters.as_ref().unwrap().value;
        assert_eq!(
            parameters[0]
                .value
                .convention
                .as_ref()
                .map(|c| c.value.keyword.value),
            parameter.then_some(Convention::Own)
        );
    }
}

#[test]
fn local_and_result_permissions_are_independently_written() {
    for (local, result) in [(false, false), (true, false), (false, true), (true, true)] {
        let local_text = if local { "own " } else { "" };
        let result_text = if result { "own " } else { "" };
        let source = format!(
            "make_transaction {result_text}Item:\n    tx {local_text}Item: value\n    return tx\n"
        );
        let file = parser::parse_strict(&source).unwrap();
        assert_header(binding(&file, 0), result, false);
        let offset = source.find("tx ").unwrap();
        let hit = file.node_at_offset(offset).unwrap();
        let local_binding = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::Binding(binding) if binding.name.value == "tx" => Some(binding),
                _ => None,
            })
            .expect("local declaration");
        assert_header(local_binding, local, false);
    }
}

#[test]
fn aliases_never_supply_unwritten_permissions() {
    let source = "Owner is own Item\nforward(tx Owner) Owner: return tx\n";
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 1);
    assert!(function.convention.is_none());
    assert!(
        function.parameters.as_ref().unwrap().value[0]
            .value
            .convention
            .is_none()
    );
    assert_eq!(annotation(function).value, Expr::Name("Owner".into()));
}

#[test]
fn convention_queries_have_exact_keyword_spans_and_declaration_owners() {
    let source = "--- hé 🦀\nThing has method() own own Item: return value\n";
    let file = parser::parse_strict(source).unwrap();
    let offset = source.find("own").unwrap();
    for position in offset..offset + 3 {
        let hit = file.node_at_offset(position).unwrap();
        let SyntaxNodeKind::BindingConvention(keyword) = hit.node.kind else {
            panic!("expected binding convention: {hit:?}");
        };
        assert_eq!(keyword.value, BindingConvention::Own);
        assert_eq!(hit.node.span.to_range(), offset..offset + 3);
        assert_eq!(file.span_table.get(keyword.span_id).extract(source), "own");
        let owners: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
        assert_eq!(owners, ["Thing", "method"]);
        assert!(format!("{hit:?}").len() < 16_384);
    }
    let hit = file.node_at_offset(offset + 4).unwrap();
    assert!(
        matches!(hit.node.kind, SyntaxNodeKind::TypePrefixKeyword(keyword)
        if keyword.value == TypePrefixKind::Own)
    );
}

#[test]
fn ref_mut_and_nested_storage_annotations_remain_type_prefixes() {
    for (header, kinds) in [
        ("x ref Item", vec![TypePrefixKind::Ref]),
        ("f() mut Item", vec![TypePrefixKind::Mut]),
        (
            "x own own ref mut Item",
            vec![
                TypePrefixKind::Own,
                TypePrefixKind::Ref,
                TypePrefixKind::Mut,
            ],
        ),
    ] {
        let file = parser::parse_strict(header).unwrap();
        let binding = binding(&file, 0);
        assert_eq!(binding.convention.is_some(), header.contains("own"));
        let mut expression = annotation(binding);
        for kind in kinds {
            let Expr::PrefixedType(prefix) = &expression.value else {
                panic!("prefix")
            };
            assert_eq!(prefix.keyword.value, kind);
            expression = &prefix.target;
        }
        assert_eq!(expression.value, Expr::Name("Item".into()));
    }
}

#[test]
fn missing_second_own_targets_preserve_headers_and_following_items() {
    for header in ["x own own", "f() own own", "Thing has f() own own"] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("own own").unwrap())
            .unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::BindingConvention(_)
        ));
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
    for source in ["x own own", "f() own own"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Expression(expression)
            if expression.value == Expr::Missing)
        );
        assert_eq!(hit.node.span.start(), source.len());
    }
}

#[test]
fn malformed_binding_convention_groups_do_not_become_storage_prefixes() {
    for header in [
        "x own{g} Item",
        "f() own{g} Item",
        "Thing has f() own{g} Item",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err());
        let hit = output
            .file
            .node_at_offset(source.find("own").unwrap())
            .unwrap();
        // Unsupported header attachments reject the header, not a fabricated type layer.
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Item(item)
            if matches!(item.value, Item::Error)));
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn method_delimiters_retain_missing_storage_targets_and_next_members() {
    let source = "Thing has method() own own, field own Item\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let Item::Type(thing) = &output.file.items[0].value else {
        panic!("type");
    };
    let TypeBody::Has(members) = &thing.body.value else {
        panic!("members");
    };
    assert_eq!(members.len(), 2);
    let TypeMember::Binding(method) = &members[0].value else {
        panic!("method");
    };
    assert_eq!(
        method.convention.as_ref().unwrap().value,
        BindingConvention::Own
    );
    let Expr::PrefixedType(prefix) = &annotation(method).value else {
        panic!("storage type");
    };
    assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
    assert_eq!(prefix.target.value, Expr::Missing);
    let TypeMember::Binding(field) = &members[1].value else {
        panic!("field");
    };
    assert_header(field, false, true);
    assert_eq!(binding(&output.file, 1).name.value, "later");
}
