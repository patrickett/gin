use ast::{Annotation, BindingOperator, Expr, Item, LiteralKind, SourceFile, Spanned};

fn text<'a, T>(file: &'a SourceFile, node: &Spanned<T>) -> &'a str {
    file.span_table.get(node.span_id).extract(&file.text)
}

#[test]
fn parses_a_binding_without_decoding_its_literal() {
    let file = parser::parse_strict("answer := 0x2a\n").expect("supported binding");
    assert_eq!(file.items.len(), 1);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected a source binding");
    };
    assert_eq!(binding.name.value, "answer");
    assert!(binding.parameters.is_none());
    assert!(binding.annotation.is_none());
    let value = binding.value.as_ref().expect("written initializer");
    assert_eq!(value.operator.value, BindingOperator::ColonEquals);
    assert_eq!(value.expression.value, Expr::Literal(LiteralKind::Integer));
    assert_eq!(text(&file, &value.expression), "0x2a");
    assert_eq!(text(&file, &file.items[0]), "answer := 0x2a");
    assert_eq!(file.text.as_ref(), "answer := 0x2a\n");
}

#[test]
fn self_annotations_use_the_same_source_expression_rule_everywhere() {
    let source = "\
replace(previous Self, next Self) Self: return next
x Self := self
f() Self: return self
";
    let file = parser::parse_strict(source).expect("simple Self annotations");
    assert_eq!(file.items.len(), 3);
    let Item::Binding(replace) = &file.items[0].value else {
        panic!("replace function");
    };
    for parameter in &replace.parameters.as_ref().unwrap().value {
        let Annotation::Expression(expression) =
            &parameter.value.annotation.as_ref().unwrap().value
        else {
            panic!("written Self annotation");
        };
        assert_eq!(expression.value, Expr::Name("Self".into()));
    }
    let Annotation::Expression(expression) = &replace.annotation.as_ref().unwrap().value else {
        panic!("written Self return annotation");
    };
    assert_eq!(expression.value, Expr::Name("Self".into()));
}

#[test]
fn anonymous_tag_parameters_are_diagnosed_not_fabricated_as_names() {
    let output = parser::parse("f(Int) Int: return 1\n").unwrap();
    assert!(output.has_errors());
    assert!(parser::parse_strict("f(Int) Int: return 1\n").is_err());
}

#[test]
fn parses_nested_calls_named_arguments_and_groups() {
    let source = "result := io.add(f(), right: (x + 2) * 3)\nf\nf()\n";
    let file = parser::parse_strict(source).expect("supported expressions");
    assert_eq!(file.items.len(), 3);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected a binding");
    };
    let Expr::Call { callee, arguments } = &binding
        .value
        .as_ref()
        .expect("initializer")
        .expression
        .value
    else {
        panic!("expected explicit application");
    };
    assert!(matches!(callee.value, Expr::Member { .. }));
    assert_eq!(arguments.value.len(), 2);
    let ast::Argument::Named { name, value } = &arguments.value[1].value else {
        panic!("expected a named argument");
    };
    assert_eq!(name.value, "right");
    assert_eq!(text(&file, value), "(x + 2) * 3");
    let Expr::Binary { left, operator, .. } = &value.value else {
        panic!("expected a binary expression");
    };
    assert_eq!(operator.value, ast::BinaryOperator::Multiply);
    assert!(matches!(left.value, Expr::Group(_)));
    assert!(matches!(
        file.items[1].value,
        Item::Expression(Spanned {
            value: Expr::Name(_),
            ..
        })
    ));
    assert!(matches!(
        file.items[2].value,
        Item::Expression(Spanned {
            value: Expr::Call { .. },
            ..
        })
    ));
}

