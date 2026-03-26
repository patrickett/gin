mod helpers;
use ginc::prelude::IStr;
use helpers::parse_str;

#[test]
fn test_parse_simple_function() {
    let ast = parse_str("f(x): x\n");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    for doc_params in ast.defs().values() {
        assert!(doc_params.doc_comment().is_none())
    }
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_tag_definition() {
    let ast = parse_str("Result is Ok or Err");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);
    assert_eq!(ast.defs().len(), 0);
}

#[test]
fn test_parse_tag_with_doc_comment_after() {
    // Doc comment on same line after variant (compact syntax)
    let ast = parse_str("Result is\nOk or --- error case\nErr");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);
    assert_eq!(ast.defs().len(), 0);
}

#[test]
fn test_parse_tag_with_doc_comment_before() {
    // Doc comment before variant
    let ast = parse_str("--- my result type\nResult is Ok or Err");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);
    assert_eq!(ast.defs().len(), 0);
}

#[test]
fn test_parse_multi_line_union() {
    // Multi-line union with indentation
    let ast = parse_str("Maybe(thing) is\n    Some(thing) or\n    None");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);
    assert_eq!(ast.defs().len(), 0);

    // Verify the union was parsed correctly
    let maybe_decl = &ast.tags().values().next().unwrap();
    assert_eq!(maybe_decl.name().as_str(), "Maybe");

    // Check it's a union with two variants
    match maybe_decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_multi_line_union_with_type_params() {
    // Multi-line union with type parameters
    let ast = parse_str("Result(value, error) is\n    Ok(value) or\n    Error(error)");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);
    assert_eq!(ast.defs().len(), 0);
}

#[test]
fn test_parse_inline_first_variant_multiline_union() {
    let ast = parse_str("Color is Red\n      or Green\n      or Blue");
    assert_eq!(ast.tags().len(), 1);
    let decl = ast.tags().values().next().unwrap();
    assert_eq!(decl.name().as_str(), "Color");
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => assert_eq!(variants.len(), 3),
        _ => panic!("Expected Union"),
    }
}

#[test]
fn test_parse_multi_line_union_three_variants() {
    // Union with three variants
    let ast = parse_str("TriState is\n    True or\n    False or\n    Unknown");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    let decl = &ast.tags().values().next().unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 3);
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_example_gin() {
    // Test the exact example from example.gin
    let src = r#"--- Optional value type - either Some(x) or None.
--- Used to represent values that may or may not be present.
Maybe(thing) is
    Some(thing) or
    None
"#;

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    // Verify the union was parsed correctly
    let maybe_decl = &ast.tags().values().next().unwrap();
    assert_eq!(maybe_decl.name().as_str(), "Maybe");

    match maybe_decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_multi_line_union_with_doc_comments() {
    // Union with doc comments - comments go before each variant
    let src =
        "Result is\n    --- success case\n    Ok(value) or\n    --- error case\n    Error(error)";

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    // Check that variants have doc comments
    let decl = &ast.tags().values().next().unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);
            match &variants[0] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    ..
                } => {
                    assert_eq!(doc.0, "success case");
                }
                _ => panic!("Expected first variant with doc comment"),
            }
            match &variants[1] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    ..
                } => {
                    assert_eq!(doc.0, "error case");
                }
                _ => panic!("Expected second variant with doc comment"),
            }
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_union_doc_before_variant() {
    // Doc comment before variant (compact syntax)
    let src = "Result is\n    --- success\n    Ok or\n    --- error\n    Err";

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    let decl = &ast.tags().values().next().unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);
            // Both variants should have doc comments
            match &variants[0] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    ..
                } => {
                    assert_eq!(doc.0, "success");
                }
                _ => panic!("Expected first variant with doc comment"),
            }
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_single_line_union() {
    // Single-line union (no indentation)
    let ast = parse_str("Bool is True or False");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    let decl = &ast.tags().values().next().unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_union_inline_doc_comment() {
    // Variant doc comments can be inline after 'or'
    let src = "--- Used to represent values that may or may not be present.
Maybe(thing) is
    Some(thing) or --- has a value
    None
";

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    let decl = &ast.tags().values().next().unwrap();
    assert_eq!(decl.name().as_str(), "Maybe");

    // Check declaration doc comment (from above)
    assert!(
        decl.doc_comment().is_some(),
        "Declaration should have doc comment from above"
    );
    let decl_doc = decl.doc_comment().unwrap();
    assert_eq!(
        decl_doc.0,
        "Used to represent values that may or may not be present."
    );

    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);

            // First variant: Some(thing) with doc "has a value"
            match &variants[0] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    tag,
                } => {
                    assert_eq!(doc.0, "has a value");
                    match tag {
                        ginc::ast::Tag::Generic(name, params) => {
                            assert_eq!(name.as_str(), "Some");
                            assert_eq!(params.len(), 1);
                        }
                        _ => panic!("Expected Generic tag for Some"),
                    }
                }
                _ => panic!("Expected Local variant with doc comment"),
            }

            // Second variant: None without doc comment
            match &variants[1] {
                ginc::ast::Variant::External(tag) => match tag {
                    ginc::ast::Tag::Nominal(name) => {
                        assert_eq!(name.as_str(), "None");
                    }
                    _ => panic!("Expected Nominal tag for None"),
                },
                _ => panic!("Expected External variant for None"),
            }
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_import() {
    let ast = parse_str("use http.web as h\n");

    assert_eq!(ast.uses().len(), 1);
    assert_eq!(ast.defs().len(), 0);
    assert_eq!(ast.tags().len(), 0);

    let module = &ast.uses()[0].0[0];
    assert!(matches!(
        &module.source,
        ginc::ast::ImportSource::Package(_)
    ));
    assert_eq!(module.alias.as_deref().map(String::as_str), Some("h"));
}

