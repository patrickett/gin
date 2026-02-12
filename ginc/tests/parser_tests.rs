mod helpers;
use helpers::parse_str;

#[test]
fn test_parse_simple_function() {
    let ast = parse_str("f(x): x\n");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    for doc_params in ast.defs.values() {
        assert!(doc_params.doc.is_none())
    }
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_tag_definition() {
    let ast = parse_str("Result is Ok | Err");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.tags.len(), 1);
    assert_eq!(ast.defs.len(), 0);
}

#[test]
fn test_parse_import() {
    let ast = parse_str("use http.web as h\n");

    assert_eq!(ast.uses.len(), 1);
    assert_eq!(ast.defs.len(), 0);
    assert_eq!(ast.tags.len(), 0);
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

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_arithmetic_expression() {
    let ast = parse_str("add(a, b): a + b");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_comparison_expression() {
    let ast = parse_str("is_equal(a, b): a = b");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_function_call() {
    let ast = parse_str("result: add(1, 2)");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_return_type() {
    let ast = parse_str("add(x Number, y Number) = Number: x + y");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_typed_variable() {
    let ast = parse_str("five_hundred = Number: 500");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
}

#[test]
fn test_parse_tag_range() {
    let ast = parse_str("DiceThrow is 1..6");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 0);
    assert_eq!(ast.tags.len(), 1);
}

#[test]
fn test_parse_multi_line_empty_nothing_variable() {
    let ast = parse_str("example:\n\nreturn\n");

    assert!(ast.uses.is_empty());
    assert_eq!(ast.defs.len(), 1);
    assert_eq!(ast.tags.len(), 0);
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
//     assert_eq!(ast.defs.len(), 0);
//     assert_eq!(ast.tags.len(), 0);
// }

// #[test]
// fn test_parse_for_loop() {
//     let ast = parse_str("example:\n\nreturn\n");

//     assert!(ast.imports.is_empty());
//     assert_eq!(ast.defs.len(), 1);
//     for (_name, params) in ast.defs {
//         match params.item.value {
//             DefValue::Expr { .. } => unreachable!(),
//             DefValue::Body { exprs, .. } => {
//                 assert_eq!(exprs.len(), 1)
//                 // TODO: continue to make sure parsed properly
//             }
//         }
//     }
//     assert_eq!(ast.tags.len(), 0);
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
//     assert_eq!(ast.defs.len(), 0);
//     assert_eq!(ast.tags.len(), 0);
// }
