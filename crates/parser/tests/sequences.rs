use std::cell::Cell;

use ast::{
    Annotation, Argument, Binding, CollectionPunctuation, Expr, Expression, Item, LiteralKind,
    ReturnStatement, SequenceDelimiters, SourceFile, SpanId, Statement, SyntaxNodeKind, TypeBody,
    TypeMember,
};
use lexer::{Lexer, TokenKind};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions, parse_with_options};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("binding")
    };
    binding
}

fn value(file: &SourceFile, index: usize) -> &Expression {
    &binding(file, index).value.as_ref().unwrap().expression
}

fn spelling(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

fn annotation(binding: &Binding) -> &Expression {
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("expression annotation")
    };
    expression
}

fn returned_at(file: &SourceFile, offset: usize) -> &ReturnStatement {
    let hit = file.node_at_offset(offset).unwrap();
    let SyntaxNodeKind::Statement(statement) = hit.node.kind else {
        panic!("return statement")
    };
    let Statement::Return(returned) = &statement.value else {
        panic!("return")
    };
    returned
}

#[test]
fn groups_sequences_calls_repeats_and_arrays_are_distinct() {
    let source = concat!(
        "empty: ()\ngroup: (x)\none: (x,)\npair: (x, y,)\n",
        "call: f(a,b)\nsequence_call: f((a,b))\nspaced_call: f (a,b)\n",
        "repeat: (seed;count)\narray: [x,y]\n",
        "nested: ((a,), (), [b], (seed; count))\n",
    );
    let file = parser::parse_strict(source).unwrap();
    for (index, count, commas) in [(0, 0, 0), (2, 1, 1), (3, 2, 2), (9, 4, 3)] {
        let Expr::Sequence(sequence) = &value(&file, index).value else {
            panic!("sequence")
        };
        assert_eq!(sequence.elements.len(), count);
        assert_eq!(sequence.separators.len(), commas);
        let SequenceDelimiters::Parenthesized { open, close } = sequence.delimiters else {
            panic!("written parentheses")
        };
        assert_eq!(spelling(&file, open), "(");
        assert_eq!(spelling(&file, close), ")");
        for comma in &sequence.separators {
            assert_eq!(spelling(&file, *comma), ",");
        }
    }
    assert!(matches!(value(&file, 1).value, Expr::Group(_)));
    assert!(matches!(value(&file, 7).value, Expr::Repeat(_)));
    assert!(matches!(value(&file, 8).value, Expr::Array(_)));
    for (index, count) in [(4, 2), (5, 1), (6, 2)] {
        let Expr::Call { arguments, .. } = &value(&file, index).value else {
            panic!("call")
        };
        assert_eq!(arguments.value.len(), count);
        if count == 1 {
            assert!(matches!(
                arguments.value[0].value,
                Argument::Positional(ast::Spanned {
                    value: Expr::Sequence(_),
                    ..
                })
            ));
        }
    }
}

#[test]
fn grouped_headers_use_syntax_not_type_names_in_every_role() {
    let source = concat!(
        "Holder(element) has\n",
        "    field (element, element)\n",
        "    method(self, value (element, element)) result (element, element)\n",
        "swap(element Type, values (element, element)) (element, element):\n",
        "    local (element, element): values\n",
        "    return (values.1, values.[0])\n",
        "spaced (left, right): value\n",
        "f() pair(T,T): return value\n",
        "singleton(value (T,)) (T,): return (value.0,)\n",
        "unit(value ()) (): return ()\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(holder) = &file.items[0].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &holder.body.value else {
        panic!("members")
    };
    for member in members {
        let TypeMember::Binding(member) = &member.value else {
            panic!("member")
        };
        assert!(matches!(annotation(member).value, Expr::Sequence(_)));
    }
    let function = binding(&file, 1);
    assert_eq!(function.parameters.as_ref().unwrap().value.len(), 2);
    assert!(matches!(annotation(function).value, Expr::Sequence(_)));
    let Expr::Block(body) = &value(&file, 1).value else {
        panic!("body")
    };
    let Statement::Binding(local) = &body.statements[0].value else {
        panic!("local")
    };
    assert!(local.parameters.is_none());
    assert!(matches!(annotation(local).value, Expr::Sequence(_)));
    assert!(binding(&file, 2).parameters.is_none());
    assert!(matches!(
        annotation(binding(&file, 2)).value,
        Expr::Sequence(_)
    ));
    assert!(binding(&file, 3).result_binder.is_none());
    assert!(matches!(
        annotation(binding(&file, 3)).value,
        Expr::Call { .. }
    ));
}

