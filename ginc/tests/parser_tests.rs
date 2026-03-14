mod helpers;
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