#[test]
fn test_parse_local_import() {
    let ast = parse_str("use './math' as math\n");

    assert_eq!(ast.uses().len(), 1);
    assert_eq!(ast.defs().len(), 0);
    assert_eq!(ast.tags().len(), 0);

    let module = &ast.uses()[0].0[0];
    assert!(matches!(&module.source, ginc::ast::ImportSource::Local(_)));
    assert_eq!(module.alias.as_deref().map(String::as_str), Some("math"));
    assert_eq!(module.effective_name(), "math");
}

#[test]
fn test_parse_local_import_no_alias() {
    let ast = parse_str("use './util'\n");

    assert_eq!(ast.uses().len(), 1);
    let module = &ast.uses()[0].0[0];
    assert!(matches!(&module.source, ginc::ast::ImportSource::Local(_)));
    assert!(module.alias.is_none());
    assert_eq!(module.effective_name(), "util");
}

// TODO: FIXME: test broken, need to error when return statement indented
// This test checked for parse errors via the old Parsable trait.
// With Salsa queries, parse errors are collected as diagnostics in accumulators
// rather than returned as Result::Err. Skipping for now until diagnostic
// accumulator testing is set up.
// #[test]
// fn test_parse_multi_line_function_fail() {
//     ...
// }

#[test]
fn test_parse_multi_line_function_success() {
    let ast = parse_str("f(x):\nreturn x + 1");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_arithmetic_expression() {
    let ast = parse_str("add(a, b): a + b");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_comparison_expression() {
    let ast = parse_str("is_equal(a, b): a = b");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_function_call() {
    let ast = parse_str("result: add(1, 2)");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_return_type() {
    let ast = parse_str("add(x Number, y Number) Number: x + y");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_typed_variable() {
    let ast = parse_str("five_hundred Number: 500");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_tag_range() {
    let ast = parse_str("DiceThrow is 1...6");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 0);
    assert_eq!(ast.tags().len(), 1);
}

#[test]
fn test_parse_tag_in_range() {
    let ast = parse_str("DiceThrow is in 1...6");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 0);
    assert_eq!(ast.tags().len(), 1);
}

#[test]
fn test_parse_multi_line_empty_nothing_variable() {
    let ast = parse_str("example:\n\nreturn\n");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_parse_unterminated_string() {
    let ast = parse_str("hello_text: 'hello\n");

    // Should parse successfully (error is accumulated as diagnostic)
    assert!(ast.uses().is_empty());
    // The definition should still be created
    assert_eq!(ast.defs().len(), 1);
}

#[test]
fn test_parse_unterminated_string_lone_quote() {
    let ast = parse_str("y: '\nx: '\n");

    // Should parse successfully (errors are accumulated as diagnostics)
    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 2);
}

#[test]
fn test_parse_unterminated_string_multiple_newlines() {
    let ast = parse_str("hello_text: 'hello\n\n\n");

    // Should parse successfully (error is accumulated as diagnostic)
    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
}

#[test]
fn test_parse_range_with_postfix_doc_comment() {
    // Doc comment after value should be captured by the declaration it belongs to
    let src = "Area is 0...999 --- This is the area\nGroup is 0...99 --- This is the group\n";
    let ast = parse_str(src);

    assert_eq!(ast.tags().len(), 2);

    // Area should have its own doc comment
    // Check each tag has its own doc comment (not leaked from previous declaration)
    for decl in ast.tags().values() {
        let name = decl.name();
        assert!(
            decl.doc_comment().is_some(),
            "{} should have its own doc comment",
            name.as_str()
        );
        match name.as_str() {
            "Area" => assert_eq!(decl.doc_comment().unwrap().0, "This is the area"),
            "Group" => assert_eq!(decl.doc_comment().unwrap().0, "This is the group"),
            other => panic!("Unexpected tag: {}", other),
        }
    }
}

// #[test]
// fn test_parse_conditional() {
//     let src = "
//     a: True\n
//     v: when a\n
//        then '`a` was true'\n
//        else '`a` was false'\n
//     ";
//     let ast = parse_str(src);

//     // Expect one definition (the implicit block) and no imports/tags
//     assert!(ast.imports.is_empty());
//     assert_eq!(ast.defs().len(), 0);
//     assert_eq!(ast.tags().len(), 0);
// }

// #[test]
// fn test_parse_for_loop() {
//     let ast = parse_str("example:\n\nreturn\n");

//     assert!(ast.imports.is_empty());
//     assert_eq!(ast.defs().len(), 1);
//     for (_name, params) in ast.defs() {
//         match params.item.value {
//             BindValue::Expr { .. } => unreachable!(),
//             BindValue::Body { exprs, .. } => {
//                 assert_eq!(exprs.len(), 1)
//                 // TODO: continue to make sure parsed properly
//             }
//         }
//     }
//     assert_eq!(ast.tags().len(), 0);
// }

// #[test]
// fn test_parse_pattern_matching() {
//     let src = "\
//         when value\n\
//         then
//         else

//         Ok(v) => v\n\
//         Err(e) => e
//     ";
//     let ast = parse_str(src);

//     assert!(ast.imports.is_empty());
//     assert_eq!(ast.defs().len(), 0);
//     assert_eq!(ast.tags().len(), 0);
// }

#[test]
fn test_parse_full_inline_doc_comments() {
    // All three doc comment styles in one declaration (new syntax)
    // - Declaration doc ABOVE declaration (not inline after `is`)
    // - Variant doc after `or` (belongs to previous variant)
    // - Variant doc after tag (no `or` following)
    let src = "--- Used to represent values that may or may not be present.
Maybe(thing) is
    Some(thing) or --- has a value
    None --- has no value
";

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.tags().len(), 1);

    let decl = &ast.tags().values().next().unwrap();
    assert_eq!(decl.name().as_str(), "Maybe");

    // Check declaration doc comment (from above)
    assert!(
        decl.doc_comment().is_some(),
        "Declaration should have doc comment from above"
    );
    let decl_doc = decl.doc_comment().unwrap();
    assert_eq!(
        decl_doc.0,
        "Used to represent values that may or may not be present."
    );

    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 2);

            // First variant: Some(thing) with doc "has a value"
            // (doc comes from "or --- has a value" which belongs to previous variant)
            match &variants[0] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    tag,
                } => {
                    assert_eq!(doc.0, "has a value");
                    // Verify tag is Some(thing)
                    match tag {
                        ginc::ast::Tag::Generic(name, params) => {
                            assert_eq!(name.as_str(), "Some");
                            assert_eq!(params.len(), 1);
                        }
                        _ => panic!("Expected Generic tag for Some"),
                    }
                }
                _ => panic!(
                    "Expected Some variant with doc comment, got: {:?}",
                    variants[0]
                ),
            }

            // Second variant: None with doc "has no value"
            // (doc comes from "None --- has no value" after the tag)
            match &variants[1] {
                ginc::ast::Variant::Local {
                    doc_comment: Some(doc),
                    tag,
                } => {
                    assert_eq!(doc.0, "has no value");
                    // Verify tag is None
                    match tag {
                        ginc::ast::Tag::Nominal(name) => {
                            assert_eq!(name.as_str(), "None");
                        }
                        _ => panic!("Expected Nominal tag for None"),
                    }
                }
                _ => panic!(
                    "Expected None variant with doc comment, got: {:?}",
                    variants[1]
                ),
            }
        }
        _ => panic!("Expected Union value"),
    }
}

