use ast::{Expr, SyntaxNodeKind};

#[test]
fn cursor_on_a_member_name_reports_call_and_declaration_context() {
    let source = "\
--- café 🦀
find_user(id UserId) User:
    return users.lookup(id)

main:
    result := find_user(";
    let output = parser::parse(source).expect("incomplete source is recoverable");
    let snapshot = output.file.clone();
    let offset_bytes = source.find("lookup").unwrap() + 2;
    let hit = output
        .file
        .node_at_offset(offset_bytes)
        .expect("valid byte cursor");
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("lookup")));
    assert_eq!(hit.node.span.extract(source), "lookup");
    assert!(hit.ancestors.iter().any(|node| {
        matches!(node.kind, SyntaxNodeKind::Expression(expression) if
            matches!(expression.value, Expr::Call { .. }))
    }));
    assert!(hit.ancestors.iter().any(|node| {
        matches!(node.kind, SyntaxNodeKind::Expression(expression) if
            matches!(expression.value, Expr::Member { .. }))
    }));
    let declarations: Vec<_> = hit
        .declarations
        .iter()
        .map(|declaration| declaration.name)
        .collect();
    assert_eq!(declarations, ["find_user"]);
    assert_eq!(output.file, snapshot, "queries must not mutate syntax");
}

#[test]
fn eof_in_an_unfinished_call_keeps_argument_and_outer_declaration_context() {
    for source in [
        "main:\n    result := find_user(",
        "main:\n    result := find_user(\n",
        "main:\n    result := find_user(\n        ",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let hit = output
            .file
            .node_at_offset(source.len())
            .expect("EOF is a valid cursor");
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::ArgumentList(_)),
            "{source:?}: {hit:?}"
        );
        let declarations: Vec<_> = hit
            .declarations
            .iter()
            .map(|declaration| declaration.name)
            .collect();
        assert_eq!(declarations, ["main", "result"]);
        assert_eq!(hit.node.span.end(), source.len());
    }
}

#[test]
fn byte_offsets_do_not_split_unicode_scalars_or_accept_out_of_bounds_positions() {
    let source = "--- café 🦀\nvalue := original\n";
    let output = parser::parse(source).unwrap();
    for scalar in ["é", "🦀"] {
        let start = source.find(scalar).unwrap();
        for offset in start + 1..start + scalar.len() {
            assert!(output.file.node_at_offset(offset).is_none());
        }
    }
    let offset_bytes = source.find("original").unwrap();
    assert_ne!(source[..offset_bytes].encode_utf16().count(), offset_bytes);
    let hit = output.file.node_at_offset(offset_bytes).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("original")));
    assert!(output.file.node_at_offset(source.len() + 1).is_none());
    assert!(output.file.node_at_offset(usize::MAX).is_none());
}

#[test]
fn punctuation_boundaries_select_the_right_syntax_not_the_name_on_the_left() {
    let source = "left+right\nbase.member\nf(x)\n";
    let output = parser::parse_strict(source).unwrap();
    let plus = output.node_at_offset(source.find('+').unwrap()).unwrap();
    let SyntaxNodeKind::BinaryOperator(operator) = plus.node.kind else {
        panic!("expected written operator");
    };
    assert_eq!(operator.value, ast::BinaryOperator::Add);
    let right = output
        .node_at_offset(source.find("right").unwrap())
        .unwrap();
    assert!(matches!(right.node.kind, SyntaxNodeKind::Name("right")));
    let dot = output.node_at_offset(source.find('.').unwrap()).unwrap();
    assert!(matches!(
        dot.node.kind,
        SyntaxNodeKind::Expression(expression) if matches!(expression.value, Expr::Member { .. })
    ));
    let member = output
        .node_at_offset(source.find("member").unwrap())
        .unwrap();
    assert!(matches!(member.node.kind, SyntaxNodeKind::Name("member")));
    let arguments = output.node_at_offset(source.find("(x)").unwrap()).unwrap();
    assert!(matches!(
        arguments.node.kind,
        SyntaxNodeKind::ArgumentList(_)
    ));
}

#[test]
fn end_touch_context_is_local_to_whitespace_or_eof() {
    let source = "name  \n";
    let file = parser::parse_strict(source).unwrap();
    let end_of_name = file.node_at_offset(4).unwrap();
    assert!(matches!(
        end_of_name.node.kind,
        SyntaxNodeKind::Name("name")
    ));
    for offset in [5, 6, 7] {
        let gap = file.node_at_offset(offset).unwrap();
        assert!(matches!(gap.node.kind, SyntaxNodeKind::File));
        assert!(gap.declarations.is_empty());
    }
    for source in ["", " \n\t"] {
        let file = parser::parse(source).unwrap().file;
        for offset in 0..=source.len() {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::File));
        }
    }
}

#[test]
fn zero_width_missing_expressions_win_even_at_an_ancestors_exclusive_end() {
    for source in ["result := f(right: )", "result := f(right:"] {
        let output = parser::parse(source).unwrap();
        let offset = source.find(')').unwrap_or(source.len());
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
        ));
        assert_eq!(hit.node.span.to_range(), offset..offset);
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::Argument(_)))
        );
        assert_eq!(hit.declarations.last().unwrap().name, "result");
    }
}