#[test]
fn parses_functions_and_bodies_without_lifting_the_return() {
    let source = "\
answer := 42
add(left Int, right Int) Int:
    sum := left + right
    return sum
result := add(answer, right: 1)
";
    let file = parser::parse_strict(source).expect("initial parser slice");
    assert_eq!(file.items.len(), 3);
    let Item::Binding(function) = &file.items[1].value else {
        panic!("expected a function binding");
    };
    let parameters = function.parameters.as_ref().expect("written parameters");
    assert_eq!(parameters.value.len(), 2);
    assert_eq!(parameters.value[0].value.name.value, "left");
    assert_eq!(parameters.value[1].value.name.value, "right");
    assert_eq!(text(&file, function.annotation.as_ref().unwrap()), "Int");
    let body = &function.value.as_ref().expect("function body").expression;
    let Expr::Block(block) = &body.value else {
        panic!("expected a source block");
    };
    assert_eq!(block.statements.len(), 2);
    assert!(matches!(
        block.statements[0].value,
        ast::Statement::Binding(_)
    ));
    let ast::Statement::Return(returned) = &block.statements[1].value else {
        panic!("return remains the second statement");
    };
    assert_eq!(text(&file, returned.value.as_ref().unwrap()), "sum");
    assert_eq!(text(&file, &block.statements[1]), "return sum");
    assert_eq!(
        text(&file, &file.items[1]),
        &source[13..source.find("\nresult").unwrap()]
    );
}

#[test]
fn comment_only_indentation_does_not_change_the_body() {
    let source = "\
--| module
--- docs
main:
    value := 'hé\\n' -- inline
        -- deeper comment only
    return value
-- trailing
";
    let file = parser::parse_strict(source).expect("comments are source trivia");
    assert_eq!(file.items.len(), 1);
    assert_eq!(file.comments.len(), 5);
    assert_eq!(text(&file, &file.comments[0]), "--| module");
    assert_eq!(text(&file, &file.comments[2]), "-- inline");
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected a binding");
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("expected a block");
    };
    assert_eq!(block.statements.len(), 2);
    let ast::Statement::Binding(local) = &block.statements[0].value else {
        panic!("expected a local binding");
    };
    assert_eq!(
        text(&file, &local.value.as_ref().unwrap().expression),
        "'hé\\n'"
    );
    assert_eq!(file.text.as_ref(), source);
}

#[test]
fn newline_lists_do_not_leak_indentation_into_function_bodies() {
    let source = "\
add(
    left Int
    right: 1) Int:
    return combine(
        left
        right)
next := 0
";
    let file = parser::parse_strict(source).expect("newline-separated lists");
    assert_eq!(file.items.len(), 2);
    let Item::Binding(function) = &file.items[0].value else {
        panic!("expected a function");
    };
    let parameters = function.parameters.as_ref().unwrap();
    assert_eq!(parameters.value.len(), 2);
    assert_eq!(text(&file, parameters), "(\n    left Int\n    right: 1)");
    assert!(parameters.value[1].value.annotation.is_none());
    assert_eq!(
        text(&file, parameters.value[1].value.default.as_ref().unwrap()),
        "1"
    );
    let Expr::Block(block) = &function.value.as_ref().unwrap().expression.value else {
        panic!("expected a body");
    };
    let ast::Statement::Return(returned) = &block.statements[0].value else {
        panic!("expected a return");
    };
    let Expr::Call { arguments, .. } = &returned.value.as_ref().unwrap().value else {
        panic!("expected a call");
    };
    assert_eq!(arguments.value.len(), 2);
}

#[test]
fn duplicate_declarations_and_parameters_survive_in_source_order() {
    let source = "same(x Int, x Int) Int: return x\nprivate\nsame() := 2\nsame Int\n";
    let file = parser::parse_strict(source).expect("duplicates are not parser semantics");
    assert_eq!(file.items.len(), 4);
    let Item::Binding(first) = &file.items[0].value else {
        panic!("first occurrence");
    };
    assert_eq!(first.parameters.as_ref().unwrap().value.len(), 2);
    assert!(matches!(file.items[1].value, Item::Private));
    let Item::Binding(second) = &file.items[2].value else {
        panic!("second occurrence");
    };
    assert_eq!(first.name.value, second.name.value);
    assert_ne!(first.name.span_id, second.name.span_id);
    assert!(second.parameters.as_ref().unwrap().value.is_empty());
    let Item::Binding(third) = &file.items[3].value else {
        panic!("third occurrence");
    };
    assert!(third.parameters.is_none());
    assert!(third.value.is_none());
    assert_eq!(text(&file, third.annotation.as_ref().unwrap()), "Int");
}

