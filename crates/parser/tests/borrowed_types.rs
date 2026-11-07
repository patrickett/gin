use ast::{
    Annotation, BinaryOperator, Convention, Expr, Item, SyntaxNodeKind, TypeBody, TypePrefixKind,
};

#[test]
fn borrowed_alias_is_source_syntax_without_resolution_or_lowering() {
    let source = "RefItem is ref Item\n";
    let file = parser::parse_strict(source).expect("a written borrowed type alias");
    assert_eq!(file.text.as_ref(), source);
    let names: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(names, ["RefItem"]);
}

#[test]
fn convention_and_type_groups_remain_distinct_written_roles() {
    let source = "inspect(value ref{slot} mut{target.items} Item) ref{result} Item: return value\n";
    let file = parser::parse_strict(source).expect("grouped convention and borrowed types");
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("written function");
    };
    let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
    let convention = parameter.convention.as_ref().unwrap();
    assert_eq!(convention.value.keyword.value, Convention::Ref);
    assert_eq!(
        file.span_table.get(convention.span_id).extract(source),
        "ref{slot}"
    );
    assert_eq!(
        convention.value.group.as_ref().unwrap().value.components[0].value,
        "slot"
    );
    let Annotation::Expression(expression) = &parameter.annotation.as_ref().unwrap().value else {
        panic!("written parameter type");
    };
    let Expr::PrefixedType(borrowed) = &expression.value else {
        panic!("type constructor, not another convention");
    };
    assert_eq!(borrowed.keyword.value, TypePrefixKind::Mut);
    let components: Vec<_> = borrowed
        .group
        .as_ref()
        .unwrap()
        .value
        .components
        .iter()
        .map(|component| component.value.as_str())
        .collect();
    assert_eq!(components, ["target", "items"]);
    assert_eq!(borrowed.target.value, Expr::Name("Item".into()));
    let convention_hit = file
        .node_at_offset(source.find("ref{slot}").unwrap())
        .unwrap();
    assert!(matches!(
        convention_hit.node.kind,
        SyntaxNodeKind::Convention(_)
    ));
    let type_hit = file
        .node_at_offset(source.find("mut{target").unwrap())
        .unwrap();
    assert!(matches!(
        type_hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(_)
    ));
}

#[test]
fn borrowed_prefixes_do_not_expand_value_predicate_or_numeric_domain_grammar() {
    for rejected in [
        // `ref place` and `mut place` are value expressions (they create a borrow), so
        // `value := ref Item` is not rejected; the predicate and domain grammars still are.
        "value Int and ref Item: 1",
        "value is ref Item: 1",
        "value in ref Item: 1",
        "T is in ref Item",
        "T is 0 to mut Item",
        "T is ref Item to limit",
        "T is Box(ref Item) to limit",
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
            "{source}"
        );
    }
}

