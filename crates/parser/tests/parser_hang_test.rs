//! Parser termination & LSP-mid-edit regression contract.
//!
//! * `bisect_*` — bisection of the original "bare bind + unindented if +
//!   another bind" hang; the full reproducer is
//!   `bisect_bare_bind_with_unindented_if_then_another_bind`.
//! * `partial_*` — mid-keystroke token streams the LSP sees while the user
//!   types. Each previously could trip the parser into a forever loop. The
//!   contract is "parse to *something* within the timeout, never panic" —
//!   AST shape is irrelevant.
//! * A handful of named non-hang regressions sharing the same timeout
//!   harness (doc comments + `#[complexity(...)]`, record provided traits).

use ast::{Expr, FnCall};
use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;
use std::time::{Duration, Instant};

/// Parse the source with the handwritten parser, aborting if it takes too long.
/// Uses a thread + channel so the test runner doesn't hang.
fn parse_handwritten_with_timeout(source: &str, timeout: Duration) -> Result<ast::FileAst, String> {
    let source_owned = source.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<ast::FileAst>();

    let handle = std::thread::Builder::new()
        .name("handwritten_parser".into())
        .spawn(move || {
            let result = TokenCursor::parse_source(&source_owned);
            let _ = tx.send(result);
        })
        .map_err(|e| format!("thread spawn failed: {e}"))?;

    match rx.recv_timeout(timeout) {
        Ok(ast) => {
            let _ = handle.join();
            Ok(ast)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "handwritten parser TIMEOUT after {:?} on source:\n{}",
            timeout, source
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            Err("handwritten parser thread exited without result".into())
        }
    }
}

fn first_def_bind<'a>(ast: &'a ast::FileAst, name: &str) -> &'a ast::Bind {
    let key = internment::Intern::<String>::from_ref(name);
    ast.defs
        .get(&key)
        .unwrap_or_else(|| panic!("def {name} should exist"))
}

#[test]
fn bind_body_skips_doc_comments_before_return() {
    let source = r"use core

main:
    cor
    -- for i in 0..10
        -- io.println('Hello, world!')
    -- loop
    return
";
    let out = source.parse_source_full();
    let bogus: Vec<_> = out
        .symptoms
        .iter()
        .filter(|s| s.message.contains("expected 'return' after bind body"))
        .collect();
    assert!(
        bogus.is_empty(),
        "unexpected bind-body return parse errors: {:?}",
        bogus
    );
}

fn find_sum_coords(ast: &ast::FileAst) -> &ast::Typed<ast::Expr> {
    ast.defs
        .values()
        .find_map(|bind| {
            if bind.name.as_str() != "sum_coords" {
                return None;
            }
            let ast::BindValue::Body { ret, .. } = &bind.value else {
                return None;
            };
            ret.values.first()
        })
        .expect("sum_coords bind")
}

#[test]
fn dotted_path_without_call_is_record_get() {
    let src = "Coord has x Int, y Int\n\nsum_coords(a Coord, b Coord) Int: return a.x + b.x\n";
    let ast = src.parse_source_full().ast;
    let body = find_sum_coords(&ast);
    let Expr::Binary(bin) = &body.value else {
        panic!("expected binary + body, got {:?}", body.value);
    };
    let Expr::RecordGet { base, field } = &bin.lhs.value else {
        panic!("expected RecordGet on lhs, got {:?}", bin.lhs.value);
    };
    assert_eq!(field.as_str(), "x");
    let Expr::FnCall(FnCall {
        path, args: None, ..
    }) = &base.value
    else {
        panic!("expected bare `a` reference, got {:?}", base.value);
    };
    assert_eq!(path.value.root.as_str(), "a");
    assert!(path.value.segments.is_empty());
}

/// Parse with a timeout and return the AST, or panic with the case name.
/// Callers that don't need the AST can ignore the return value.
fn assert_handwritten_parses(case: &str, source: &str) -> ast::FileAst {
    eprintln!(
        "  handwritten parsing ({case}): {:?}...",
        &source[..source.len().min(60)]
    );
    let start = Instant::now();
    match parse_handwritten_with_timeout(source, Duration::from_secs(3)) {
        Ok(ast) => {
            eprintln!("    => OK in {:?}", start.elapsed());
            ast
        }
        Err(msg) => panic!("case `{case}` failed: {msg}"),
    }
}

// ─── Bisection of the "bare bind + unindented if + another bind" hang ──────────
// Each test parses a strictly larger slice of the failing source so a future
// regression points at the slice that broke.

