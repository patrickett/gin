use std::cell::Cell;

use ast::{
    Annotation, CollectionPunctuation, Expr, Expression, Item, LoopExpression, LoopKeyword,
    SourceFile, SpanId, Statement, SyntaxNodeKind, TypeBody,
};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions, parse_with_options};

fn value(file: &SourceFile, index: usize) -> &Expression {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("binding")
    };
    &binding.value.as_ref().unwrap().expression
}

fn spelling(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn bracket_values_repeated_types_and_allocation_keep_distinct_written_shapes() {
    let source = concat!(
        "Vector is [T; count]\n",
        "items := [first, second,]\n",
        "one := [T]\nempty := []\n",
        "storage := (seed; count + 1)\n",
        "nested := [[1, 2], [3, 4]]\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("alias")
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("written array type")
    };
    let Expr::Array(array) = &expression.value else {
        panic!("array")
    };
    assert_eq!(array.elements.len(), 1);
    assert_eq!(spelling(&file, array.elements[0].span_id), "T");
    let length = array.length.as_ref().unwrap();
    assert_eq!(spelling(&file, length.separator), ";");
    assert_eq!(spelling(&file, length.value.span_id), "count");
    assert_eq!(spelling(&file, array.open), "[");
    assert_eq!(spelling(&file, array.close), "]");
    let Expr::Array(array) = &value(&file, 1).value else {
        panic!("array literal")
    };
    assert!(array.length.is_none());
    assert_eq!(array.elements.len(), 2);
    assert_eq!(array.separators.len(), 2);
    for comma in &array.separators {
        assert_eq!(spelling(&file, *comma), ",");
    }
    for (index, count) in [(2, 1), (3, 0)] {
        let Expr::Array(array) = &value(&file, index).value else {
            panic!("contextual brackets")
        };
        assert_eq!(array.elements.len(), count);
        assert!(array.length.is_none());
    }
    let Expr::Repeat(repeat) = &value(&file, 4).value else {
        panic!("parenthesized allocation, not Group")
    };
    assert_eq!(spelling(&file, repeat.value.span_id), "seed");
    assert_eq!(spelling(&file, repeat.length.span_id), "count + 1");
    assert_eq!(spelling(&file, repeat.open), "(");
    assert_eq!(spelling(&file, repeat.close), ")");
    assert_eq!(spelling(&file, value(&file, 5).span_id), "[[1, 2], [3, 4]]");
}

#[test]
fn array_annotations_work_in_every_existing_header_role() {
    let source = concat!(
        "Holder has\n",
        "    items [ref T; count]\n",
        "    read(self, input ref [T; count]) [T; count]\n",
        "copy(input [T; count]) result [T; count]:\n",
        "    local [T; count]: input\n",
        "    return local\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[1].value else {
        panic!("function")
    };
    assert_eq!(binding.result_binder.as_ref().unwrap().value, "result");
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("array annotation")
    };
    assert!(matches!(expression.value, Expr::Array(_)));
    assert_eq!(spelling(&file, expression.span_id), "[T; count]");
}

#[test]
fn multiline_lists_and_nested_delimiters_do_not_open_body_layout() {
    let source = concat!(
        "main:\n",
        "    values := [\n        1\n        2,\n        f(3)\n    ]\n",
        "    buffer := (0;\n        count + 1\n    )\n",
        "    return values.[\n        0;\n        count\n    ]\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.items.len(), 2);
    let Expr::Block(block) = &value(&file, 0).value else {
        panic!("body")
    };
    assert_eq!(block.statements.len(), 3);
    let Statement::Binding(binding) = &block.statements[0].value else {
        panic!("local")
    };
    let Expr::Array(array) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("list")
    };
    assert_eq!(array.elements.len(), 3);
    assert_eq!(array.separators.len(), 1);
}