#[test]
fn return_lists_and_parenthesized_sequences_share_one_expression_shape() {
    let file =
        parser::parse_strict("bare() (T,T): return a,b\nparen() (T,T): return (a,b)\n")
            .unwrap();
    for index in 0..2 {
        let Expr::Block(block) = &value(&file, index).value else {
            panic!("return body")
        };
        let Statement::Return(statement) = &block.statements[0].value else {
            panic!("return")
        };
        let Expr::Sequence(sequence) = &statement.value.as_ref().unwrap().value else {
            panic!("one sequence expression")
        };
        assert_eq!(sequence.elements.len(), 2);
        assert_eq!(sequence.elements[0].value, Expr::Name("a".into()));
        assert_eq!(sequence.elements[1].value, Expr::Name("b".into()));
        assert_eq!(sequence.separators.len(), 1);
        assert_eq!(
            matches!(sequence.delimiters, SequenceDelimiters::Bare),
            index == 0
        );
    }
}

#[test]
fn return_sequence_forms_are_equivalent_in_every_statement_context() {
    for (written, count) in [("a,b", 2), ("(a,b)", 2), ("a,b,c", 3), ("(a,b,c)", 3)] {
        let source = format!(
            concat!(
                "inline(): return {written}\n",
                "block():\n    return {written}\n",
                "nested():\n    inner(): return {written}\n",
                "    while flag do\n        return {written}\n",
                "    for item in data do\n        return {written}\n",
                "    choice: when tag is\n",
                "        Some(x) then\n            return {written}\n",
                "        None then\n            local(): return {written}\n",
            ),
            written = written,
        );
        let file = parser::parse_strict(&source).unwrap();
        assert_eq!(source.match_indices("return").count(), 7);
        for (offset, _) in source.match_indices("return") {
            let returned = returned_at(&file, offset);
            let value = returned.value.as_ref().unwrap();
            assert_eq!(spelling(&file, value.span_id), written);
            let Expr::Sequence(sequence) = &value.value else {
                panic!("sequence")
            };
            assert_eq!(sequence.elements.len(), count);
            assert_eq!(sequence.elements[0].value, Expr::Name("a".into()));
            assert_eq!(sequence.elements[1].value, Expr::Name("b".into()));
            assert_eq!(sequence.separators.len(), count - 1);
            assert_eq!(spelling(&file, sequence.separators[0]), ",");
            assert_eq!(
                matches!(sequence.delimiters, SequenceDelimiters::Bare),
                !written.starts_with('('),
            );
            let comma = file.span_table.get(sequence.separators[0]).start();
            let hit = file.node_at_offset(comma).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::CollectionPunctuation(CollectionPunctuation::Comma)
            ));
            assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
                SyntaxNodeKind::Sequence(found) if std::ptr::eq(found, sequence.as_ref()))));
        }
    }
}

#[test]
fn absent_scalar_group_unit_and_singleton_returns_keep_written_shapes() {
    for (written, shape) in [
        ("", "absent"),
        ("value", "name"),
        ("(value)", "group"),
        ("()", "unit"),
        ("(value,)", "singleton"),
    ] {
        let source = format!("f(): return {written}\n");
        let file = parser::parse_strict(&source).unwrap();
        let returned = returned_at(&file, source.find("return").unwrap());
        let actual = match returned.value.as_ref().map(|value| &value.value) {
            None => "absent",
            Some(Expr::Name(_)) => "name",
            Some(Expr::Group(_)) => "group",
            Some(Expr::Sequence(sequence)) => {
                assert!(matches!(
                    sequence.delimiters,
                    SequenceDelimiters::Parenthesized { .. }
                ));
                if sequence.elements.is_empty() {
                    "unit"
                } else {
                    assert_eq!(sequence.elements.len(), 1);
                    "singleton"
                }
            }
            _ => panic!("unexpected expression"),
        };
        assert_eq!(actual, shape);
    }
    let file = parser::parse_strict("f(): return()\n").unwrap();
    let returned = returned_at(&file, 5);
    assert!(
        matches!(&returned.value.as_ref().unwrap().value, Expr::Sequence(sequence)
        if sequence.elements.is_empty())
    );
}