const BISECT_CASES: &[(&str, &str)] = &[
    ("bisect_just_declare", "Maybe(x) is Some(x) or None\n"),
    (
        "bisect_declare_and_empty_main",
        "Maybe[x] is Some(x) or None

main:
    return 0
",
    ),
    (
        "bisect_main_with_simple_bind",
        "main:
    x: 42
    return x
",
    ),
    (
        "bisect_main_with_tagged_bind",
        "main:
    val Maybe(3): Some(3)
    return val
",
    ),
    (
        "bisect_main_with_const_bind",
        "main:
    val := Some(3)
    return val
",
    ),
    (
        "bisect_main_with_if",
        "main:
    x: 5
    when x > 0 then
        x
        return 0
    return 0
",
    ),
    (
        "bisect_main_with_if_is",
        "main:
    val: Some(3)
    when val is Some(v) then
        v
        return 0
    return 0
",
    ),
    (
        "bisect_main_with_if_and_bind_body",
        "main:
    z:
        when true then
            x: 1
            y: 2
            return x
        return z
",
    ),
];

/// `core.` with nothing after — the exact reproducer that froze the LSP from a
/// hello-world-style `core.` partial input before progress assertions landed.
const PARTIAL_CASES: &[(&str, &str)] = &[
    (
        "partial_dangling_dot_after_id",
        "main:\n    core.\n    return 0\n",
    ),
    ("partial_dangling_dot_at_top_level", "core.\n"),
    ("partial_double_dot", "main:\n    core..\n    return 0\n"),
    ("partial_dangling_dot_eof_no_newline", "core."),
    ("partial_dangling_use_keyword", "use\n"),
    ("partial_dangling_use_dot", "use core.\n"),
    ("partial_open_paren_eof", "main:\n    f(\n    return 0\n"),
    (
        "partial_open_paren_id_eof",
        "main:\n    f(x\n    return 0\n",
    ),
    (
        "partial_dangling_binary_op",
        "main:\n    x: 1 +\n    return 0\n",
    ),
    (
        "partial_dangling_range_op",
        "main:\n    r: 1 to\n    return 0\n",
    ),
    (
        "partial_unterminated_string",
        "main:\n    s: 'unclosed\n    return 0\n",
    ),
    (
        "partial_unterminated_format_string",
        "main:\n    s: \"hello {name\n    return 0\n",
    ),
    ("partial_bare_dot_in_body", "main:\n    .\n    return 0\n"),
    ("partial_just_newlines_then_dot", "\n\n.\n"),
];

#[test]
fn handwritten_parser_termination_cases() {
    for &(name, source) in BISECT_CASES.iter().chain(PARTIAL_CASES) {
        assert_handwritten_parses(name, source);
    }
}

#[test]
fn bisect_bare_bind_with_unindented_if_then_another_bind() {
    // Full reproducer for the original hang: a bare bind followed by an
    // unindented `if` and then another bind tripped the parser into a loop.
    // Also verifies `parse_source_full` reports no parse flaws on this input.
    let source = "Maybe(x) is Some(x) or None\n\nInt is in 1 to 400\n\n\n-- is_empty(v Maybe(x)) Bool: when v is None then True else False\n\n--- Find the index of a target value in a buffer.\n--- Scans each byte from left to right until a match is found.\n#complexity(Linear(len))\nfind_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\n    while i < len do\n        -- if buf.[i] = target\n        -- return i\n        i: i + 1\n    return -1\n\n\n-- TODO: better type for this\n-- but also would be sick if we know that the Int we get back is less then 10\n-- so we can have functions narrow potential values for us and that is kept inside the type\n-- system\nless_than_ten(num Int) Maybe(Int):\n    when num < 10 then\n        return Some(num)\n    return None\n\n-- also this below\nis_positive(x): return x > 0\n\ntest:\n    value Int: 3\n    is_above_zero: is_positive(value)\n    when is_above_zero is True then\n        return value\n    return\n\nmaid:\n    val Maybe(Int): Some(3)\n\n    when val is Some(v) then\n        return v + 1\n\n\n    return\n";
    assert_handwritten_parses(
        "bisect_bare_bind_with_unindented_if_then_another_bind",
        source,
    );
    let output = source.parse_source_full();
    let parse_flaws: Vec<_> = output
        .symptoms
        .iter()
        .filter(|s| s.code.slug().starts_with("parse-") && s.category == diagnostic::Category::Flaw)
        .collect();
    assert!(
        parse_flaws.is_empty(),
        "parse_source_full produced {} parse errors: {:?}",
        parse_flaws.len(),
        parse_flaws
    );
}

