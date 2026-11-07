//! Hand-built source examples: these test the proposed shapes, not a parser.

use ast::{
    Annotation, Argument, BinaryOperator, Binding, BindingOperator, BindingValue, Block,
    CommentKind, Expr, Item, LiteralKind, Parameter, ReturnStatement, SourceFile, Spanned,
    Statement, TypeBody, TypeDeclaration,
};

fn file(text: &str) -> SourceFile {
    SourceFile {
        text: text.into(),
        span_table: Default::default(),
        items: Vec::new(),
        comments: Vec::new(),
    }
}

fn node<T>(file: &mut SourceFile, text: &str, occurrence: usize, value: T) -> Spanned<T> {
    assert!(!text.is_empty());
    let (start, _) = file
        .text
        .match_indices(text)
        .nth(occurrence)
        .expect("example contains this source occurrence");
    let end = start + text.len();
    assert!(end <= file.text.len());
    Spanned::with_range(&mut file.span_table, value, start..end)
}

fn text<'a, T>(file: &'a SourceFile, node: &Spanned<T>) -> &'a str {
    file.span_table.get(node.span_id).extract(&file.text)
}

#[test]
fn names_and_explicit_empty_calls_are_different_syntax() {
    let mut file = file("f\nf()\n");
    let name = node(&mut file, "f", 0, Expr::Name("f".into()));
    let callee = node(&mut file, "f", 1, Expr::Name("f".into()));
    let arguments = node(&mut file, "()", 0, Vec::new());
    let call = node(
        &mut file,
        "f()",
        0,
        Expr::Call {
            callee: Box::new(callee),
            arguments,
        },
    );
    assert!(matches!(name.value, Expr::Name(_)));
    let Expr::Call { callee, arguments } = &call.value else {
        panic!("explicit application is a call");
    };
    assert!(arguments.value.is_empty());
    assert_eq!(text(&file, callee), "f");
    assert_eq!(text(&file, arguments), "()");
    assert_eq!(text(&file, &name), "f");
    assert_eq!(text(&file, &call), "f()");
}

#[test]
fn dotted_calls_keep_named_arguments_out_of_bindings() {
    let mut file = file("io.print(0x2a, label: 'hé\\n')\n");
    let base = node(&mut file, "io", 0, Expr::Name("io".into()));
    let name = node(&mut file, "print", 0, "print".into());
    let callee = node(
        &mut file,
        "io.print",
        0,
        Expr::Member {
            base: Box::new(base),
            name,
        },
    );
    let number = node(&mut file, "0x2a", 0, Expr::Literal(LiteralKind::Integer));
    let positional = Spanned::new(Argument::Positional(number.clone()), number.span_id);
    let name = node(&mut file, "label", 0, "label".into());
    let value = node(&mut file, "'hé\\n'", 0, Expr::Literal(LiteralKind::String));
    let named = node(
        &mut file,
        "label: 'hé\\n'",
        0,
        Argument::Named { name, value },
    );
    let arguments = node(
        &mut file,
        "(0x2a, label: 'hé\\n')",
        0,
        vec![positional, named],
    );
    let call = node(
        &mut file,
        "io.print(0x2a, label: 'hé\\n')",
        0,
        Expr::Call {
            callee: Box::new(callee),
            arguments,
        },
    );
    let Expr::Call { callee, arguments } = &call.value else {
        panic!("expected a call");
    };
    assert!(matches!(callee.value, Expr::Member { .. }));
    assert!(matches!(arguments.value[0].value, Argument::Positional(_)));
    let Argument::Named { name, value } = &arguments.value[1].value else {
        panic!("a named argument is its own syntax");
    };
    assert_eq!(name.value, "label");
    assert_eq!(text(&file, value), "'hé\\n'");
    assert_eq!(text(&file, &number), "0x2a");
}