#[test]
fn selections_preserve_dot_index_and_optional_written_length() {
    let file = parser::parse_strict(
        "first := data.[offset + 1].field\nslice := data.[offset; count].read()\n",
    )
    .unwrap();
    let Expr::Member { base, .. } = &value(&file, 0).value else {
        panic!("postfix member")
    };
    let Expr::Select(select) = &base.value else {
        panic!("index")
    };
    assert_eq!(spelling(&file, select.dot), ".");
    assert_eq!(spelling(&file, select.index.span_id), "offset + 1");
    assert!(select.length.is_none());
    let Expr::Call { callee, .. } = &value(&file, 1).value else {
        panic!("call")
    };
    let Expr::Member { base, .. } = &callee.value else {
        panic!("member")
    };
    let Expr::Select(select) = &base.value else {
        panic!("slice")
    };
    assert_eq!(spelling(&file, select.index.span_id), "offset");
    assert_eq!(
        spelling(&file, select.length.as_ref().unwrap().value.span_id),
        "count"
    );
}

#[test]
fn inclusive_ranges_keep_to_and_arithmetic_without_synthesized_iteration() {
    let file = parser::parse_strict("bounds := start + 1 to count - 1\n").unwrap();
    let Expr::Range {
        start,
        to_keyword,
        end,
    } = &value(&file, 0).value
    else {
        panic!("range")
    };
    assert_eq!(spelling(&file, start.span_id), "start + 1");
    assert_eq!(spelling(&file, *to_keyword), "to");
    assert_eq!(spelling(&file, end.span_id), "count - 1");
}

#[test]
fn existing_for_and_while_forms_nest_with_case_and_return_bodies() {
    let source = concat!(
        "scan(data [T; count]):\n",
        "    for index in 0 to count - 1 do\n",
        "        while index < count do\n",
        "            result := when index is\n",
        "                0 then data.[index]\n",
        "                1 then data.[index; count]\n",
        "            return result\n",
        "    return data\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(block) = &value(&file, 0).value else {
        panic!("function body")
    };
    assert_eq!(block.statements.len(), 2);
    let Statement::Expression(expression) = &block.statements[0].value else {
        panic!("for statement")
    };
    let Expr::Loop(loop_expression) = &expression.value else {
        panic!("for")
    };
    let LoopExpression::For {
        for_keyword,
        binder,
        in_keyword,
        iterable,
        do_keyword,
        body,
        ..
    } = loop_expression.as_ref()
    else {
        panic!("for")
    };
    assert_eq!(spelling(&file, *for_keyword), "for");
    assert_eq!(binder.value, "index");
    assert_eq!(spelling(&file, *in_keyword), "in");
    assert!(matches!(iterable.value, Expr::Range { .. }));
    assert_eq!(spelling(&file, *do_keyword), "do");
    let Expr::Block(body) = &body.value else {
        panic!("for body")
    };
    let Statement::Expression(expression) = &body.statements[0].value else {
        panic!("while statement")
    };
    let Expr::Loop(loop_expression) = &expression.value else {
        panic!("while")
    };
    let LoopExpression::While {
        while_keyword,
        condition,
        do_keyword,
        ..
    } = loop_expression.as_ref()
    else {
        panic!("while")
    };
    assert_eq!(spelling(&file, *while_keyword), "while");
    assert_eq!(spelling(&file, condition.span_id), "index < count");
    assert_eq!(spelling(&file, *do_keyword), "do");
}

#[test]
fn missing_lengths_indices_and_closes_are_distinct_from_absence() {
    for source in ["x := data.[]", "x := data.[0;]", "x := [T;]", "x := (0;)"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        let expression = value(&output.file, 0);
        let missing = match &expression.value {
            Expr::Select(select) => select
                .length
                .as_ref()
                .map_or(&select.index, |length| &length.value),
            Expr::Array(array) => &array.length.as_ref().unwrap().value,
            Expr::Repeat(repeat) => &repeat.length,
            _ => panic!("collection retained"),
        };
        assert_eq!(missing.value, Expr::Missing);
        assert!(output.file.span_table.get(missing.span_id).is_empty());
    }
    for incomplete in ["[", "[T;", "data.[0;", "(0;", "[f("] {
        let source = format!("x := {incomplete}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert_eq!(
            output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            ["x", "later"],
            "{source}",
        );
        for offset in 0..=source.len() {
            output.file.node_at_offset(offset).unwrap();
        }
    }
}

