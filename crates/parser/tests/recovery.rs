use ast::{Argument, Expr, Item};

#[test]
fn failed_constructs_consume_their_unregistered_indentation_scope() {
    for rejected in [
        "Thing is\n    @bad\n    tail\n",
        "use math\n    add as\n    tail\n",
        "Thing has read():\n    @bad\n",
    ] {
        let source = format!("{rejected}next := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert_eq!(
            output.file.items.len(),
            2,
            "{rejected}: {:?}",
            output.file.items
        );
        assert_eq!(output.file.items[0].value, Item::Error);
        let Item::Binding(next) = &output.file.items[1].value else {
            panic!("next declaration survives");
        };
        assert_eq!(next.name.value, "next");
        let hit = output
            .file
            .node_at_offset(source.find("next").unwrap())
            .unwrap();
        assert!(matches!(
            hit.node.kind,
            ast::SyntaxNodeKind::Name("next")
        ));
        assert!(
            output
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message != "unexpected dedent")
        );
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn syntax_errors_return_a_tree_and_preserve_later_declarations() {
    let output = parser::parse("first := 1\nbroken := )\nlast := 2\n")
        .expect("ordinary syntax errors are recoverable");
    assert!(!output.diagnostics.is_empty());
    assert_eq!(output.file.items.len(), 3);
    let Item::Binding(first) = &output.file.items[0].value else {
        panic!("first declaration survives");
    };
    assert_eq!(first.name.value, "first");
    let Item::Binding(last) = &output.file.items[2].value else {
        panic!("later declaration survives");
    };
    assert_eq!(last.name.value, "last");
}

#[test]
fn unfinished_calls_retain_the_callee_and_missing_argument() {
    for source in ["value := f(", "value := f(right:", "value := f(1 +"] {
        let output = parser::parse(source).expect("incomplete calls are recoverable");
        assert!(output.has_errors());
        let Item::Binding(binding) = &output.file.items[0].value else {
            panic!("binding should survive: {source}");
        };
        let Expr::Call { callee, arguments } = &binding
            .value
            .as_ref()
            .expect("initializer")
            .expression
            .value
        else {
            panic!("call structure should survive: {source}");
        };
        assert_eq!(callee.value, Expr::Name("f".into()));
        if source.ends_with(':') {
            let Argument::Named { value, .. } = &arguments.value[0].value else {
                panic!("named argument survives");
            };
            assert_eq!(value.value, Expr::Missing);
            assert!(output.file.span_table.get(value.span_id).is_empty());
        }
    }
}

#[test]
fn missing_initializers_do_not_consume_the_next_declaration() {
    let output = parser::parse("first :=\nnext := 2\n").unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 2);
    let Item::Binding(first) = &output.file.items[0].value else {
        panic!("incomplete first binding survives");
    };
    assert_eq!(
        first.value.as_ref().unwrap().expression.value,
        Expr::Missing
    );
    let Item::Binding(next) = &output.file.items[1].value else {
        panic!("next binding survives");
    };
    assert_eq!(next.name.value, "next");
}

#[test]
fn unclosed_calls_do_not_pull_later_declarations_inside_a_body() {
    let source = "main:\n    value := f(\nnext := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 2);
    let Item::Binding(next) = &output.file.items[1].value else {
        panic!("next remains a top-level binding");
    };
    assert_eq!(next.name.value, "next");
}

#[test]
fn incomplete_member_access_retains_the_completion_receiver() {
    let output = parser::parse("value := receiver.\nnext := 2\n").unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 2);
    let Item::Binding(binding) = &output.file.items[0].value else {
        panic!("binding survives");
    };
    let Expr::IncompleteMember { base } = &binding.value.as_ref().unwrap().expression.value else {
        panic!("missing member name is explicit, not an invented identifier");
    };
    assert_eq!(base.value, Expr::Name("receiver".into()));
}