#[test]
fn nested_loop_and_case_returns_keep_lists_without_comma_expressions() {
    let source = concat!(
        "f() (T,T):\n",
        "    while flag do\n        return first,second\n",
        "    value: when tag is\n        Some(x) then\n            return x,other\n",
        "    return (first,second)\nlater: 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(block) = &value(&file, 0).value else {
        panic!("body")
    };
    let Statement::Expression(loop_expression) = &block.statements[0].value else {
        panic!("loop")
    };
    let Expr::Loop(loop_expression) = &loop_expression.value else {
        panic!("loop")
    };
    let ast::LoopExpression::While { body, .. } = loop_expression.as_ref() else {
        panic!("while")
    };
    let Expr::Block(body) = &body.value else {
        panic!("body")
    };
    let Statement::Return(statement) = &body.statements[0].value else {
        panic!("return")
    };
    let Expr::Sequence(sequence) = &statement.value.as_ref().unwrap().value else {
        panic!("sequence")
    };
    assert_eq!(sequence.elements.len(), 2);
    assert_eq!(sequence.delimiters, SequenceDelimiters::Bare);
    let Statement::Binding(case) = &block.statements[1].value else {
        panic!("case")
    };
    let Expr::Match(case) = &case.value.as_ref().unwrap().expression.value else {
        panic!("case")
    };
    let Expr::Block(body) = &case.arms.value[0].value.body.value else {
        panic!("arm")
    };
    let Statement::Return(statement) = &body.statements[0].value else {
        panic!("return")
    };
    let Expr::Sequence(sequence) = &statement.value.as_ref().unwrap().value else {
        panic!("sequence")
    };
    assert_eq!(sequence.elements.len(), 2);
    assert_eq!(sequence.delimiters, SequenceDelimiters::Bare);
}

#[test]
fn incomplete_bare_return_sequences_retain_values_commas_and_later_items() {
    for context in ["f(): return ", "f():\n    while flag do\n        return "] {
        for written in ["a,", "a,", "a,b +"] {
            let source = format!("{context}{written}\nlater: 2\n");
            let output = parser::parse(&source).unwrap();
            assert!(output.has_errors(), "{source}");
            assert!(parser::parse_strict(&source).is_err());
            assert_eq!(
                output.file.declaration_outline().last().unwrap().name,
                "later"
            );
            let returned = returned_at(&output.file, source.find("return").unwrap());
            let value = returned.value.as_ref().unwrap();
            let Expr::Sequence(sequence) = &value.value else {
                panic!("sequence")
            };
            assert_eq!(sequence.delimiters, SequenceDelimiters::Bare);
            assert_eq!(sequence.elements.len(), 2);
            assert_eq!(sequence.separators.len(), 1);
            assert_eq!(spelling(&output.file, sequence.separators[0]), ",");
            let missing = match &sequence.elements[1].value {
                Expr::Missing => &sequence.elements[1],
                Expr::Binary { right, .. } => right,
                _ => panic!("missing operand"),
            };
            assert_eq!(missing.value, Expr::Missing);
            assert!(output.file.span_table.get(missing.span_id).is_empty());
        }
    }
}