#[test]
fn unsupported_collection_and_loop_forms_diagnose_and_keep_later_items() {
    for rejected in [
        "x := [a, b; n]\n",
        "x := [a,,b]\n",
        "x := data[0]\n",
        "x := data.[0; 1; 2]\n",
        "x := [in Byte; count]\n",
        "for Pair(a, b) in values do\n    return a\n",
        "while value do\n    break\n",
        "for value in values do\n    continue\n",
        "while value do return value\n",
        "for value in values do return value\n",
        "x := f(when tag is\n    Empty then None\n)\n",
        "x := [when tag is\n    Empty then None\n]\n",
        "x := f(for item in items do\n    return item\n)\n",
    ] {
        let source = format!("{rejected}later := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert_eq!(
            output.file.declaration_outline().last().unwrap().name,
            "later"
        );
    }
}

#[test]
fn missing_loop_keywords_and_bodies_recover_before_sibling_declarations() {
    for source in [
        "while flag\nlater := 2\n",
        "while flag do\nlater := 2\n",
        "for item in items\nlater := 2\n",
        "for item in items do\nlater := 2\n",
        "for item in\nlater := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert_eq!(
            output.file.declaration_outline().last().unwrap().name,
            "later"
        );
        let Item::Expression(expression) = &output.file.items[0].value else {
            panic!("loop retained: {source}")
        };
        let Expr::Loop(loop_expression) = &expression.value else {
            panic!("loop retained")
        };
        let body = match loop_expression.as_ref() {
            LoopExpression::For { body, .. } => body,
            LoopExpression::While { body, .. } => body,
        };
        assert_eq!(body.value, Expr::Missing);
        assert!(output.file.span_table.get(body.span_id).is_empty());
        for offset in 0..=source.len() {
            output.file.node_at_offset(offset).unwrap();
        }
    }
}

#[test]
fn for_and_while_require_do_and_retain_missing_keyword_body_and_later_declarations() {
    for header in ["for item in items", "while flag"] {
        for written in [false, true] {
            let keyword = if written { " do" } else { "" };
            let source =
                format!("{header}{keyword}\n    local := 1\n    return local\nlater := 2\n");
            let output = parser::parse(&source).unwrap();
            assert_eq!(output.has_errors(), !written, "{source}");
            assert_eq!(
                parser::parse_strict(&source).is_ok(),
                written,
                "{source}"
            );
            let Item::Expression(expression) = &output.file.items[0].value else {
                panic!("loop retained")
            };
            let Expr::Loop(loop_expression) = &expression.value else {
                panic!("loop retained")
            };
            let (do_keyword, body) = match loop_expression.as_ref() {
                LoopExpression::For {
                    do_keyword, body, ..
                }
                | LoopExpression::While {
                    do_keyword, body, ..
                } => (*do_keyword, body),
            };
            let span = output.file.span_table.get(do_keyword);
            assert_eq!(span.extract(&source), if written { "do" } else { "" });
            assert_eq!(span.is_empty(), !written);
            let hit = output.file.node_at_offset(span.start()).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::LoopKeyword(LoopKeyword::Do)
            ));
            assert_eq!(hit.node.span, span);
            let Expr::Block(block) = &body.value else {
                panic!("indented body retained with or without do")
            };
            assert_eq!(block.statements.len(), 2);
            assert_eq!(
                output.file.declaration_outline().last().unwrap().name,
                "later",
            );
            if !written {
                assert!(
                    output.diagnostics.iter().any(|diagnostic| {
                        diagnostic.message == "expected 'do' after loop header"
                    })
                );
            }
        }
    }
}

#[test]
fn missing_loop_body_in_case_subject_preserves_the_following_declaration() {
    for source in [
        "bad := when while flag do\nlater := 2\n",
        "bad := when for item in items\nlater := 2\n",
        "bad := when for item in items do\nlater := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert_eq!(
            output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            ["bad", "later"],
            "{source}",
        );
        let Expr::Match(case) = &value(&output.file, 0).value else {
            panic!("case structure retained")
        };
        assert!(output.file.span_table.get(case.is_keyword).is_empty());
        let Expr::Loop(loop_expression) = &case.subject.value else {
            panic!("loop structure retained")
        };
        let body = match loop_expression.as_ref() {
            LoopExpression::For { body, .. } | LoopExpression::While { body, .. } => body,
        };
        assert_eq!(body.value, Expr::Missing);
        for offset in 0..=source.len() {
            output.file.node_at_offset(offset).unwrap();
        }
        assert!(parser::parse_strict(source).is_err());
    }
}

