use ginc::frontend::parser::Parsable;

#[test]
fn test_parse_simple_function() {
    let ast = "f(x): x\n".to_ast().unwrap();

    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        for (_def_name, doc_params) in node.defs {
            assert!(doc_params.doc.is_none())
        }

        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_tag_definition() {
    let ast = "Result is Ok | Err".to_ast().unwrap();

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.tags.len(), 1);
        assert_eq!(node.defs.len(), 0);
    }
}

#[test]
fn test_parse_import() {
    let ast = "use http.web as h\n".to_ast().unwrap();

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert_eq!(node.imports.len(), 1);
        assert_eq!(node.defs.len(), 0);
        assert_eq!(node.tags.len(), 0);
    }
}

// TODO: FIXME: test broken, need to error when return statement indented
#[test]
fn test_parse_multi_line_function_fail() {
    match "f(x):\n    return x + 1".to_ast() {
        Ok(_ast) => unreachable!(),
        Err(errors) => assert!(!errors.is_empty()),
    }
}

#[test]
fn test_parse_multi_line_function_success() {
    let ast = "f(x):\nreturn x + 1".to_ast().unwrap();
    // TODO: handle error

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_arithmetic_expression() {
    let ast = "add(a, b): a + b".to_ast().unwrap();

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_comparison_expression() {
    let ast = "is_equal(a, b): a == b".to_ast().unwrap();

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_function_call() {
    let ast = "result: add(1, 2)".to_ast().unwrap();

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_return_type() {
    let src = "add(x Number, y Number) -> Number: x + y";
    let ast = src.to_ast().unwrap();

    assert_eq!(ast.nodes.len(), 1);
    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_typed_variable() {
    let src = "five_hundred -> Number: 500";
    let ast = src.to_ast().unwrap();

    assert_eq!(ast.nodes.len(), 1);
    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}

#[test]
fn test_parse_tag_range() {
    let src = "DiceThrow is 1..6";
    let ast = src.to_ast().unwrap();

    assert_eq!(ast.nodes.len(), 1);
    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 0);
        assert_eq!(node.tags.len(), 1);
    }
}

#[test]
fn test_parse_multi_line_empty_nothing_variable() {
    let src = "example:\n\nreturn\n";
    let ast = src.to_ast().unwrap();

    assert_eq!(ast.nodes.len(), 1);
    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);

        assert_eq!(node.tags.len(), 0);
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
//     let ast = src.to_ast().unwrap();

//     assert_eq!(ast.nodes.len(), 1);
//     for (_path, node) in ast.nodes {
//         // Expect one definition (the implicit block) and no imports/tags
//         assert!(node.imports.is_empty());
//         assert_eq!(node.defs.len(), 0);
//         assert_eq!(node.tags.len(), 0);
//     }
// }

// #[test]
// fn test_parse_for_loop() {
//     let src = "example:\n\nreturn\n";
//     let ast = src.to_ast().unwrap();

//     assert_eq!(ast.nodes.len(), 1);
//     for (_path, node) in ast.nodes {
//         assert!(node.imports.is_empty());
//         assert_eq!(node.defs.len(), 1);
//         for (_name, params) in node.defs {
//             match params.item.value {
//                 DefValue::Expr { .. } => unreachable!(),
//                 DefValue::Body { exprs, .. } => {
//                     assert_eq!(exprs.len(), 1)
//                     // TODO: continue to make sure parsed properly
//                 }
//             }
//         }
//         assert_eq!(node.tags.len(), 0);
//     }
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
//     let ast = src.to_ast().unwrap();

//     assert_eq!(ast.nodes.len(), 1);
//     for (_path, node) in ast.nodes {
//         assert!(node.imports.is_empty());
//         assert_eq!(node.defs.len(), 0);
//         assert_eq!(node.tags.len(), 0);
//     }
// }