#[test]
fn duplicate_bindings_and_private_marker_keep_source_order() {
    let mut file = file("answer := 1\nprivate\nanswer := 2\n");
    let mut bindings = Vec::new();
    for (occurrence, spelling) in ["1", "2"].into_iter().enumerate() {
        let name = node(&mut file, "answer", occurrence, "answer".into());
        let operator = node(&mut file, ":=", occurrence, BindingOperator::ColonEquals);
        let expression = node(&mut file, spelling, 0, Expr::Literal(LiteralKind::Integer));
        let binding = Binding {
            attributes: Vec::new(),
            name,
            parameters: None,
            convention: None,
            result_binder: None,
            annotation: None,
            contract: None,
            alternatives: Vec::new(),
            extra_results: Vec::new(),
            value: Some(BindingValue {
                operator,
                expression,
            }),
        };
        bindings.push(node(
            &mut file,
            &format!("answer := {spelling}"),
            0,
            Item::Binding(Box::new(binding)),
        ));
    }
    let private = node(&mut file, "private", 0, Item::Private);
    bindings.insert(1, private);
    file.items = bindings;
    assert_eq!(file.items.len(), 3);
    assert_eq!(text(&file, &file.items[0]), "answer := 1");
    assert!(matches!(file.items[1].value, Item::Private));
    assert_eq!(text(&file, &file.items[2]), "answer := 2");
    let Item::Binding(first) = &file.items[0].value else {
        panic!("expected the first binding");
    };
    let Item::Binding(second) = &file.items[2].value else {
        panic!("expected the second binding");
    };
    assert_eq!(first.name.value, second.name.value);
    assert_ne!(first.name.span_id, second.name.span_id);
}

#[test]
fn duplicate_parameters_remain_available_for_diagnostics() {
    let mut file = file("echo(x Int, x Int) Int: return x\n");
    let mut parameters = Vec::new();
    for occurrence in 0..2 {
        let name = node(&mut file, "x", occurrence, "x".into());
        let annotation = node(&mut file, "Int", occurrence, Expr::Name("Int".into()));
        let parameter = Parameter {
            name,
            convention: None,
            annotation: Some(Spanned::new(
                Annotation::Expression(annotation.clone()),
                annotation.span_id,
            )),
            default: None,
        };
        parameters.push(node(&mut file, "x Int", occurrence, parameter));
    }
    let parameters = node(&mut file, "(x Int, x Int)", 0, parameters);
    assert_eq!(parameters.value.len(), 2);
    assert_eq!(parameters.value[0].value.name.value, "x");
    assert_eq!(parameters.value[1].value.name.value, "x");
    assert_ne!(
        parameters.value[0].value.name.span_id,
        parameters.value[1].value.name.span_id
    );
    assert_eq!(text(&file, &parameters), "(x Int, x Int)");
}

#[test]
fn parameter_defaults_keep_the_written_expression() {
    let mut file = file("count: 0x04");
    let name = node(&mut file, "count", 0, "count".into());
    let default = node(&mut file, "0x04", 0, Expr::Literal(LiteralKind::Integer));
    let parameter = Parameter {
        name,
        convention: None,
        annotation: None,
        default: Some(default),
    };
    assert!(parameter.annotation.is_none());
    let default = parameter.default.as_ref().expect("written default");
    assert_eq!(text(&file, default), "0x04");
}

#[test]
fn absent_and_explicit_empty_parameter_lists_are_distinct() {
    let mut file = file("value := 1\nfunction() := 1\n");
    let value_name = node(&mut file, "value", 0, "value".into());
    let function_name = node(&mut file, "function", 0, "function".into());
    let parameters = node(&mut file, "()", 0, Vec::new());
    let mut bindings = Vec::new();
    for (occurrence, (name, parameters)) in [(value_name, None), (function_name, Some(parameters))]
        .into_iter()
        .enumerate()
    {
        let operator = node(&mut file, ":=", occurrence, BindingOperator::ColonEquals);
        let expression = node(
            &mut file,
            "1",
            occurrence,
            Expr::Literal(LiteralKind::Integer),
        );
        bindings.push(Binding {
            attributes: Vec::new(),
            name,
            parameters,
            convention: None,
            result_binder: None,
            annotation: None,
            contract: None,
            alternatives: Vec::new(),
            extra_results: Vec::new(),
            value: Some(BindingValue {
                operator,
                expression,
            }),
        });
    }
    assert!(bindings[0].parameters.is_none());
    let parameters = bindings[1].parameters.as_ref().expect("written list");
    assert!(parameters.value.is_empty());
    assert_eq!(text(&file, parameters), "()");
}