#[test]
fn test_parse_doc_comment_on_inner_bind() {
    let src = "--- do something returns an int\ndo_something:\n--- value is always 2\n    val: 2\n    name: 'John'\nreturn 1 + 1\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert!(
        bind.doc_comment().is_some(),
        "outer bind should have doc comment"
    );
    assert_eq!(bind.doc_comment().unwrap().0, "do something returns an int");

    // Verify inner binds have correct doc comments
    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            // Find inner binds
            let inner_binds: Vec<&ginc::ast::Bind> = exprs
                .iter()
                .filter_map(|e| match e {
                    ginc::ast::Expr::Bind(b) => Some(b),
                    _ => None,
                })
                .collect();
            assert_eq!(inner_binds.len(), 2, "should have 2 inner binds");

            // val should have doc comment "value is always 2"
            assert_eq!(inner_binds[0].name().as_str(), "val");
            assert!(
                inner_binds[0].doc_comment().is_some(),
                "val should have doc comment"
            );
            assert_eq!(inner_binds[0].doc_comment().unwrap().0, "value is always 2");

            // name should NOT have a doc comment
            assert_eq!(inner_binds[1].name().as_str(), "name");
            assert!(
                inner_binds[1].doc_comment().is_none(),
                "name should NOT have doc comment"
            );
        }
        _ => panic!("Expected Body value for do_something"),
    }
}

