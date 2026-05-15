use std::time::{Duration, Instant};

/// Parse the source with the handwritten parser, aborting if it takes too long.
/// Uses a thread + channel so the test runner doesn't hang.
fn parse_handwritten_with_timeout(source: &str, timeout: Duration) -> Result<ast::FileAst, String> {
    let source_owned = source.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<ast::FileAst>();

    let handle = std::thread::Builder::new()
        .name("handwritten_parser".into())
        .spawn(move || {
            let result = parser::expr::parse_source(&source_owned);
            let _ = tx.send(result);
        })
        .map_err(|e| format!("thread spawn failed: {e}"))?;

    match rx.recv_timeout(timeout) {
        Ok(ast) => {
            // Wait for thread to actually finish
            let _ = handle.join();
            Ok(ast)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "handwritten parser TIMEOUT after {:?} on source:\n{}",
            timeout, source
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            // Thread panicked or exited without sending
            let _ = handle.join();
            Err("handwritten parser thread exited without result".into())
        }
    }
}

fn first_def_bind<'a>(ast: &'a ast::FileAst, name: &str) -> &'a ast::Bind {
    let key = internment::Intern::<String>::from_ref(name);
    ast.defs()
        .get(&key)
        .unwrap_or_else(|| panic!("def {name} should exist"))
}

fn assert_handwritten_parses(source: &str) {
    eprintln!(
        "  handwritten parsing: {:?}...",
        &source[..source.len().min(60)]
    );
    let start = Instant::now();
    match parse_handwritten_with_timeout(source, Duration::from_secs(3)) {
        Ok(ast) => {
            eprintln!("    => OK in {:?}", start.elapsed());
            std::hint::black_box(ast);
        }
        Err(msg) => panic!("{}", msg),
    }
}