#[test]
fn doc_comment_before_attribute_attaches_to_bind() {
    // Regression: a doc comment immediately before an attribute was previously
    // consumed by attribute parsing and lost.
    let src = "--- Find the index of a target value in a buffer.\n--- Scans each byte from left to right until a match is found.\n#complexity(Linear(len))\nfind_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\n    return -1\n";
    let ast = assert_handwritten_parses("doc_comment_before_attribute_attaches_to_bind", src);

    let bind = first_def_bind(&ast, "find_index");

    let doc = bind
        .doc_comment
        .as_ref()
        .expect("should have a doc comment");
    assert!(
        doc.value.contains("Find the index"),
        "doc should contain 'Find the index', got: {:?}",
        doc.value
    );
    assert!(
        doc.value.contains("Scans each byte"),
        "doc should contain 'Scans each byte', got: {:?}",
        doc.value
    );

    let c = bind
        .attributes
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert!(matches!(c, ast::Complexity::Linear(_)));
}

#[test]
fn doc_comment_survives_full_regression_fixture() {
    // Regression fixture (formerly from an example package main.gin).
    // Previously, the -- comment and extra blank lines before the doc comments
    // caused the doc comments to be consumed by error handling before parse_bind
    // could see them.
    let src = "\
Maybe(x) is Some(x) or None

Int is in 1 to 400


-- is_empty(v Maybe(x)) Bool: when v is None then True else False

--- Find the index of a target value in a buffer.
--- Scans each byte from left to right until a match is found.
#complexity(Linear(len))
find_index(target Byte, buf Buffer, len Int) Int:
    i: 0
    while i < len do
        when buf.[i] = target then
            return i
        i: i + 1
    return -1

main:
    val Maybe(3): Some(3)

    when val is Some(v) then
        val
        four: v + 1
        return four

    -- when v is Some(x)
    -- then S
    -- else D

    val
    return
";
    let ast = assert_handwritten_parses("doc_comment_survives_full_regression_fixture", src);

    let bind = first_def_bind(&ast, "find_index");

    let doc = bind
        .doc_comment
        .as_ref()
        .expect("find_index should have a doc comment");
    assert!(
        doc.value.contains("Find the index"),
        "doc should contain 'Find the index', got: {:?}",
        doc.value
    );
    assert!(
        doc.value.contains("Scans each byte"),
        "doc should contain 'Scans each byte', got: {:?}",
        doc.value
    );

    let c = bind
        .attributes
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert!(matches!(c, ast::Complexity::Linear(_)));
}

#[test]
fn grouped_attributes_emit_removed_syntax_error() {
    let out =
        "#[inline, complexity(Constant)]\nget(i Int) Byte: return buf.[i]\n".parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-removed-grouped-attributes"),
        "expected parse-removed-grouped-attributes, got symptoms: {:?}",
        out.symptoms,
    );
}

#[test]
fn consecutive_inline_and_complexity_attributes_parse() {
    let src =
        "#inline\n#complexity(Constant)\nget(i Int) Byte: return buf.[i]\n    return buf.[i]\n";
    let ast = assert_handwritten_parses("consecutive_inline_and_complexity_attributes_parse", src);
    let bind = first_def_bind(&ast, "get");
    assert!(bind.attributes.inline_always);
    let c = bind
        .attributes
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(c, &ast::Complexity::Constant);
}

#[test]
fn record_declaration_with_provided_trait_parses_ast_shape() {
    let source =
        "Capacity has IsEmpty\n    count PointerSize\n    IsEmpty.is_empty: self.count > 0\n";
    let ast = assert_handwritten_parses(
        "record_declaration_with_provided_trait_parses_ast_shape",
        source,
    );
    let cap_tag = ast
        .tags
        .get(&internment::Intern::<String>::from_ref("Capacity"));
    assert!(cap_tag.is_some(), "Capacity tag should exist");
    let decl = cap_tag.unwrap();
    assert_eq!(
        decl.provided_traits.len(),
        1,
        "should have 1 provided trait"
    );
    let pt = &decl.provided_traits[0];
    assert_eq!(
        pt.trait_name.as_str(),
        "IsEmpty",
        "provided trait should be IsEmpty"
    );
    assert_eq!(pt.fields.len(), 1, "should have 1 field");
    assert_eq!(
        pt.fields[0].0.as_str(),
        "is_empty",
        "field should be is_empty"
    );
}