#[test]
fn numeric_projection_chains_have_exact_spans_without_changing_raw_float_tokens() {
    let source = concat!(
        "chain: data.0.1.2.3\nbracket: data.[0]\n",
        "float: 0.1\nfloat_select: data.[0.1]\n",
        "mixed: data.0.[1].field(2.5)\n",
        "huge: data.999999999999999999999999999999999999\n",
    );
    let raw: Vec<_> = Lexer::new("data.0.1").map(|token| token.kind).collect();
    assert_eq!(raw, [TokenKind::Id, TokenKind::Dot, TokenKind::Float]);
    let file = parser::parse_strict(source).unwrap();
    let mut chain = value(&file, 0);
    for digit in ["3", "2", "1", "0"] {
        let Expr::NumericSelect(select) = &chain.value else {
            panic!("numeric projection")
        };
        assert_eq!(spelling(&file, select.dot), ".");
        assert_eq!(spelling(&file, select.index.span_id), digit);
        assert_eq!(select.index.value, Expr::Literal(LiteralKind::Integer));
        assert_eq!(
            file.span_table.get(chain.span_id).end(),
            file.span_table.get(select.index.span_id).end(),
        );
        chain = &select.base;
    }
    assert_eq!(spelling(&file, chain.span_id), "data");
    assert!(matches!(value(&file, 1).value, Expr::Select(_)));
    assert_eq!(value(&file, 2).value, Expr::Literal(LiteralKind::Float));
    let Expr::Select(select) = &value(&file, 3).value else {
        panic!("bracket selection")
    };
    assert_eq!(select.index.value, Expr::Literal(LiteralKind::Float));
}

#[test]
fn sequence_and_projection_queries_borrow_written_nodes_and_punctuation() {
    let source = "--- hé 🦀\nvalue: ('hé🦀', source.0.1)\n";
    let file = parser::parse_strict(source).unwrap();
    let Expr::Sequence(sequence) = &value(&file, 0).value else {
        panic!("sequence")
    };
    for (offset, punctuation) in [
        (source.find('(').unwrap(), CollectionPunctuation::Open),
        (source.find(',').unwrap(), CollectionPunctuation::Comma),
        (source.find(')').unwrap(), CollectionPunctuation::Close),
        (source.find(".0").unwrap(), CollectionPunctuation::Dot),
        (source.find(".1").unwrap(), CollectionPunctuation::Dot),
    ] {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(
            hit.node.kind, SyntaxNodeKind::CollectionPunctuation(found) if found == punctuation
        ));
        assert!(hit.ancestors.iter().any(|node| matches!(
            node.kind, SyntaxNodeKind::Sequence(found) if std::ptr::eq(found, sequence.as_ref())
        )));
    }
    for digit in ["0", "1"] {
        let hit = file.node_at_offset(source.find(digit).unwrap()).unwrap();
        assert_eq!(hit.node.span.extract(source), digit);
        assert!(matches!(
            hit.node.kind, SyntaxNodeKind::Expression(expression)
                if expression.value == Expr::Literal(LiteralKind::Integer)
        ));
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::NumericSelect(_)))
        );
    }
}