#[test]
fn unsupported_nested_constructs_keep_error_spans_and_later_scopes() {
    let source =
        "main:\n    when value is\n        Unsupported if guard then 1\n    return 2\nnext := 3\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 2);
    let Item::Binding(main) = &output.file.items[0].value else {
        panic!("main survives");
    };
    let Expr::Block(block) = &main.value.as_ref().unwrap().expression.value else {
        panic!("body survives");
    };
    assert_eq!(block.statements.len(), 2);
    assert_eq!(block.statements[0].value, ast::Statement::Error);
    let error_text = output
        .file
        .span_table
        .get(block.statements[0].span_id)
        .extract(source);
    assert_eq!(
        error_text,
        "when value is\n        Unsupported if guard then 1"
    );
    assert!(matches!(
        block.statements[1].value,
        ast::Statement::Return(_)
    ));
}

#[test]
fn lexical_errors_and_comments_do_not_prevent_later_source_queries() {
    let source = "--| module\nfirst := 1\r\nnext := 2\r\n";
    let output = parser::parse(source).unwrap();
    assert_eq!(output.file.items.len(), 2);
    assert_eq!(output.file.comments.len(), 1);
    assert_eq!(output.diagnostics.len(), 2);
    assert!(output.has_errors());
    assert!(output.into_result().is_err());
    assert!(parser::parse_strict(source).is_err());
}

#[test]
fn recovery_is_deterministic_and_complete_source_still_passes_the_strict_adapter() {
    let source = "broken := f(\nnext := 2\n";
    assert_eq!(
        parser::parse(source).unwrap(),
        parser::parse(source).unwrap()
    );
    let valid = "result := f(1, right: 2)\n";
    let output = parser::parse(valid).unwrap();
    assert!(!output.has_errors());
    assert_eq!(
        output.into_result().unwrap(),
        parser::parse_strict(valid).unwrap()
    );
}

#[test]
fn short_malformed_layout_sequences_terminate_with_a_snapshot() {
    let parts = [
        "x", ":=", "(", ")", "\n", "\n    ", "return", "[", "when", "private", "'hé'",
    ];
    for first in parts {
        for second in parts {
            for third in parts {
                let source = format!("{first} {second} {third}\n");
                let output = parser::parse(&source).expect("small input stays in budget");
                assert_eq!(output.file.text.as_ref(), source);
                for diagnostic in output.diagnostics {
                    assert!(
                        source.get(diagnostic.span.to_range()).is_some(),
                        "{diagnostic:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn malformed_signatures_do_not_consume_enclosing_body_dedents() {
    let source = "main:\n    broken(x Int\nnext := 2\n) Int: return 3\n";
    let output = parser::parse(source).expect("malformed signature remains recoverable");
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 3);
    let Item::Binding(next) = &output.file.items[1].value else {
        panic!("next declaration must stay outside main");
    };
    assert_eq!(next.name.value, "next");
}

#[test]
fn unexpected_indentation_does_not_duplicate_the_following_declaration_as_an_error() {
    let source = "    bad := 1\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert_eq!(output.file.items.len(), 2);
    assert_eq!(output.file.items[0].value, Item::Error);
    let Item::Binding(later) = &output.file.items[1].value else {
        panic!("later must be represented once, as a binding");
    };
    assert_eq!(later.name.value, "later");
    let end = output
        .file
        .span_table
        .get(output.file.items[0].span_id)
        .end();
    let start = output.file.span_table.get(later.name.span_id).start();
    assert!(end <= start);
}

#[test]
fn repeated_unmatched_calls_preserve_all_following_declarations() {
    let source: String = (0..4_000)
        .map(|index| format!("f(\nx{index} := 0\n"))
        .collect();
    let options = parser::ParseOptions {
        limits: parser::ParseLimits {
            diagnostics_max: 5_000,
            ..Default::default()
        },
        ..Default::default()
    };
    let output = parser::parse_with_options(&source, options).unwrap();
    assert_eq!(output.diagnostics.len(), 4_000);
    assert_eq!(output.file.items.len(), 8_000);
}