#[test]
fn incomplete_member_queries_keep_the_receiver_without_fabricating_a_name() {
    let source = "value := receiver.";
    let output = parser::parse(source).unwrap();
    let hit = output.file.node_at_offset(source.len()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::Expression(expression) if
            matches!(expression.value, Expr::IncompleteMember { .. })
    ));
    assert_eq!(hit.node.span.extract(source), "receiver.");
    assert_eq!(hit.declarations.last().unwrap().name, "value");
}

#[test]
fn parameter_and_named_argument_names_have_different_structural_context() {
    let source = "\
call(left Box(Int), right: 2) Int:
    return use_value(left, label: right)
";
    let file = parser::parse_strict(source).unwrap();
    let parameter = file.node_at_offset(source.find("left").unwrap()).unwrap();
    assert!(matches!(parameter.node.kind, SyntaxNodeKind::Name("left")));
    assert!(
        parameter
            .ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Parameter(_)))
    );
    let annotation = file.node_at_offset(source.find("Int").unwrap()).unwrap();
    assert!(matches!(annotation.node.kind, SyntaxNodeKind::Name("Int")));
    assert!(
        annotation
            .ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Parameter(_)))
    );
    let argument = file.node_at_offset(source.find("label").unwrap()).unwrap();
    assert!(matches!(argument.node.kind, SyntaxNodeKind::Name("label")));
    assert!(
        argument
            .ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Argument(_)))
    );
    assert_eq!(argument.declarations.len(), 1);
    assert_eq!(argument.declarations[0].name, "call");
}

#[test]
fn comments_and_rejected_items_do_not_fabricate_name_nodes() {
    let source =
        "-- module words\n#unsupported core\nmain:\n    -- inside words\n    return item\n";
    let output = parser::parse(source).unwrap();
    let comment = output
        .file
        .node_at_offset(source.find("words").unwrap())
        .unwrap();
    assert!(matches!(comment.node.kind, SyntaxNodeKind::File));
    let rejected = output
        .file
        .node_at_offset(source.find("core").unwrap())
        .unwrap();
    assert!(matches!(
        rejected.node.kind, SyntaxNodeKind::Item(item) if item.value == ast::Item::Error
    ));
    let body_comment = output
        .file
        .node_at_offset(source.find("inside").unwrap())
        .unwrap();
    assert!(!matches!(body_comment.node.kind, SyntaxNodeKind::Name(_)));
    assert_eq!(body_comment.declarations[0].name, "main");
}

#[test]
fn a_callee_name_is_not_resolved_to_a_same_named_outline_declaration() {
    let source = "lookup() Int: return 1\nmain:\n    result := lookup()\n";
    let file = parser::parse_strict(source).unwrap();
    let call_offset = source.rfind("lookup").unwrap();
    let hit = file.node_at_offset(call_offset).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("lookup")));
    let names: Vec<_> = hit
        .declarations
        .iter()
        .map(|declaration| declaration.name)
        .collect();
    assert_eq!(names, ["main", "result"]);
    assert_ne!(
        hit.node.span,
        file.declaration_outline().next().unwrap().name_span
    );
}

#[test]
fn every_utf8_edit_prefix_can_be_queried_at_each_valid_insertion_point() {
    let source = "--- café 🦀\nmain:\n    result := f(right: receiver.member)\n    return result\n";
    for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
        let prefix = &source[..end];
        let file = parser::parse(prefix).unwrap().file;
        for offset in (0..=prefix.len()).filter(|offset| prefix.is_char_boundary(*offset)) {
            let hit = file.node_at_offset(offset).expect("valid insertion point");
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            assert!(hit.node.span.start() <= offset);
            assert!(offset <= hit.node.span.end());
            let mut child = hit.node.span;
            for parent in hit.ancestors.iter().rev() {
                assert!(parent.span.start() <= child.start());
                assert!(child.end() <= parent.span.end());
                child = parent.span;
            }
        }
    }
}

#[test]
fn deeply_grouped_lookup_is_iterative_and_debug_does_not_dump_the_ast() {
    let source = format!("value := {}name{}", "(".repeat(100), ")".repeat(100));
    let file = parser::parse_strict(&source).unwrap();
    let hit = file.node_at_offset(source.find("name").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("name")));
    assert!(hit.ancestors.len() > 100);
    assert!(format!("{hit:?}").len() < 32 * 1024);
}

#[test]
fn declaration_operators_and_bare_return_keywords_are_source_nodes() {
    let source = "main:\n    value := 1\n    return\n";
    let file = parser::parse_strict(source).unwrap();
    let operator = file.node_at_offset(source.find(":=").unwrap()).unwrap();
    assert!(matches!(
        operator.node.kind,
        SyntaxNodeKind::BindingOperator(operator) if
            operator.value == ast::BindingOperator::ColonEquals
    ));
    let returned = file.node_at_offset(source.find("return").unwrap()).unwrap();
    assert!(matches!(
        returned.node.kind,
        SyntaxNodeKind::Statement(statement) if
            matches!(&statement.value, ast::Statement::Return(returned)
                if returned.value.is_none())
    ));
    assert_eq!(returned.declarations.last().unwrap().name, "main");
}
