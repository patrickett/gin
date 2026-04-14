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
        ("generic_decl", "Maybe(T) is Some(T) or None\n"),
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
        return x
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
    assert_handwritten_parses("Maybe(T) is Some(T) or None\n");
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
        return 1
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
    assert_handwritten_parses("Maybe(x) is Some(x) or None\n");
}

#[test]
fn bisect_declare_and_empty_main() {
    assert_handwritten_parses(
        "Maybe(x) is Some(x) or None

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
        return x
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
        return v
    return 0
",
    );
}

#[test]
fn bisect_main_with_if_and_bind_body() {
    assert_handwritten_parses(
        "main:
    if true
        x: 1
        y: 2
    return x
",
    );
}

#[test]
fn hw_small_program() {
    assert_handwritten_parses(
        "Maybe(x) is Some(x) or None

main:
    val Maybe(3): Some(3)
    if val is Some(v)
        val
        four: v + 1
    return four
",
    );
}