#[test]
fn supports_bare_returns_holes_and_eof_without_a_final_newline() {
    let file = parser::parse_strict("main:\n    return\n    #todo").unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("expected main");
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("expected a block");
    };
    let ast::Statement::Return(returned) = &block.statements[0].value else {
        panic!("bare return");
    };
    assert!(returned.value.is_none());
    assert!(matches!(
        block.statements[1].value,
        ast::Statement::Expression(Spanned {
            value: Expr::Hole,
            ..
        })
    ));
}

#[test]
fn parses_real_operator_fixture_functions_without_the_old_ast() {
    let fixture = include_str!(
        "../../test-fixtures/fixtures/ordinary_integer_literal_infix_declarations.gin"
    );
    let start = fixture
        .find("add(a Word, b Word) Word: return b\n")
        .unwrap();
    let source = &fixture[start..];
    let file = parser::parse_strict(source).expect("fixture's ordinary function tail");
    assert_eq!(file.items.len(), 11);
    assert_eq!(file.text.as_ref(), source);
    let Item::Binding(last) = &file.items[10].value else {
        panic!("last fixture function");
    };
    assert_eq!(last.name.value, "use_shift_right");
}

#[test]
fn empty_and_comment_only_files_are_complete_source_snapshots() {
    for source in [
        "",
        "\n\n",
        "-- only",
        "    -- only\n",
        "--- docs\n--| module\n",
    ] {
        let file = parser::parse_strict(source).expect("no source expressions");
        assert!(file.items.is_empty());
        assert_eq!(file.text.as_ref(), source);
    }
}

#[test]
fn tab_indentation_preserves_the_original_source() {
    let source = "main:\n\treturn 1\nnext := 0\n";
    let file = parser::parse_strict(source).expect("source lexer tab width");
    assert_eq!(file.items.len(), 2);
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(text(&file, &file.items[0]), "main:\n\treturn 1");
}

#[test]
fn adapter_diagnostics_preserve_raw_crlf_offsets() {
    let source = "x := 1\r\ny := 2\r\n";
    let errors =
        parser::parse_strict(source).expect_err("adapter reports raw unknown CR markers");
    assert_eq!(errors.len(), 2);
    for error in errors {
        assert_eq!(error.code.slug(), "lex-unexpected-character");
        assert_eq!(error.span.extract(source), "\r");
    }
}

#[test]
fn strict_adapter_rejects_recovered_or_unsupported_source() {
    for source in [
        "use core.\n",
        "Thing is in 0 to 255 to 256\n",
        "x := [1, 2; 3]\n",
        "x := +1\n",
        "x := f(1 2)\n",
        "x := f(value:)\n",
        "x := f(1 +)\n",
        "x := (1\n",
        "x := 1\nuse core as\n",
        "x := 1 as\n",
        "x := 1 == 1\n",
        "x := 1 /= 2\n",
        "main:\n-- no body\n",
        "f(x Int: 1): return x\n",
        "f(ref x): return x\n",
        "x :: 2\n",
        "return 1\n",
        "x := 'unterminated\n",
    ] {
        let errors = parser::parse_strict(source).expect_err(source);
        assert!(!errors.is_empty(), "{source}");
        for error in errors {
            assert!(error.span.end() <= source.len(), "{error:?}");
            assert!(source.is_char_boundary(error.span.start()), "{error:?}");
            assert!(source.is_char_boundary(error.span.end()), "{error:?}");
        }
    }
}

#[test]
fn short_token_combinations_do_not_panic_or_hide_errors() {
    let parts = [
        "x", "1", "'hé'", "(", ")", ":", ":=", "+", ",", ".", "return", "#todo",
    ];
    for first in parts {
        for second in parts {
            for third in parts {
                let source = format!("{first} {second} {third}\n");
                match parser::parse_strict(&source) {
                    Ok(file) => assert_eq!(file.text.as_ref(), source),
                    Err(errors) => assert!(!errors.is_empty(), "{source}"),
                }
            }
        }
    }
}