#[test]
fn multiline_sequences_require_commas_and_keep_later_declarations_on_error() {
    parser::parse_strict("value: (\n    a,\n    b,\n)\nlater: 2\n").unwrap();
    for bad in [
        "(a,,b)",
        "(,)",
        "(a b)",
        "(a\nb)",
        "(a,b; n)",
        "(a; n,b)",
        "a,b",
        "f (x T): return x",
        "f() T,U: return x",
        "data.-1",
        "data.",
    ] {
        let source = format!("value: {bad}\nlater: 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(!output.diagnostics.is_empty(), "{bad}");
        assert!(parser::parse_strict(&source).is_err(), "{bad}");
        assert!(
            output
                .file
                .declaration_outline()
                .any(|declaration| declaration.name == "later"),
            "{bad}"
        );
    }
    for source in [
        "value: (a,\nlater: 2\n",
        "value: (a,\nnext (T,U): pair\nlater: 2\n",
        "value: (data.0,\nlater: 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(!output.diagnostics.is_empty());
        assert!(
            output
                .file
                .declaration_outline()
                .any(|declaration| declaration.name == "later")
        );
    }
}

#[test]
fn unfinished_sequence_elements_are_retained_and_diagnosed() {
    for source in [
        "value: (a, b +)\nlater: 2\n",
        "value: (a, b.0 +)\nlater: 2\n",
        "value: (a, b +, c)\nlater: 2\n",
        "value: [a\nb +, c]\nlater: 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(!output.diagnostics.is_empty(), "{source}");
        assert!(parser::parse_strict(source).is_err(), "{source}");
        assert_eq!(
            output.file.declaration_outline().last().unwrap().name,
            "later"
        );
        let elements = match &value(&output.file, 0).value {
            Expr::Sequence(sequence) => &sequence.elements,
            Expr::Array(array) => &array.elements,
            _ => panic!("collection"),
        };
        assert!(elements.len() >= 2);
        let Expr::Binary { right, .. } = &elements[1].value else {
            panic!("unfinished binary expression is retained")
        };
        assert_eq!(right.value, Expr::Missing);
        assert!(output.file.span_table.get(right.span_id).is_empty());
    }
    let source = "f(x (T,U +)): return x\nlater: 2\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.diagnostics.is_empty());
    assert!(parser::parse_strict(source).is_err());
    let parameter = &binding(&output.file, 0).parameters.as_ref().unwrap().value[0].value;
    let Annotation::Expression(expression) = &parameter.annotation.as_ref().unwrap().value else {
        panic!("annotation")
    };
    let Expr::Sequence(sequence) = &expression.value else {
        panic!("sequence")
    };
    assert_eq!(sequence.elements.len(), 2);
    assert!(
        matches!(&sequence.elements[1].value, Expr::Binary { right, .. }
        if right.value == Expr::Missing)
    );
}

#[test]
fn malformed_sequence_and_projection_combinations_make_bounded_progress() {
    let tokens = ["(", ")", ",", ".", "0", "0.1", ";", "x", "[", "]"];
    for first in tokens {
        for second in tokens {
            for third in tokens {
                let source = format!("value: ({first} {second} {third})\nlater: 2\n");
                let output = parser::parse(&source).unwrap();
                assert_eq!(output.file.text.as_ref(), source);
            }
        }
    }
    for source in [
        "f(): return a,\n",
        "f():\n    while flag do\n        return a,\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}

#[test]
fn bare_return_sequences_share_depth_and_cancellation_budgets() {
    for written in ["(a),b", "((a),b)"] {
        let source = format!("f(): return {written}\n");
        for (maximum, succeeds) in [(2, false), (3, true)] {
            let result = parse_with_options(
                &source,
                ParseOptions {
                    limits: ParseLimits {
                        expression_depth_max: maximum,
                        ..Default::default()
                    },
                    is_cancelled: None,
                },
            );
            if succeeds {
                assert!(!result.unwrap().has_errors());
            } else {
                assert!(matches!(
                    result,
                    Err(ParseFailure::LimitExceeded {
                        limit: ParseLimit::ExpressionDepth,
                        ..
                    })
                ));
            }
        }
    }
    let source = format!("f(): return {}x\n", "x,".repeat(1000));
    let calls = Cell::new(0);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&count),
            ..Default::default()
        },
    )
    .unwrap();
    let total = calls.replace(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() > total - 10
    };
    assert!(matches!(
        parse_with_options(
            &source,
            ParseOptions {
                is_cancelled: Some(&cancel),
                ..Default::default()
            }
        ),
        Err(ParseFailure::Cancelled { .. })
    ));
}

#[test]
fn sequences_and_numeric_chains_obey_depth_and_cancellation_budgets() {
    for (source, limits, expected) in [
        (
            "((x,),)",
            ParseLimits {
                parentheses_max: 1,
                ..Default::default()
            },
            ParseLimit::Parentheses,
        ),
        (
            "((x,),)",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "x.0.1",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
    ] {
        let result = parse_with_options(
            source,
            ParseOptions {
                limits,
                is_cancelled: None,
            },
        );
        assert!(matches!(
            result, Err(ParseFailure::LimitExceeded { limit, .. }) if limit == expected
        ));
    }
    for source in [
        format!("value: ({})\n", "x,".repeat(1000)),
        format!("value: x{}\n", ".0.1".repeat(60)),
    ] {
        let calls = Cell::new(0);
        let count = || {
            calls.set(calls.get() + 1);
            false
        };
        parse_with_options(
            &source,
            ParseOptions {
                is_cancelled: Some(&count),
                ..Default::default()
            },
        )
        .unwrap();
        let total = calls.replace(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > total - 10
        };
        assert!(matches!(
            parse_with_options(
                &source,
                ParseOptions {
                    is_cancelled: Some(&cancel),
                    ..Default::default()
                }
            ),
            Err(ParseFailure::Cancelled { .. })
        ));
    }
}
