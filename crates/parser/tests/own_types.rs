use ast::{
    Annotation, Binding, BindingConvention, Convention, Expr, Expression, Item, PrefixedType,
    SourceFile, SyntaxNodeKind, TypeBody, TypeMember, TypePrefixKind,
};

#[test]
fn own_is_a_type_prefix_in_aliases_without_initialization_inference() {
    let source = "StorageOf(element) is own element\n";
    let file = parser::parse_strict(source).expect("a written owning-storage type");
    assert_eq!(file.text.as_ref(), source);
    let names: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(names, ["StorageOf"]);
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("type alias");
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("alias body");
    };
    let owned = type_prefix(expression);
    assert_eq!(owned.keyword.value, TypePrefixKind::Own);
    assert!(owned.group.is_none());
    assert_eq!(owned.target.value, Expr::Name("element".into()));
    assert_eq!(
        file.span_table.get(expression.span_id).extract(source),
        "own element"
    );
}

fn expression_annotation(binding: &Binding) -> &Expression {
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("written expression annotation");
    };
    expression
}

fn type_prefix(expression: &Expression) -> &PrefixedType {
    let Expr::PrefixedType(prefix) = &expression.value else {
        panic!("written prefix");
    };
    prefix
}

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("binding");
    };
    binding
}

#[test]
fn fields_and_storage_types_keep_own_separate_from_header_conventions() {
    let source = concat!(
        "Allocation(element) has\n    address own element\n",
        "take(value own Item, storage own own Item, view ref own Item) ",
        "own own Item: return storage\n",
        "local own own Item: storage\n",
        "Generic is Box(own Item)\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(allocation) = &file.items[0].value else {
        panic!("allocation");
    };
    let TypeBody::Has(fields) = &allocation.body.value else {
        panic!("fields");
    };
    let TypeMember::Binding(address) = &fields[0].value else {
        panic!("address");
    };
    assert_eq!(
        type_prefix(expression_annotation(address)).keyword.value,
        TypePrefixKind::Own
    );
    let function = binding(&file, 1);
    assert!(address.convention.is_none());
    assert_eq!(
        function.convention.as_ref().unwrap().value,
        BindingConvention::Own
    );
    assert_eq!(
        binding(&file, 2).convention.as_ref().unwrap().value,
        BindingConvention::Own
    );
    let parameters = &function.parameters.as_ref().unwrap().value;
    let ordinary = &parameters[0].value;
    assert_eq!(
        ordinary.convention.as_ref().unwrap().value.keyword.value,
        Convention::Own
    );
    let Annotation::Expression(value_type) = &ordinary.annotation.as_ref().unwrap().value else {
        panic!("ordinary value type");
    };
    assert_eq!(value_type.value, Expr::Name("Item".into()));
    for parameter in &parameters[1..] {
        let Annotation::Expression(expression) =
            &parameter.value.annotation.as_ref().unwrap().value
        else {
            panic!("owning handle type");
        };
        assert_eq!(type_prefix(expression).keyword.value, TypePrefixKind::Own);
    }
    assert_eq!(
        type_prefix(expression_annotation(function)).keyword.value,
        TypePrefixKind::Own
    );
    assert_eq!(
        type_prefix(expression_annotation(binding(&file, 2)))
            .keyword
            .value,
        TypePrefixKind::Own
    );
    check_field_and_parameter_queries(&file, source);
}

fn check_field_and_parameter_queries(file: &SourceFile, source: &str) {
    let hit = file
        .node_at_offset(source.find("own element").unwrap())
        .unwrap();
    assert!(
        matches!(hit.node.kind, SyntaxNodeKind::TypePrefixKeyword(keyword)
        if keyword.value == TypePrefixKind::Own)
    );
    let hit = file
        .node_at_offset(source.find("own Item").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Convention(keyword)
        if keyword.value == Convention::Own));
}