#[test]
fn malformed_collection_and_loop_token_combinations_always_make_progress() {
    let tokens = [
        "[", "]", "(", ")", ";", ",", ".", "for", "while", "in", "do", "to", "x", "1",
    ];
    for first in tokens {
        for second in tokens {
            for third in tokens {
                let source = format!("bad := {first} {second} {third}\nlater := 2\n");
                let output = parser::parse(&source).unwrap();
                for offset in 0..=source.len() {
                    output.file.node_at_offset(offset).unwrap();
                }
                assert_eq!(
                    output
                        .file
                        .declaration_outline()
                        .last()
                        .map(|entry| entry.name),
                    Some("later"),
                    "{source}\n{output:?}"
                );
            }
        }
    }
}

#[test]
fn borrowed_queries_expose_collection_punctuation_keywords_and_utf8_containment() {
    let source = concat!(
        "--- hé 🦀\nmain:\n",
        "    for item in 0 to 1 do\n",
        "        while item < 2 do\n",
        "            return ['hé🦀', data.[item; count]]\n",
    );
    let file = parser::parse_strict(source).unwrap();
    for (spelling, expected) in [
        ("[", CollectionPunctuation::Open),
        (",", CollectionPunctuation::Comma),
        (";", CollectionPunctuation::Semicolon),
        (".", CollectionPunctuation::Dot),
        ("]", CollectionPunctuation::Close),
    ] {
        let hit = file.node_at_offset(source.find(spelling).unwrap()).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::CollectionPunctuation(found) if found == expected)
        );
        assert_eq!(hit.node.span.extract(source), spelling);
    }
    for (spelling, expected) in [
        ("for", LoopKeyword::For),
        ("in", LoopKeyword::In),
        ("while", LoopKeyword::While),
        ("do", LoopKeyword::Do),
    ] {
        let offset = if spelling == "in" {
            source.find(" in ").unwrap() + 1
        } else {
            source.find(spelling).unwrap()
        };
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::LoopKeyword(found) if found == expected));
        assert_eq!(hit.node.span.extract(source), spelling);
    }
    let binder = file.node_at_offset(source.find("item").unwrap()).unwrap();
    assert!(matches!(binder.node.kind, SyntaxNodeKind::LoopBinder(name) if name.value == "item"));
    let range = file.node_at_offset(source.find("to").unwrap()).unwrap();
    assert!(matches!(range.node.kind, SyntaxNodeKind::RangeKeyword));
    for offset in 0..=source.len() {
        if source.is_char_boundary(offset) {
            file.node_at_offset(offset).unwrap();
        } else {
            assert!(file.node_at_offset(offset).is_none());
        }
    }
}

#[test]
fn collection_and_loop_limits_and_late_cancellation_use_existing_budgets() {
    for (source, limits, expected) in [
        (
            "[[x]]",
            ParseLimits {
                parentheses_max: 1,
                ..Default::default()
            },
            ParseLimit::Parentheses,
        ),
        (
            "[[x]]",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "x.[0].[1]",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "for i in xs do\n    while i < n do\n        x\n",
            ParseLimits {
                body_depth_max: 1,
                ..Default::default()
            },
            ParseLimit::BodyDepth,
        ),
        (
            "for i in xs do\n    x\n",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
    ] {
        let failure = parse_with_options(
            source,
            ParseOptions {
                limits,
                is_cancelled: None,
            },
        )
        .unwrap_err();
        assert!(matches!(failure, ParseFailure::LimitExceeded { limit, .. } if limit == expected));
    }
    for source in [
        format!("values := [{}]\n", "1,\n".repeat(1000)),
        format!("for item in items do\n{}", "    f(item)\n".repeat(1000)),
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
        let total = calls.get();
        calls.set(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > total - 20
        };
        assert!(matches!(
            parse_with_options(
                &source,
                ParseOptions {
                    is_cancelled: Some(&cancel),
                    ..Default::default()
                }
            ),
            Err(ParseFailure::Cancelled { .. }),
        ));
    }
}