#[test]
fn record_declaration_with_multiple_provided_traits_parse() {
    let source = "Capacity has IsEmpty and Display\n    count PointerSize\n    IsEmpty.is_empty: self.count > 0\n    Display.text: 'capacity'\n";
    let ast = assert_handwritten_parses(
        "record_declaration_with_multiple_provided_traits_parse",
        source,
    );
    let decl = ast
        .tags
        .get(&internment::Intern::<String>::from_ref("Capacity"))
        .expect("Capacity tag should exist");

    let trait_names: Vec<_> = decl
        .provided_traits
        .iter()
        .map(|pt| pt.trait_name.as_str())
        .collect();
    assert_eq!(trait_names, ["IsEmpty", "Display"]);
    assert_eq!(decl.provided_traits[0].fields[0].0.as_str(), "is_empty");
    assert_eq!(decl.provided_traits[1].fields[0].0.as_str(), "text");
}

#[test]
fn interface_composition_uses_is_and() {
    let source = "InOut is Input and Output\n";
    let ast = assert_handwritten_parses("interface_composition_uses_is_and", source);
    let decl = ast
        .tags
        .get(&internment::Intern::<String>::from_ref("InOut"))
        .expect("InOut tag should exist");

    let trait_names: Vec<_> = decl
        .provided_traits
        .iter()
        .map(|pt| pt.trait_name.as_str())
        .collect();
    assert_eq!(trait_names, ["Input", "Output"]);
    assert!(trait_names.iter().all(|name| {
        decl.provided_traits
            .iter()
            .find(|pt| pt.trait_name.as_str() == *name)
            .is_some_and(|pt| pt.fields.is_empty())
    }));
}

#[test]
fn composed_interface_accepts_qualified_method_body() {
    let source = "TraitA has run(self ref) Int\nTraitB has stop(self ref) Int\nCombined has TraitA and TraitB\n    ordinary(self ref) Int: return 0\n    TraitA.run(self ref) Int: return 1\n";
    let ast = assert_handwritten_parses("composed_interface_accepts_qualified_method_body", source);
    let decl = ast
        .tags
        .get(&internment::Intern::<String>::from_ref("Combined"))
        .expect("Combined tag should exist");

    let ast::DeclareValue::Has(members) = &decl.value else {
        panic!("Combined should be an interface");
    };
    let ast::HasMember::Function(ordinary) = &members[0] else {
        panic!("ordinary member should be a function");
    };
    assert!(ordinary.qualifier.is_none());
    assert_eq!(ordinary.name.as_str(), "ordinary");

    let ast::HasMember::Function(qualified) = &members[1] else {
        panic!("qualified member should be a function");
    };
    assert_eq!(
        qualified.qualifier.as_ref().map(|q| q.name.as_str()),
        Some("TraitA")
    );
    assert_eq!(qualified.name.as_str(), "run");
    assert!(
        ast.defs
            .contains_key(&internment::Intern::from_ref("Combined.ordinary"))
    );
    assert!(
        ast.defs
            .contains_key(&internment::Intern::from_ref("Combined.TraitA.run"))
    );
}

#[test]
fn qualified_conflicting_methods_have_distinct_canonical_names() {
    let source = "TraitA has run(self ref) Int\nTraitB has run(self ref) Int\nCombined has TraitA and TraitB\n    TraitA.run(self ref) Int: return 1\n    TraitB.run(self ref) Int: return 2\n";
    let ast = assert_handwritten_parses(
        "qualified_conflicting_methods_have_distinct_canonical_names",
        source,
    );

    assert!(
        ast.defs
            .contains_key(&internment::Intern::from_ref("Combined.TraitA.run"))
    );
    assert!(
        ast.defs
            .contains_key(&internment::Intern::from_ref("Combined.TraitB.run"))
    );
    assert_eq!(ast.method_binds.len(), 2);
}

#[test]
fn provided_composed_interface_flattens_components() {
    let source = "Input has read\nOutput has write\nInOut is Input and Output\nX0 has InOut\n";
    let ast = assert_handwritten_parses("provided_composed_interface_flattens_components", source);
    let decl = ast
        .tags
        .get(&internment::Intern::<String>::from_ref("X0"))
        .expect("X0 tag should exist");

    let trait_names: Vec<_> = decl
        .provided_traits
        .iter()
        .map(|pt| pt.trait_name.as_str())
        .collect();
    assert_eq!(trait_names, ["InOut", "Input", "Output"]);
}