#[test]
fn nested_own_ref_and_mut_layers_do_not_collapse_or_move_their_groups() {
    let source = concat!(
        "Nested is own ref{loan} mut Item\n",
        "Double is own own Item\n",
        "Arguments is Container(own Item, slot: ref own Item)\n",
    );
    let file = parser::parse_strict(source).unwrap();
    for (index, expected) in [
        vec![
            TypePrefixKind::Own,
            TypePrefixKind::Ref,
            TypePrefixKind::Mut,
        ],
        vec![TypePrefixKind::Own, TypePrefixKind::Own],
    ]
    .iter()
    .enumerate()
    {
        let Item::Type(declaration) = &file.items[index].value else {
            panic!("alias");
        };
        let TypeBody::Is(expression) = &declaration.body.value else {
            panic!("is");
        };
        let mut expression = expression;
        let mut actual = Vec::new();
        for _ in 0..expected.len() {
            let prefix = type_prefix(expression);
            actual.push(prefix.keyword.value);
            if prefix.keyword.value == TypePrefixKind::Ref {
                assert_eq!(
                    prefix.group.as_ref().unwrap().value.components[0].value,
                    "loan"
                );
            } else {
                assert!(prefix.group.is_none());
            }
            expression = &prefix.target;
        }
        assert_eq!(&actual, expected);
        assert_eq!(expression.value, Expr::Name("Item".into()));
    }
    let Item::Type(declaration) = &file.items[2].value else {
        panic!("alias");
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("is");
    };
    let Expr::Call { arguments, .. } = &expression.value else {
        panic!("type call");
    };
    assert_eq!(arguments.value.len(), 2);
}

#[test]
fn incomplete_own_types_keep_missing_targets_and_declaration_owners() {
    for (source, owner) in [
        ("Owner is own", "Owner"),
        ("make() own own", "make"),
        ("Allocation(element) has address own", "address"),
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(
                hit.node.kind,
                SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
            ),
            "{source}: {hit:?}"
        );
        assert_eq!(hit.node.span.start(), source.len());
        assert_eq!(hit.declarations.last().unwrap().name, owner);
    }
    let source = "take(storage own own, next Item): return next\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let function = binding(&output.file, 0);
    let parameters = &function.parameters.as_ref().unwrap().value;
    assert_eq!(parameters.len(), 2);
    let Annotation::Expression(expression) =
        &parameters[0].value.annotation.as_ref().unwrap().value
    else {
        panic!("own prefix");
    };
    assert_eq!(type_prefix(expression).keyword.value, TypePrefixKind::Own);
    assert_eq!(type_prefix(expression).target.value, Expr::Missing);
    assert_eq!(parameters[1].value.name.value, "next");
    assert_eq!(binding(&output.file, 1).name.value, "later");
}

#[test]
fn owning_prefix_groups_diagnose_but_retain_source_structure() {
    for header in [
        "Owner is own{origin} Item",
        "make() own own{origin} Item",
        "Container has address own{origin} Item",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert!(parser::parse_strict(&source).is_err());
        let hit = output
            .file
            .node_at_offset(source.rfind("own").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::TypePrefixKeyword(keyword)
            if keyword.value == TypePrefixKind::Own)
        );
        let prefix = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::Expression(expression) => match &expression.value {
                    Expr::PrefixedType(prefix) => Some(prefix),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(prefix.target.value, Expr::Name("Item".into()));
        let group = prefix.group.as_ref().unwrap();
        assert_eq!(group.value.components[0].value, "origin");
        assert!(group.value.close.is_some());
        let later = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(later.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn own_does_not_expand_value_predicate_or_domain_syntax() {
    for rejected in [
        "value := own Item",
        "value := call(own Item)",
        "value is own Item: 1",
        "value Item and own Item: 1",
        "value in own Item: 1",
        "Bad is in own Item",
        "Bad is own Item to limit",
        "Bad is Box(own Item) to limit",
        "Bad is 0 to own Item",
        "Container has address own in Byte",
        "Container has address own is >= 0",
        "Container has address own is in Byte",
    ] {
        let source = format!("{rejected}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}: {hit:?}"
        );
    }
}

#[test]
fn owning_type_parameters_and_local_declarations_keep_their_layout() {
    let source = concat!(
        "factory(\n    value own own Item\n    observer ref own Item\n) own own Item:\n",
        "    local own own Item: value\n    return local\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 0);
    assert_eq!(function.parameters.as_ref().unwrap().value.len(), 2);
    assert_eq!(
        type_prefix(expression_annotation(function)).keyword.value,
        TypePrefixKind::Own
    );
    assert_eq!(binding(&file, 1).name.value, "later");
    let source = "broken := f(\nnext own Item: value\nlast := 2\n)\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let names: Vec<_> = output
        .file
        .declaration_outline()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["broken", "next", "last"]);
}