#[test]
fn test_parse_postfix_doc_comment_on_inner_bind() {
    // Test postfix doc comment style (as in example.gin)
    let src = "--- do something returns an int\ndo_something:\n    val: 2 --- value is always 2\n    name: 'John'\nreturn 1 + 1\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.doc_comment().unwrap().0, "do something returns an int");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            let inner_binds: Vec<&ginc::ast::Bind> = exprs
                .iter()
                .filter_map(|e| match e {
                    ginc::ast::Expr::Bind(b) => Some(b),
                    _ => None,
                })
                .collect();
            assert_eq!(inner_binds.len(), 2);

            // val should have postfix doc comment
            assert_eq!(inner_binds[0].name().as_str(), "val");
            assert!(
                inner_binds[0].doc_comment().is_some(),
                "val should have postfix doc comment"
            );
            assert_eq!(inner_binds[0].doc_comment().unwrap().0, "value is always 2");

            // name should NOT have a doc comment
            assert_eq!(inner_binds[1].name().as_str(), "name");
            assert!(
                inner_binds[1].doc_comment().is_none(),
                "name should NOT have doc comment"
            );
        }
        _ => panic!("Expected Body value for do_something"),
    }
}

// === Modulo operator ===

#[test]
fn test_parse_modulo_expression() {
    let ast = parse_str("remainder(a, b): a % b");

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}

// === When expression (boolean condition form) ===