/// Print the token stream from HandLexer for debugging.
fn print_tokens(label: &str, source: &str) {
    eprintln!("\n=== Tokens for {:?} ===", label);
    let tokens: Vec<_> = lexer::Lexer::new(source)
        .by_ref()
        .filter(|(t, _)| !matches!(t, lexer::Token::Comment(_)))
        .collect();
    for (i, (tok, span)) in tokens.iter().enumerate() {
        eprintln!("  {:3}: {:?} @ {:?}", i, tok, span);
    }
    eprintln!("  total: {} tokens", tokens.len());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 0: Token stream inspection
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn print_all_token_streams() {
    let cases: &[(&str, &str)] = &[
        ("empty", ""),
        ("newline", "\n"),
        ("int", "42\n"),
        ("string", "'hello'\n"),
        ("bind_int", "x: 42\n"),
        ("const_bind", "x := 42\n"),
        ("typed_bind", "x Int: 42\n"),
        ("fn_no_params", "f(): 42\n"),
        ("fn_one_param", "f(x Int): x\n"),
        ("fn_two_params", "f(x, y Int): x + y\n"),
        ("fn_named_return", "f() result: 42\n"),
        ("union_decl", "Result is Ok or Err\n"),
        ("generic_decl", "Maybe[T] is Some(T) or None\n"),
        ("record_decl", "Point has (x Int, y Int)\n"),
        ("range_decl", "U8 is 0...255\n"),
        ("import", "use http.web\n"),
        ("import_alias", "use http.web as h\n"),
        ("tag_call", "Some(5)\n"),
        ("fn_call", "f(1, 2)\n"),
        ("negate", "-1\n"),
        ("binary", "1 + 2\n"),
        ("parens", "(1 + 2)\n"),
    ];
    for (label, source) in cases {
        print_tokens(label, source);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2: Handwritten parser — minimal inputs
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_empty() {
    assert_handwritten_parses("");
}
#[test]
fn hw_newline() {
    assert_handwritten_parses("\n");
}
#[test]
fn hw_two_newlines() {
    assert_handwritten_parses("\n\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 3: Handwritten parser — literals
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_int() {
    assert_handwritten_parses("42\n");
}
#[test]
fn hw_string() {
    assert_handwritten_parses("'hello'\n");
}
#[test]
fn hw_float() {
    assert_handwritten_parses("3.14\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 4: Handwritten parser — simple binds
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_bind_int() {
    assert_handwritten_parses("x: 42\n");
}
#[test]
fn hw_bind_string() {
    assert_handwritten_parses("x: 'hello'\n");
}
#[test]
fn hw_const_bind() {
    assert_handwritten_parses("x := 42\n");
}
#[test]
fn hw_bind_float() {
    assert_handwritten_parses("x: 3.14\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 5: Handwritten parser — typed binds
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_typed_bind_tag() {
    assert_handwritten_parses("x Str: 'hello'\n");
}
#[test]
fn hw_typed_bind_int() {
    assert_handwritten_parses("x Int: 42\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 6: Handwritten parser — function binds
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_fn_empty_params() {
    assert_handwritten_parses("f(): 42\n");
}
#[test]
fn hw_fn_one_param() {
    assert_handwritten_parses("f(x Int): x\n");
}
#[test]
fn hw_fn_two_params() {
    assert_handwritten_parses("f(x, y Int): x + y\n");
}
#[test]
fn hw_fn_named_return() {
    assert_handwritten_parses("f() result: 42\n");
}
#[test]
fn hw_fn_return_type_tag() {
    assert_handwritten_parses("f() Int: 42\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 7: Handwritten parser — multi-line bodies
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_multiline_simple() {
    assert_handwritten_parses(
        "f():
    42
return 0
",
    );
}
#[test]
fn hw_multiline_two_exprs() {
    assert_handwritten_parses(
        "f():
    x: 1
    y: 2
return x
",
    );
}
#[test]
fn hw_multiline_if() {
    assert_handwritten_parses(
        "f(x Int):
    if x > 0
        x
    return 0
return 0
",
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 8: Handwritten parser — declarations
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_declare_union() {
    assert_handwritten_parses("Result is Ok or Err\n");
}
#[test]
fn hw_declare_union_with_params() {
    assert_handwritten_parses("Maybe[T] is Some(T) or None\n");
}
#[test]
fn hw_declare_range() {
    assert_handwritten_parses("U8 is 0...255\n");
}
#[test]
fn hw_declare_record() {
    assert_handwritten_parses(
        "Point has
    (x Int, y Int)
",
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 9: Handwritten parser — imports
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_import_package() {
    assert_handwritten_parses("use http.web\n");
}
#[test]
fn hw_import_alias() {
    assert_handwritten_parses("use http.web as h\n");
}
#[test]
fn hw_import_local() {
    assert_handwritten_parses("use 'math' as math\n");
}
#[test]
fn hw_import_multiple() {
    assert_handwritten_parses("use http.web, crypto.hash\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 10: Handwritten parser — expressions
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_binary_add() {
    assert_handwritten_parses("x: 1 + 2\n");
}
#[test]
fn hw_binary_mul() {
    assert_handwritten_parses("x: 3 * 4\n");
}
#[test]
fn hw_negate() {
    assert_handwritten_parses("x: -1\n");
}
#[test]
fn hw_parens() {
    assert_handwritten_parses("x: (1 + 2)\n");
}
#[test]
fn hw_comparison() {
    assert_handwritten_parses("x: 1 == 2\n");
}
#[test]
fn hw_fn_call() {
    assert_handwritten_parses("x: f(1, 2)\n");
}
#[test]
fn hw_tag_call() {
    assert_handwritten_parses("x: Some(5)\n");
}
#[test]
fn hw_anon_tag() {
    assert_handwritten_parses("x: None\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 11: Handwritten parser — control flow
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_if_return() {
    assert_handwritten_parses(
        "f():
    if true
        1
    return 0
return 0
",
    );
}
#[test]
fn hw_for_loop() {
    assert_handwritten_parses(
        "f():
    for i in 0...10
        i
    loop
return 0
",
    );
}
#[test]
fn hw_while_loop() {
    assert_handwritten_parses(
        "f():
    while true
        1
    loop
return 0
",
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 12: Handwritten parser — complex programs
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Bisection: which part of the small program hangs? ───────────────────────

#[test]
fn bisect_just_declare() {
    assert_handwritten_parses("Maybe[x] is Some(x) or None\n");
}

#[test]
fn bisect_declare_and_empty_main() {
    assert_handwritten_parses(
        "Maybe[x] is Some(x) or None

main:
    return 0
",
    );
}

#[test]
fn bisect_main_with_simple_bind() {
    assert_handwritten_parses(
        "main:
    x: 42
return x
",
    );
}

#[test]
fn bisect_main_with_tagged_bind() {
    // This tests: val Maybe(3): Some(3) — a bind with a type annotation
    assert_handwritten_parses(
        "main:
    val Maybe(3): Some(3)
return val
",
    );
}

#[test]
fn bisect_main_with_const_bind() {
    assert_handwritten_parses(
        "main:
    val := Some(3)
return val
",
    );
}

#[test]
fn bisect_main_with_if() {
    assert_handwritten_parses(
        "main:
    x: 5
    if x > 0
        x
    return 0
return 0
",
    );
}

#[test]
fn bisect_main_with_if_is() {
    assert_handwritten_parses(
        "main:
    val: Some(3)
    if val is Some(v)
        v
    return 0
return 0
",
    );
}

#[test]
fn bisect_main_with_if_and_bind_body() {
    assert_handwritten_parses(
        "main:
    z:
        if true
            x: 1
            y: 2
        return x
    return z
",
    );
}

#[test]
fn bisect_bare_bind_with_unindented_if_then_another_bind() {
    let source = "Maybe[x] is Some(x) or None

Int is 1...400


-- is_empty(v Maybe[x]) Bool: when v is None then True else False

--- Find the index of a target value in a buffer.
--- Scans each byte from left to right until a match is found.
#[complexity(Linear(len))]
find_index(target Byte, buf Buffer, len Int) Int:
    i: 0
    while i < len
        -- if buf.(i) = target
        -- return i
        i: i + 1
    loop
return -1


-- TODO: better type for this
-- but also would be sick if we know that the Int we get back is less then 10
-- so we can have functions narrow potential values for us and that is kept inside the type
-- system
less_than_ten(num Int) Maybe[Int]:
    if num < 10
    return Some(num)
return None

-- also this below
is_positive(x): x > 0

test:
    value Int: 3
    is_above_zero: is_positive(value)
    if is_above_zero
    return value
return

maid:
    val Maybe[Int]: Some(3)

    if val is Some(v)
    return v + 1


return
";
    // Verify both parse_source (ignores errors) and parse_source_full (collects errors) succeed
    assert_handwritten_parses(source);
    let output = parser::parse_source_full(source);
    let parse_flaws: Vec<_> = output
        .symptoms
        .iter()
        .filter(|s| {
            matches!(s.code, diagnostic::DiagnosticCode::Parse(_))
                && matches!(s.category, diagnostic::Category::Flaw)
        })
        .collect();
    assert!(
        parse_flaws.is_empty(),
        "parse_source_full produced {} parse errors: {:?}",
        parse_flaws.len(),
        parse_flaws
    );
}

#[test]
fn hw_small_program() {
    assert_handwritten_parses(
        "Maybe[x] is Some(x) or None

main:
    val Maybe(3): Some(3)
    if val is Some(v)
        val
        four: v + 1
    return four
",
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 8: Handwritten parser — #[complexity(...)] attribute
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hw_complexity_constant() {
    use ast::Complexity;
    let src = "#[complexity(Constant)]\nget(i Int) Byte: buf.(i)\nreturn buf.(i)\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "get");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(c, &Complexity::Constant);
    assert_eq!(c.display_label(), "Constant");
    assert_eq!(c.display_big_o(), "O(1)");
}

#[test]
fn hw_complexity_linear() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(Linear(n))]\nfind_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\nreturn i\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "find_index");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Linear(ComplexityExpr::Var(internment::Intern::<String>::from_ref(
            "n"
        )))
    );
    assert_eq!(c.display_label(), "Linear(n)");
    assert_eq!(c.display_big_o(), "O(n)");
}

#[test]
fn hw_doc_comment_before_attribute() {
    let src = "--- Find the index of a target value in a buffer.\n--- Scans each byte from left to right until a match is found.\n#[complexity(Linear(len))]\nfind_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\nreturn -1\n";
    let ast = parser::expr::parse_source(src);

    let bind = first_def_bind(&ast, "find_index");

    // Doc comment should be attached to the bind, not lost
    let doc = bind.doc_comment().expect("should have a doc comment");
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

    // Complexity attribute should also be present
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    use ast::Complexity;
    assert!(matches!(c, Complexity::Linear(_)));
}

#[test]
fn hw_doc_comment_with_full_main_gin_source() {
    // The exact source from packages/example/src/main.gin.
    // Previously, the -- comment and extra blank lines before the doc comments
    // caused the doc comments to be consumed by error handling before parse_bind
    // could see them.
    let src = "\
Maybe[x] is Some(x) or None

Int is 1...400


-- is_empty(v Maybe[x]) Bool: when v is None then True else False

--- Find the index of a target value in a buffer.
--- Scans each byte from left to right until a match is found.
#[complexity(Linear(len))]
find_index(target Byte, buf Buffer, len Int) Int:
    i: 0
    while i < len
        if buf.(i) = target
        return i
        i: i + 1
    loop
return -1

main:
    val Maybe(3): Some(3)

    if val is Some(v)
        val
        four: v + 1
    return four

    -- when v is Some(x)
    -- then S
    -- else D

    val
return
";
    let ast = parser::expr::parse_source(src);

    let bind = first_def_bind(&ast, "find_index");

    let doc = bind
        .doc_comment()
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
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    use ast::Complexity;
    assert!(matches!(c, Complexity::Linear(_)));
}

#[test]
fn hw_complexity_quadratic() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(Quadratic(n))]\nsort(list List) List:\n    x: list\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "sort");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Quadratic(ComplexityExpr::Var(internment::Intern::<String>::from_ref(
            "n"
        )))
    );
    assert_eq!(c.display_label(), "Quadratic(n)");
    assert_eq!(c.display_big_o(), "O(n²)");
}

#[test]
fn hw_complexity_logarithmic() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(Logarithmic(n))]\nbinary_search(list List, target Int) Int:\n    i: 0\nreturn i\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "binary_search");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Logarithmic(ComplexityExpr::Var(internment::Intern::<String>::from_ref(
            "n"
        )))
    );
    assert_eq!(c.display_label(), "Logarithmic(n)");
    assert_eq!(c.display_big_o(), "O(log n)");
}

#[test]
fn hw_complexity_log_linear() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(LogLinear(n))]\nmerge_sort(list List) List:\n    x: list\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "merge_sort");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::LogLinear(ComplexityExpr::Var(internment::Intern::<String>::from_ref(
            "n"
        )))
    );
    assert_eq!(c.display_label(), "LogLinear(n)");
    assert_eq!(c.display_big_o(), "O(n log n)");
}

#[test]
fn hw_complexity_with_other_attributes() {
    use ast::Complexity;
    let src = "#[inline, complexity(Constant)]\nget(i Int) Byte: buf.(i)\nreturn buf.(i)\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "get");
    assert!(bind.attributes().inline_always);
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(c, &Complexity::Constant);
}

#[test]
fn hw_no_complexity() {
    let src = "add(a Int, b Int) Int: a + b\nreturn a\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "add");
    assert!(bind.attributes().complexity.is_none());
}

#[test]
fn hw_complexity_big_o_with_custom_var() {
    let src = "#[complexity(Linear(items))]\ncount(list List) Int:\n    x: 0\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "count");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(c.display_label(), "Linear(items)");
    assert_eq!(c.display_big_o(), "O(items)");
}

#[test]
fn hw_complexity_product_expr() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(Linear(rows * cols))]\nmatrix_mul(a Matrix, b Matrix) Matrix:\n    x: a\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "matrix_mul");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Linear(ComplexityExpr::Product(vec![
            internment::Intern::<String>::from_ref("rows"),
            internment::Intern::<String>::from_ref("cols"),
        ]))
    );
    assert_eq!(c.display_label(), "Linear(rows * cols)");
    assert_eq!(c.display_big_o(), "O(rows * cols)");
}

#[test]
fn hw_complexity_sum_expr() {
    use ast::{Complexity, ComplexityExpr};
    let src =
        "#[complexity(Linear(V + E))]\nbfs(graph Graph, start Int) List:\n    x: start\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "bfs");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Linear(ComplexityExpr::Sum(vec![
            internment::Intern::<String>::from_ref("V"),
            internment::Intern::<String>::from_ref("E"),
        ]))
    );
    assert_eq!(c.display_label(), "Linear(V + E)");
    assert_eq!(c.display_big_o(), "O(V + E)");
}

#[test]
fn hw_complexity_quadratic_product_expr() {
    use ast::{Complexity, ComplexityExpr};
    let src = "#[complexity(Quadratic(n * m))]\ncross(list_a List, list_b List) List:\n    x: list_a\nreturn x\n";
    let ast = parser::expr::parse_source(src);
    let bind = first_def_bind(&ast, "cross");
    let c = bind
        .attributes()
        .complexity
        .as_ref()
        .expect("should have complexity");
    assert_eq!(
        c,
        &Complexity::Quadratic(ComplexityExpr::Product(vec![
            internment::Intern::<String>::from_ref("n"),
            internment::Intern::<String>::from_ref("m"),
        ]))
    );
    // Compound expr wrapped in parens for superscript
    assert_eq!(c.display_label(), "Quadratic(n * m)");
    assert_eq!(c.display_big_o(), "O((n * m)²)");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Partial-input regression — inputs the LSP routinely sees mid-keystroke.
//
// These cover the class of bug that motivated the cursor `advance_push` /
// `advance_pop` progress assertions: mid-edit token streams that previously
// could trip a parser loop into running forever (or into a soft-recovery
// path that masked the real bug). They must all parse to *something* within
// the timeout — the AST shape is irrelevant here, the contract is "parser
// always terminates and never panics on incomplete user input".
// ═══════════════════════════════════════════════════════════════════════════════

/// `core.` with nothing after — the exact reproducer that froze the LSP
/// from `modules/hello_world/main.gin` before the progress assertions landed.
#[test]
fn partial_dangling_dot_after_id() {
    assert_handwritten_parses("main:\n    core.\nreturn 0\n");
}

#[test]
fn partial_dangling_dot_at_top_level() {
    assert_handwritten_parses("core.\n");
}

#[test]
fn partial_double_dot() {
    assert_handwritten_parses("main:\n    core..\nreturn 0\n");
}

#[test]
fn partial_dangling_dot_eof_no_newline() {
    assert_handwritten_parses("core.");
}

#[test]
fn partial_dangling_use_keyword() {
    assert_handwritten_parses("use\n");
}

#[test]
fn partial_dangling_use_dot() {
    assert_handwritten_parses("use core.\n");
}

#[test]
fn partial_open_paren_eof() {
    assert_handwritten_parses("main:\n    f(\nreturn 0\n");
}

#[test]
fn partial_open_paren_id_eof() {
    assert_handwritten_parses("main:\n    f(x\nreturn 0\n");
}

#[test]
fn partial_dangling_binary_op() {
    assert_handwritten_parses("main:\n    x: 1 +\nreturn 0\n");
}

#[test]
fn partial_dangling_range_op() {
    assert_handwritten_parses("main:\n    r: 1...\nreturn 0\n");
}

#[test]
fn partial_unterminated_string() {
    assert_handwritten_parses("main:\n    s: 'unclosed\nreturn 0\n");
}

#[test]
fn partial_unterminated_format_string() {
    assert_handwritten_parses("main:\n    s: \"hello {name\nreturn 0\n");
}

#[test]
fn partial_bare_dot_in_body() {
    assert_handwritten_parses("main:\n    .\nreturn 0\n");
}

#[test]
fn partial_just_newlines_then_dot() {
    assert_handwritten_parses("\n\n.\n");
}