#[test]
fn orphan_closes_do_not_hide_borrowed_fields_after_unfinished_value_calls() {
    for keyword in ["ref", "mut"] {
        let source = format!(
            "Thing has\n    broken := f(\n    field {keyword}{{arena}} Item\n\
             )\n    next Int\nlater := 1\n"
        );
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        let offset = source.find("field").unwrap();
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("field")),
            "{source}: {hit:?}"
        );
        let owners: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
        assert_eq!(owners, ["Thing", "field"]);
        let later = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(later.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn type_prefixes_nest_without_collapsing_and_follow_postfix_precedence() {
    let source = concat!(
        "Nested is ref{outer} mut{inner} Box(ref{argument} Item).Member\n",
        "Union is ref First or mut Second\n",
        "Grouped is ref (First or mut Second)\n",
        "Thing has field own ref Item, method(self ref, value own mut Item) ref mut Item\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("type");
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("alias");
    };
    let Expr::PrefixedType(first) = &expression.value else {
        panic!("outer");
    };
    let Expr::PrefixedType(second) = &first.target.value else {
        panic!("inner");
    };
    assert_eq!(first.keyword.value, TypePrefixKind::Ref);
    assert_eq!(second.keyword.value, TypePrefixKind::Mut);
    assert!(matches!(second.target.value, Expr::Member { .. }));
    let Item::Type(declaration) = &file.items[1].value else {
        panic!("type");
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("alias");
    };
    let Expr::Binary {
        left,
        operator,
        right,
    } = &expression.value
    else {
        panic!("union");
    };
    assert_eq!(operator.value, BinaryOperator::Or);
    assert!(matches!(left.value, Expr::PrefixedType(_)));
    assert!(matches!(right.value, Expr::PrefixedType(_)));
    let Item::Type(declaration) = &file.items[2].value else {
        panic!("type");
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("alias");
    };
    let Expr::PrefixedType(outer) = &expression.value else {
        panic!("borrow");
    };
    assert!(matches!(outer.target.value, Expr::Group(_)));
}

#[test]
fn group_paths_preserve_spacing_and_missing_parts_without_resolving_them() {
    let source = "f(x ref { arena . Items . values } mut { other } Item) ref{result} Item\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function");
    };
    let convention = binding.parameters.as_ref().unwrap().value[0]
        .value
        .convention
        .as_ref()
        .unwrap();
    let group = convention.value.group.as_ref().unwrap();
    assert_eq!(
        file.span_table.get(group.span_id).extract(source),
        "{ arena . Items . values }"
    );
    assert_eq!(group.value.components.len(), 3);
    assert!(group.value.close.is_some());
    for bad in [
        "f(x ref{} Item): return x",
        "f(x ref{Root} Item): return x",
        "f(x ref{self} Item): return x",
        "f(x ref{root.Self} Item): return x",
        "f(x ref{root..part} Item): return x",
        "f(x ref{root.} Item): return x",
        "f(x own{root} Item): return x",
        "Alias is ref{root\n    .part} Item",
        "Alias is ref @{root} Item",
        "Alias is own{root} Item",
    ] {
        let source = format!("{bad}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
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
fn missing_group_closes_stop_at_parameter_and_header_boundaries() {
    for source in [
        "f(x ref{group, next Item): return next\nlater := 2\n",
        "f(x ref{group): return x\nlater := 2\n",
        "f() ref{group: return 1\nlater := 2\n",
        "Alias is ref{group\nNext has field Item\nlater := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}: {hit:?}"
        );
        assert!(parser::parse_strict(source).is_err());
    }
    for source in ["Alias is ref", "Alias is mut{group}", "f() ref{group}"] {
        let output = parser::parse(source).unwrap();
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(
                hit.node.kind,
                SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
            ),
            "{source}: {hit:?}"
        );
        assert_eq!(hit.node.span.start(), source.len());
        assert_eq!(hit.node.span.end(), source.len());
    }
}

#[test]
fn reserved_group_words_are_retained_as_invalid_components_not_reinterpreted() {
    let mut paths = vec!["if".to_owned()];
    for keyword in lexer::TokenKind::KEYWORDS {
        paths.push(format!("g.{}", keyword.keyword().unwrap()));
    }
    for path in paths {
        let source = format!("Alias is ref{{{path}}} Item\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        let Item::Type(declaration) = &output.file.items[0].value else {
            panic!("malformed group must not destroy the declaration: {source}");
        };
        let TypeBody::Is(expression) = &declaration.body.value else {
            panic!("alias");
        };
        let Expr::PrefixedType(borrowed) = &expression.value else {
            panic!("borrow");
        };
        let group = borrowed.group.as_ref().unwrap();
        assert!(group.value.close.is_some());
        let components: Vec<_> = group
            .value
            .components
            .iter()
            .map(|component| component.value.as_str())
            .collect();
        assert_eq!(components, path.split('.').collect::<Vec<_>>());
        assert_eq!(borrowed.target.value, Expr::Name("Item".into()));
        let word = group.value.components.last().unwrap();
        let offset = output.file.span_table.get(word.span_id).start();
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(_)), "{hit:?}");
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::GroupPath(_)))
        );
    }
    let output = parser::parse("f(x ref{g.if}) Item: return x\n").unwrap();
    assert!(output.has_errors());
    assert!(matches!(output.file.items[0].value, Item::Binding(_)));
}