#[test]
fn annotations_can_be_expressions_without_resolving_a_type() {
    let mut file = file("items Buffer(Int, 4)\n");
    let callee = node(&mut file, "Buffer", 0, Expr::Name("Buffer".into()));
    let element = node(&mut file, "Int", 0, Expr::Name("Int".into()));
    let count = node(&mut file, "4", 0, Expr::Literal(LiteralKind::Integer));
    let arguments = node(
        &mut file,
        "(Int, 4)",
        0,
        vec![
            Spanned::new(Argument::Positional(element.clone()), element.span_id),
            Spanned::new(Argument::Positional(count.clone()), count.span_id),
        ],
    );
    let annotation = node(
        &mut file,
        "Buffer(Int, 4)",
        0,
        Expr::Call {
            callee: Box::new(callee),
            arguments,
        },
    );
    let name = node(&mut file, "items", 0, "items".into());
    let binding = Binding {
        attributes: Vec::new(),
        name,
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: Some(Spanned::new(
            Annotation::Expression(annotation.clone()),
            annotation.span_id,
        )),
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    assert!(binding.value.is_none());
    let annotation = binding.annotation.as_ref().expect("written annotation");
    assert!(matches!(&annotation.value, Annotation::Expression(expr)
        if matches!(expr.value, Expr::Call { .. })));
    assert_eq!(text(&file, annotation), "Buffer(Int, 4)");
}

#[test]
fn grouping_and_operator_locations_survive() {
    let mut file = file("(x + 0x2a)\n");
    let left = node(&mut file, "x", 0, Expr::Name("x".into()));
    let operator = node(&mut file, "+", 0, BinaryOperator::Add);
    let right = node(&mut file, "0x2a", 0, Expr::Literal(LiteralKind::Integer));
    let binary = node(
        &mut file,
        "x + 0x2a",
        0,
        Expr::Binary {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        },
    );
    let group = node(&mut file, "(x + 0x2a)", 0, Expr::Group(Box::new(binary)));
    let Expr::Group(inner) = &group.value else {
        panic!("parentheses are represented");
    };
    let Expr::Binary { operator, .. } = &inner.value else {
        panic!("expected the grouped binary");
    };
    assert_eq!(text(&file, &group), "(x + 0x2a)");
    assert_eq!(text(&file, inner), "x + 0x2a");
    assert_eq!(text(&file, operator), "+");
}

#[test]
fn type_extensions_remain_separate_source_declarations() {
    let mut file = file("Thing is 'thing'\nThing has\n    value Int\n");
    let name = node(&mut file, "Thing", 0, "Thing".into());
    let literal = node(&mut file, "'thing'", 0, Expr::Literal(LiteralKind::String));
    let body = node(&mut file, "is 'thing'", 0, TypeBody::Is(literal));
    let first = TypeDeclaration {
        name,
        parameters: None,
        body,
    };
    let first = node(
        &mut file,
        "Thing is 'thing'",
        0,
        Item::Type(Box::new(first)),
    );
    let name = node(&mut file, "Thing", 1, "Thing".into());
    let member_name = node(&mut file, "value", 0, "value".into());
    let annotation = node(&mut file, "Int", 0, Expr::Name("Int".into()));
    let member = Binding {
        attributes: Vec::new(),
        name: member_name,
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: Some(Spanned::new(
            Annotation::Expression(annotation.clone()),
            annotation.span_id,
        )),
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    let member = node(
        &mut file,
        "value Int",
        0,
        ast::TypeMember::Binding(Box::new(member)),
    );
    let body = node(
        &mut file,
        "has\n    value Int",
        0,
        TypeBody::Has(vec![member]),
    );
    let second = TypeDeclaration {
        name,
        parameters: None,
        body,
    };
    let second = node(
        &mut file,
        "Thing has\n    value Int",
        0,
        Item::Type(Box::new(second)),
    );
    file.items = vec![first, second];
    assert_eq!(file.items.len(), 2);
    let Item::Type(first) = &file.items[0].value else {
        panic!("expected a type declaration");
    };
    let Item::Type(second) = &file.items[1].value else {
        panic!("expected a separate extension");
    };
    assert_eq!(first.name.value, second.name.value);
    assert_ne!(first.name.span_id, second.name.span_id);
    assert!(matches!(first.body.value, TypeBody::Is(_)));
    assert!(matches!(second.body.value, TypeBody::Has(_)));
}

#[test]
fn local_bindings_stay_inside_the_source_body() {
    let mut file = file("main:\n    count := 1\n    return count\n");
    let name = node(&mut file, "count", 0, "count".into());
    let operator = node(&mut file, ":=", 0, BindingOperator::ColonEquals);
    let expression = node(&mut file, "1", 0, Expr::Literal(LiteralKind::Integer));
    let binding = Binding {
        attributes: Vec::new(),
        name,
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: Some(BindingValue {
            operator,
            expression,
        }),
    };
    let binding = node(
        &mut file,
        "count := 1",
        0,
        Statement::Binding(Box::new(binding)),
    );
    let value = node(&mut file, "count", 1, Expr::Name("count".into()));
    let returned = node(
        &mut file,
        "return count",
        0,
        Statement::Return(ReturnStatement {
            value: Some(value),
        }),
    );
    let block = Block {
        statements: vec![binding, returned],
    };
    assert!(file.items.is_empty());
    assert_eq!(block.statements.len(), 2);
    assert!(matches!(block.statements[0].value, Statement::Binding(_)));
    assert!(matches!(block.statements[1].value, Statement::Return(_)));
    assert_eq!(text(&file, &block.statements[0]), "count := 1");
}

#[test]
fn returns_stay_in_body_order_including_unreachable_syntax() {
    let mut file = file("main:\n    return 1\n    #todo\n");
    let value = node(&mut file, "1", 0, Expr::Literal(LiteralKind::Integer));
    let returned = node(
        &mut file,
        "return 1",
        0,
        Statement::Return(ReturnStatement {
            value: Some(value),
        }),
    );
    let hole = node(&mut file, "#todo", 0, Expr::Hole);
    let hole = Spanned::new(Statement::Expression(hole.clone()), hole.span_id);
    let block = node(
        &mut file,
        "return 1\n    #todo",
        0,
        Expr::Block(Block {
            statements: vec![returned, hole],
        }),
    );
    let name = node(&mut file, "main", 0, "main".into());
    let operator = node(&mut file, ":", 0, BindingOperator::Colon);
    let binding = Binding {
        attributes: Vec::new(),
        name,
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: Some(BindingValue {
            operator,
            expression: block,
        }),
    };
    let body = &binding.value.as_ref().expect("written body").expression;
    let Expr::Block(block) = &body.value else {
        panic!("expected an indented body");
    };
    assert_eq!(block.statements.len(), 2);
    assert!(matches!(block.statements[0].value, Statement::Return(_)));
    let Statement::Return(returned) = &block.statements[0].value else {
        unreachable!()
    };
    assert!(returned.value.is_some());
    assert!(matches!(
        block.statements[1].value,
        Statement::Expression(Spanned {
            value: Expr::Hole,
            ..
        })
    ));
    assert_eq!(text(&file, &block.statements[0]), "return 1");
    assert_eq!(text(&file, &block.statements[1]), "#todo");
}

#[test]
fn bare_return_is_not_a_synthesized_unit_expression() {
    let mut file = file("main: return\n");
    let statement = node(
        &mut file,
        "return",
        0,
        Statement::Return(ReturnStatement {
            value: None,
        }),
    );
    let Statement::Return(returned) = &statement.value else {
        panic!("expected the written return");
    };
    assert!(returned.value.is_none());
    assert_eq!(text(&file, &statement), "return");
}

#[test]
fn comments_and_literal_spelling_use_the_original_snapshot() {
    let source = "--| module\n--- docs\nvalue := 1.00 -- ordinary\n\n'hé\\n'\n";
    let mut file = file(source);
    let module = node(&mut file, "--| module", 0, CommentKind::ModuleDocumentation);
    let docs = node(&mut file, "--- docs", 0, CommentKind::Documentation);
    let ordinary = node(&mut file, "-- ordinary", 0, CommentKind::Ordinary);
    let float = node(&mut file, "1.00", 0, Expr::Literal(LiteralKind::Float));
    let string = node(&mut file, "'hé\\n'", 0, Expr::Literal(LiteralKind::String));
    file.comments = vec![module, docs, ordinary];
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(text(&file, &float), "1.00");
    assert_eq!(text(&file, &string), "'hé\\n'");
    let comments: Vec<_> = file
        .comments
        .iter()
        .map(|comment| text(&file, comment))
        .collect();
    assert_eq!(comments, ["--| module", "--- docs", "-- ordinary"]);
    let snapshot = file.clone();
    assert!(std::sync::Arc::ptr_eq(&file.text, &snapshot.text));
    assert_eq!(file, snapshot);
}