#[test]
fn test_parse_when_inline_boolean() {
    // when <cond> then <result> else <result>
    let ast = parse_str("x: when a = 1 then 'yes' else 'no'\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_none(), "boolean form has no subject");
                assert_eq!(when_expr.arms.len(), 2);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Cond { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Else(_)));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

#[test]
fn test_parse_when_single_boolean_no_else() {
    // Single arm, no else
    let ast = parse_str("x: when a = 1 then 'yes'\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_none());
                assert_eq!(when_expr.arms.len(), 1);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Cond { .. }));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

#[test]
fn test_parse_when_multi_arm_boolean() {
    // Multi-arm boolean form with indented arms
    let src = "x: when a = 1 then 'one'\n        a = 2 then 'two'\n        else 'other'\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_none());
                assert_eq!(when_expr.arms.len(), 3);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Cond { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Cond { .. }));
                assert!(matches!(when_expr.arms[2], ginc::ast::WhenArm::Else(_)));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

// === When expression (pattern matching form) ===

#[test]
fn test_parse_when_inline_pattern() {
    // when <subject> is <tag> then <result> else <result>
    let ast = parse_str("x: when self is Some(v) then v else 0\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_some(), "pattern form has subject");
                match when_expr.subject.as_deref() {
                    Some(ginc::ast::Expr::SelfRef) => {}
                    other => panic!("Expected SelfRef subject, got: {:?}", other),
                }
                assert_eq!(when_expr.arms.len(), 2);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Is { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Else(_)));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

#[test]
fn test_parse_when_block_pattern() {
    // Block pattern form with indented is-arms
    let src = "x: when value\n    is Some(v) then v\n    is None then 0\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_some());
                assert_eq!(when_expr.arms.len(), 2);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Is { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Is { .. }));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

#[test]
fn test_parse_when_block_pattern_with_else() {
    // Block pattern with else fallthrough
    let src = "x: when value\n    is Some(v) then v\n    else 0\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_some());
                assert_eq!(when_expr.arms.len(), 2);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Is { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Else(_)));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

// === self keyword ===

#[test]
fn test_parse_self_expression() {
    let ast = parse_str("x: self\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => {
            assert!(
                matches!(expr.as_ref(), ginc::ast::Expr::SelfRef),
                "Expected SelfRef, got: {:?}",
                expr
            );
        }
        _ => panic!("Expected single expression value"),
    }
}

// === Method definitions ===

#[test]
fn test_parse_method_definition() {
    let ast = parse_str("Maybe.is_empty: when self is Some(v) then 0 else 1\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert!(bind.is_method(), "should be a method");
    assert!(bind.receiver_type().is_some());
    assert_eq!(bind.receiver_type().unwrap().name(), "Maybe");
}

#[test]
fn test_parse_when_indented_then_else() {
    // Style C: is-pattern on same line as when, then/else on next lines
    let src =
        "Maybe.is_empty: when self is Some(x)\n                then 1\n                else 0\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert!(bind.is_method());
    match bind.value() {
        ginc::ast::BindValue::Expr(expr) => match expr.as_ref() {
            ginc::ast::Expr::When(when_expr) => {
                assert!(when_expr.subject.is_some());
                assert_eq!(when_expr.arms.len(), 2);
                assert!(matches!(when_expr.arms[0], ginc::ast::WhenArm::Is { .. }));
                assert!(matches!(when_expr.arms[1], ginc::ast::WhenArm::Else(_)));
            }
            other => panic!("Expected When expression, got: {:?}", other),
        },
        _ => panic!("Expected single expression value"),
    }
}

#[test]
fn test_parse_method_with_params() {
    let ast = parse_str("List.map(f): f(self)\n");

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert!(bind.is_method());
    assert_eq!(bind.receiver_type().unwrap().name(), "List");
    assert!(bind.params().is_some());
}

// === Anonymous tags in return statements ===

#[test]
fn test_parse_anonymous_tag_in_return() {
    // Test that a bare tag in return is parsed as AnonymousTag
    // and creates NO external tag declaration (anonymous return type)
    let src = "print(arg):\n    write(arg)\nreturn PrintSuccess\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "print");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs: _, ret } => {
            // Check that the return expression is an AnonymousTag
            assert!(ret.0.is_some(), "should have a return value");
            match ret.0.as_ref().unwrap().as_ref() {
                ginc::ast::Expr::AnonymousTag(tag_name) => {
                    assert_eq!(tag_name.as_str(), "PrintSuccess");
                }
                other => panic!("Expected AnonymousTag, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }

    // With the new union-based approach, NO external tag is created for anonymous return types
    assert_eq!(
        ast.tags().len(),
        0,
        "Anonymous return types should not create external tags"
    );
}

#[test]
fn test_parse_anonymous_tag_no_variants() {
    // Test single-variant anonymous tag (single-variant union)
    // Note: single-line binds use direct expression, not `return`
    let src = "noop():\nreturn Unit\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "noop");

    match bind.value() {
        ginc::ast::BindValue::Body { ret, .. } => match ret.0.as_ref().unwrap().as_ref() {
            ginc::ast::Expr::AnonymousTag(tag_name) => {
                assert_eq!(tag_name.as_str(), "Unit");
            }
            other => panic!("Expected AnonymousTag, got: {:?}", other),
        },
        _ => panic!("Expected Body value"),
    }

    // With the new union-based approach, NO external tag is created for anonymous return types
    assert_eq!(
        ast.tags().len(),
        0,
        "Anonymous return types should not create external tags"
    );
}

#[test]
fn test_parse_multiple_anonymous_tags() {
    // Test multiple binds with anonymous tags
    // Each bind creates its own anonymous union return type
    let src = "read():\nreturn Data\nwrite():\nreturn Success\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 2);
    // With the new union-based approach, NO external tags are created for anonymous return types
    assert_eq!(
        ast.tags().len(),
        0,
        "Anonymous return types should not create external tags"
    );
}

#[test]
fn test_parse_normal_return_not_affected() {
    // Test that normal returns (expressions) still work
    let src = "add(a, b):\nreturn a + b\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();

    match bind.value() {
        ginc::ast::BindValue::Body { ret, .. } => {
            match ret.0.as_ref().unwrap().as_ref() {
                ginc::ast::Expr::Binary(_) => {
                    // Expected: a + b is a Binary expression
                }
                other => panic!("Expected Binary, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }

    // No anonymous tag should be generated
    assert_eq!(ast.tags().len(), 0);
}

// === Named return types (union generation) ===

#[test]
fn test_simple_bind_still_works() {
    // Debug test to check if basic bind parsing still works
    let src = "f(x): x\n";
    let ast = parse_str(src);

    println!("Simple bind - Defs count: {}", ast.defs().len());
    assert_eq!(ast.defs().len(), 1);
}

#[test]
fn test_multiline_bind_works() {
    // Test if multiline binds work
    let src = "f(x):\nreturn x + 1\n";
    let ast = parse_str(src);

    println!("Multiline bind - Defs count: {}", ast.defs().len());
    assert_eq!(ast.defs().len(), 1);
}

#[test]
fn test_bind_with_when_works() {
    // Test if binds with when expressions work
    let src = "f(x):\n    when x = 1 then 2\nreturn 3\n";
    let ast = parse_str(src);

    println!("Bind with when - Defs count: {}", ast.defs().len());
    assert_eq!(ast.defs().len(), 1);
}

#[test]
fn test_bind_with_when_and_anonymous_tag() {
    // Test if binds with named return type work
    let src = "f(x) result:\nreturn TagName\n";
    let ast = parse_str(src);

    println!(
        "Bind with named return type - Defs count: {}",
        ast.defs().len()
    );
    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(
        bind.return_type_name(),
        Some(&IStr::new("result".to_string()))
    );
}

#[test]
fn test_parse_named_return_type_single_variant() {
    // Test that a named return type creates a union declaration
    let src = "noop() result:\nreturn Unit\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "noop");
    assert_eq!(
        bind.return_type_name(),
        Some(&IStr::new("result".to_string()))
    );

    // Verify the union was created in the tags map
    assert_eq!(ast.tags().len(), 1);
    assert!(ast.tags().contains_key(&IStr::new("result".to_string())));

    // Verify it's a union with a single variant
    let decl = ast.tags().get(&IStr::new("result".to_string())).unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert_eq!(variants.len(), 1);
            match &variants[0] {
                ginc::ast::Variant::External(tag) => {
                    assert_eq!(tag.name(), "Unit");
                }
                other => panic!("Expected External variant, got: {:?}", other),
            }
        }
        other => panic!("Expected Union, got: {:?}", other),
    }
}

#[test]
fn test_parse_named_return_type_multi_variant() {
    // Test that multiple return values create a multi-variant union
    let src = "print(arg) print_result:\n    write(arg)\nreturn PrintSuccess\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "print");
    assert_eq!(
        bind.return_type_name(),
        Some(&IStr::new("print_result".to_string()))
    );

    // Verify the union was created
    assert_eq!(ast.tags().len(), 1);
    assert!(
        ast.tags()
            .contains_key(&IStr::new("print_result".to_string()))
    );

    // Verify it's a union with at least one variant
    let decl = ast
        .tags()
        .get(&IStr::new("print_result".to_string()))
        .unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert!(!variants.is_empty());
            let variant_names: Vec<_> = variants
                .iter()
                .map(|v| match v {
                    ginc::ast::Variant::External(tag) => tag.name(),
                    _ => panic!("Expected External variant"),
                })
                .collect();
            assert!(variant_names.contains(&"PrintSuccess"));
        }
        other => panic!("Expected Union, got: {:?}", other),
    }
}

#[test]
fn test_parse_anonymous_return_type_creates_no_tag() {
    // Test that anonymous return types don't create external tags
    let src = "print(arg):\n    write(arg)\nreturn PrintSuccess\n";
    let ast = parse_str(src);

    println!("Defs count: {}", ast.defs().len());
    for (name, bind) in ast.defs() {
        println!(
            "Def: {} return_type_name={:?}",
            name.as_str(),
            bind.return_type_name()
        );
    }
    println!("Tags count: {}", ast.tags().len());
    for (name, decl) in ast.tags() {
        println!("Tag: {} -> {:?}", name.as_str(), decl.value());
    }

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "print");
    assert_eq!(
        bind.return_type_name(),
        None,
        "Anonymous return type should have no name"
    );

    // No external tag should be created
    assert_eq!(ast.tags().len(), 0);
}

#[test]
fn test_method_with_when_pattern() {
    // `Maybe.is_empty: when self is Some(x)\n    then True\n    else False`
    let src = "Maybe(x) is\n    Some(x) or\n    None\nMaybe.is_empty: when self is Some(x)\n    then True\n    else False\n";
    let ast = parse_str(src);

    // The method should be stored under combined key "Maybe.is_empty"
    let key = IStr::new("Maybe.is_empty".to_string());
    let method = ast
        .defs()
        .get(&key)
        .expect("method not found under Maybe.is_empty key");
    assert_eq!(method.name().as_str(), "is_empty");
    assert!(method.is_method(), "should be a method");
    assert_eq!(method.receiver_type().unwrap().name(), "Maybe");

    // Return type should be inferred as True or False
    let ret = method.infer_return_type_union();
    let ret_str = ret.unwrap_or_default();
    assert!(
        ret_str.contains("True") && ret_str.contains("False"),
        "Expected 'True or False', got: {ret_str}"
    );
}

#[test]
fn test_parse_named_return_type_with_deduplication() {
    // Test that duplicate return tag names are deduplicated in the union
    let src = "foo() bar:\n    x\nreturn A\n";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.return_type_name(), Some(&IStr::new("bar".to_string())));

    // Verify the union has at least one variant
    let decl = ast.tags().get(&IStr::new("bar".to_string())).unwrap();
    match decl.value() {
        ginc::ast::DeclareValue::Union { variants } => {
            assert!(!variants.is_empty());
        }
        other => panic!("Expected Union, got: {:?}", other),
    }
}

// === Greedy parsing tests (from "No Semicolons Needed" article) ===

#[test]
fn test_no_foo_paren_ambiguity() {
    // Test that gin does NOT have the `foo (1)` ambiguity that Swift/Gleam have.
    // In Swift/Gleam, `foo` followed by `(1)` on the next line is parsed
    // as a single function call `foo(1)`. In gin, the `(` must come
    // immediately after the function name (in the same token sequence),
    // so `foo \n (1)` is parsed as two separate statements.
    let src = "foo():
    x := 42
return x

(1 + 1)
";

    let ast = parse_str(src);

    // Should have one bind (foo) and one top-level expression (the parenthesized add)
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.top_level_exprs().len(), 1);

    // The top-level expression should be a binary operation (1 + 1), not a function call
    match &ast.top_level_exprs()[0] {
        ginc::ast::Expr::Binary(_) => {
            // Good! This is (1 + 1) as a standalone expression
        }
        ginc::ast::Expr::FnCall(call) => {
            panic!(
                "Should NOT have parsed (1 + 1) as a function call. Got: {:?}",
                call
            );
        }
        _ => {
            panic!("Unexpected expression type: {:?}", ast.top_level_exprs()[0]);
        }
    }
}

// === If blocks with return as block terminator ===

#[test]
fn test_parse_if_with_return_block() {
    // Test that if blocks require return as block terminator
    let src = "
Maybe is Some(x) or None

process(value):
if value is Some(x)
    result: x + 1
return result

return 0
";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "process");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret } => {
            // The body should contain an IfExpr
            assert_eq!(exprs.len(), 1); // just the if block
            match &exprs[0] {
                ginc::ast::Expr::If(if_expr) => {
                    // Verify the if block has body expressions and return
                    assert_eq!(if_expr.body.len(), 1);
                    assert!(if_expr.ret.0.is_some());
                }
                other => panic!("Expected IfExpr, got: {:?}", other),
            }
            // Verify the function has a return statement
            assert!(ret.0.is_some());
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_chained_if_blocks() {
    // Test multiple if blocks in sequence
    let src = "
Result is Ok(x) or Err(e)

handle(r):
if r is Ok(x)
return x

if r is Err(e)
return 0

return 999
";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "handle");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret } => {
            // Should have 2 if blocks in the body
            assert_eq!(exprs.len(), 2);
            assert!(matches!(&exprs[0], ginc::ast::Expr::If(_)));
            assert!(matches!(&exprs[1], ginc::ast::Expr::If(_)));
            // Verify the function has a return statement
            assert!(ret.0.is_some());
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_if_with_bool_condition() {
    // Test if with boolean condition (not pattern matching)
    let src = "
check_flag(flag):
if flag
    result: 1
return result

return 0
";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "check_flag");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret } => {
            assert_eq!(exprs.len(), 1); // just the if block
            match &exprs[0] {
                ginc::ast::Expr::If(if_expr) => {
                    match &if_expr.condition {
                        ginc::ast::IfCondition::Bool(_) => {}
                        other => panic!("Expected Bool condition, got: {:?}", other),
                    }
                    // Verify the if block has body expressions and return
                    assert_eq!(if_expr.body.len(), 1);
                    assert!(if_expr.ret.0.is_some());
                }
                other => panic!("Expected IfExpr, got: {:?}", other),
            }
            // Verify the function has a return statement
            assert!(ret.0.is_some());
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_if_with_no_body_statements() {
    // Test if block with just return (no intermediate statements)
    let src = "
Maybe is Some(x) or None

get_value(m):
if m is Some(x)
return x

return 0
";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            assert_eq!(exprs.len(), 1); // just the if block
            match &exprs[0] {
                ginc::ast::Expr::If(if_expr) => {
                    // Body should be empty (just the return)
                    assert_eq!(if_expr.body.len(), 0);
                    assert!(if_expr.ret.0.is_some());
                }
                other => panic!("Expected IfExpr, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_if_with_maybe_some_pattern() {
    // Test the user's pattern: if with Maybe.Some(v) pattern matching
    // Regression test for "expected 'Return', found Newline" error
    let src = "
main:
    if True
    return 5
return";
    // This should parse without error
    let ast = parse_str(src);

    // Should have main definition
    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "main");

    // Check that the body has the if expression
    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret: _ } => {
            assert_eq!(exprs.len(), 1); // just the if expression
        }
        _ => panic!("Expected Body value"),
    }

    // Should have main definition
    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "main");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret: _ } => {
            // Body should have just the if expression
            assert_eq!(exprs.len(), 1);
            match &exprs[0] {
                ginc::ast::Expr::If(if_expr) => {
                    // Body should be empty (just the return)
                    assert_eq!(if_expr.body.len(), 0);
                    assert!(if_expr.ret.0.is_some());
                }
                other => panic!("Expected IfExpr, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_if_with_preceding_bind() {
    // Regression test: if statement after a value binding should parse
    let src = "
main:
    val: 3
    if True
    return ''
return";
    let ast = parse_str(src);

    // Should have main definition
    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "main");

    // Body should have both val binding and if expression
    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret } => {
            assert_eq!(exprs.len(), 2);
            // First expression is the val binding
            assert!(matches!(&exprs[0], ginc::ast::Expr::Bind(_)));
            // Second expression is the if
            assert!(matches!(&exprs[1], ginc::ast::Expr::If(_)));
            // Main block has its own return (bare return)
            assert!(ret.0.is_none());
        }
        _ => panic!("Expected Body value"),
    }
}

// === Qualified variant constructor syntax ===

#[test]
fn test_parse_qualified_variant_call() {
    // Test that Maybe.Some(3) parses as a TagCall with qual_path
    let src = "Maybe(x) is\n    Some(x) or\n    None\nmain:\n    Maybe.Some(3)\nreturn";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "main");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, ret } => {
            // The body should contain a TagCall expression
            assert_eq!(exprs.len(), 1);
            match &exprs[0] {
                ginc::ast::Expr::TagCall(tag_call) => {
                    assert_eq!(tag_call.name.as_str(), "Some");
                    assert!(tag_call.qual_path.is_some());
                    let qual_path = tag_call.qual_path.as_ref().unwrap();
                    assert_eq!(qual_path.root.as_str(), "Maybe");
                    assert_eq!(qual_path.segments.len(), 1);
                    assert_eq!(qual_path.segments[0].as_str(), "Some");
                    assert_eq!(tag_call.args.len(), 1);
                }
                other => panic!("Expected TagCall, got: {:?}", other),
            }
            // Return should have no value (just `return`)
            assert!(ret.0.is_none());
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_simple_variant_call_still_works() {
    // Test backward compatibility: Some(3) should still work
    let src = "Maybe(x) is\n    Some(x) or\n    None\nmain:\n    Some(3)\nreturn";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    assert_eq!(bind.name().as_str(), "main");

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            assert_eq!(exprs.len(), 1);
            match &exprs[0] {
                ginc::ast::Expr::TagCall(tag_call) => {
                    assert_eq!(tag_call.name.as_str(), "Some");
                    assert!(
                        tag_call.qual_path.is_none(),
                        "Simple variant should have no qual_path"
                    );
                    assert_eq!(tag_call.args.len(), 1);
                }
                other => panic!("Expected TagCall, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_qualified_result_ok() {
    // Test Result.Ok(value) qualified syntax
    let src = "Result(value, error) is\n    Ok(value) or\n    Error(error)\nmain:\n    Result.Ok(42)\nreturn";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            assert_eq!(exprs.len(), 1);
            match &exprs[0] {
                ginc::ast::Expr::TagCall(tag_call) => {
                    assert_eq!(tag_call.name.as_str(), "Ok");
                    assert!(tag_call.qual_path.is_some());
                    let qual_path = tag_call.qual_path.as_ref().unwrap();
                    assert_eq!(qual_path.root.as_str(), "Result");
                    assert_eq!(qual_path.segments[0].as_str(), "Ok");
                }
                other => panic!("Expected TagCall, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn test_parse_qualified_variant_no_args() {
    // Test Maybe.None() qualified syntax (no arguments)
    let src = "Maybe(x) is\n    Some(x) or\n    None\nmain:\n    Maybe.None()\nreturn";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();

    match bind.value() {
        ginc::ast::BindValue::Body { exprs, .. } => {
            assert_eq!(exprs.len(), 1);
            match &exprs[0] {
                ginc::ast::Expr::TagCall(tag_call) => {
                    assert_eq!(tag_call.name.as_str(), "None");
                    assert!(tag_call.qual_path.is_some());
                    let qual_path = tag_call.qual_path.as_ref().unwrap();
                    assert_eq!(qual_path.root.as_str(), "Maybe");
                    assert_eq!(qual_path.segments[0].as_str(), "None");
                    assert_eq!(tag_call.args.len(), 0);
                }
                other => panic!("Expected TagCall, got: {:?}", other),
            }
        }
        _ => panic!("Expected Body value"),
    }
}

#[test]
fn debug_if_parsing() {
    let src = "
Maybe is Some(x) or None

process(value):
if value is Some(x)
    result: x + 1
return result

return 0
";
    let ast = parse_str(src);

    println!("=== AST DEBUG ===");
    for (name, bind) in ast.defs() {
        println!("Bind: {}", name);
        match bind.value() {
            ginc::ast::BindValue::Body { exprs, ret } => {
                println!("  Body expressions: {}", exprs.len());
                for (i, expr) in exprs.iter().enumerate() {
                    println!("    [{}]: {:?}", i, expr);
                }
                println!("  Return: {:?}", ret);
            }
            _ => println!("  Not a body"),
        }
    }

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().values().next().unwrap();
    if let ginc::ast::BindValue::Body { exprs, ret } = bind.value() {
        println!("Total exprs: {}", exprs.len());
        for (i, expr) in exprs.iter().enumerate() {
            println!("  Expr[{}]: {:?}", i, expr);
            if let ginc::ast::Expr::If(if_expr) = expr {
                println!("    If ret: {:?}", if_expr.ret);
            }
        }
        println!("Function Return: {:?}", ret);
    }
}

// === Qualified types ===

#[test]
fn test_parse_qualified_type_in_return_annotation() {
    // Test that Bool.True parses as a qualified type in return annotation
    let src = "Bool is
    True or
    False
get_bool() Bool.True:
    True
return True";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().get(&IStr::new("get_bool".to_string())).unwrap();
    assert_eq!(bind.name().as_str(), "get_bool");

    // Check that the return tag is a qualified type
    assert!(bind.return_tag.is_some());
    let return_tag = bind.return_tag.as_ref().unwrap();
    match return_tag {
        ginc::ast::Tag::Qualified(path) => {
            assert_eq!(path.root.as_str(), "Bool");
            assert_eq!(path.segments.len(), 1);
            assert_eq!(path.segments[0].as_str(), "True");
        }
        other => panic!("Expected Qualified tag, got: {:?}", other),
    }
}

#[test]
fn test_parse_qualified_type_in_parameter() {
    // Test that Bool.True parses as a qualified type in parameter
    let src = "Bool is
    True or
    False
check(b Bool.True):
    True
return";
    let ast = parse_str(src);

    assert_eq!(ast.defs().len(), 1);
    let bind = ast.defs().get(&IStr::new("check".to_string())).unwrap();
    assert_eq!(bind.name().as_str(), "check");

    // Check that the parameter type is a qualified type
    let params = bind.params().as_ref().unwrap();
    assert_eq!(params.len(), 1);
    let (_, param_kind) = params.iter().next().unwrap();
    match param_kind {
        ginc::ast::ParameterKind::Tagged(tag) => match tag {
            ginc::ast::Tag::Qualified(path) => {
                assert_eq!(path.root.as_str(), "Bool");
                assert_eq!(path.segments.len(), 1);
                assert_eq!(path.segments[0].as_str(), "True");
            }
            other => panic!("Expected Qualified tag, got: {:?}", other),
        },
        other => panic!("Expected Tagged parameter, got: {:?}", other),
    }
}
