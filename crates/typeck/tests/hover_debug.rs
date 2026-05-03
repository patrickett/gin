//! Debug tests for while loop comparison narrowing.
//!
//! Run with: cargo test -p analyze --test hover_debug -- --nocapture

use parser::expr::parse_source;
use typeck::hover_at;
use typeck::word_at_byte_offset;

fn debug_hover_at_marker(source: &str) -> Option<String> {
    let count = source.matches('†').count();
    assert_eq!(
        count, 1,
        "expected exactly one † cursor marker, found {count}"
    );

    let cursor_byte = source.find('†').unwrap();
    let cleaned = source.replace('†', "");

    let ast = parse_source(&cleaned);

    println!("=== Cleaned source ===");
    println!("{cleaned}");
    println!("=== Cursor byte: {cursor_byte} ===");

    println!("\n=== AST Defs (body structure) ===");
    for (name, bind) in ast.defs() {
        println!("def: {name}");
        match bind.value() {
            ast::BindValue::Body { exprs, ret } => {
                println!("  body exprs ({}):", exprs.len());
                for (i, e) in exprs.iter().enumerate() {
                    print!("    [{i}] ");
                    match &e.0 {
                        ast::Expr::Bind(b) => println!("Bind({})", b.name()),
                        ast::Expr::If(if_expr) => {
                            println!(
                                "If(body=[{} exprs], ret={:?})",
                                if_expr.body.len(),
                                if_expr.ret.0.as_ref().map(|r| format!("{:?}", r.0))
                            );
                        }
                        ast::Expr::Loop(loop_expr) => match loop_expr {
                            ast::Loop::While(w) => {
                                println!(
                                    "While(cond={:?}, body=[{} exprs])",
                                    w.cond.0,
                                    w.exprs.len()
                                );
                                for (j, e2) in w.exprs.iter().enumerate() {
                                    print!("      [{j}] ");
                                    match &e2.0 {
                                        ast::Expr::Bind(b) => println!("Bind({})", b.name()),
                                        ast::Expr::If(if_expr) => {
                                            println!(
                                                "If(body=[{} exprs], ret={:?})",
                                                if_expr.body.len(),
                                                if_expr
                                                    .ret
                                                    .0
                                                    .as_ref()
                                                    .map(|r| format!("{:?}", r.0))
                                            );
                                        }
                                        other => println!("{:?}", other),
                                    }
                                }
                            }
                            ast::Loop::ForIn(f) => {
                                println!("ForIn(body=[{} exprs])", f.exprs.len());
                            }
                        },
                        ast::Expr::FnCall(call) => println!("FnCall({})", call.path.root),
                        other => println!("{:?}", other),
                    }
                }
                if let Some(r) = &ret.0 {
                    println!("  ret: {:?}", r.0);
                }
            }
            ast::BindValue::Expr(e) => println!("  value: Expr({:?})", e.0),
            ast::BindValue::Extern => println!("  extern"),
        }
    }

    // Run flow analysis
    let ty_env = typeck::TyEnv::from_file_ast(&ast);
    let mut analyzer = typeck::FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);
    let flow = analyzer.into_result();
    let span_table = ast.span_table();

    println!("\n=== Flow Analysis ===");
    println!("expr_contexts: {} entries", flow.expr_contexts.len());
    let mut ctx_indices: Vec<_> = flow.expr_contexts.keys().collect();
    ctx_indices.sort();
    for idx in ctx_indices {
        let ctx = &flow.expr_contexts[idx];
        let mut parts = Vec::new();
        for (var, constraint) in ctx.local_constraints() {
            parts.push(format!("{}: {:?}", var.as_str(), constraint));
        }
        for (var, val) in ctx.local_constants() {
            parts.push(format!("{} = {}", var.as_str(), val.to_hover_string()));
        }
        if parts.is_empty() {
            println!("  index {idx}: (empty)");
        } else {
            println!("  index {idx}:");
            for p in parts {
                println!("    {p}");
            }
        }
    }

    println!("\nexpr_spans: {} entries", flow.expr_spans.len());
    let mut span_entries: Vec<_> = flow.expr_spans.iter().collect();
    span_entries.sort_by_key(|(_, idx)| **idx);
    for (span_id, idx) in &span_entries {
        let span = span_table.get(**span_id);
        let snippet = if span.start < cleaned.len() && span.end <= cleaned.len() {
            &cleaned[span.start..span.end]
        } else {
            "<out of bounds>"
        };
        println!(
            "  index {:>3}: span {}..{} ({:>3} chars) {:?}",
            idx,
            span.start,
            span.end,
            span.end - span.start,
            snippet,
        );
    }

    // Check narrowing and constants at cursor
    let word = word_at_byte_offset(&cleaned, cursor_byte);
    println!("\n=== Word at cursor ===");
    println!("word: {:?}", word);

    if let Some(ref w) = word {
        // Find innermost span containing cursor
        let mut best_idx: Option<usize> = None;
        let mut best_len = usize::MAX;
        for (span_id, idx) in &flow.expr_spans {
            let span = span_table.get(*span_id);
            if cursor_byte >= span.start && cursor_byte <= span.end {
                let len = span.end - span.start;
                if len < best_len {
                    best_len = len;
                    best_idx = Some(*idx);
                }
            }
        }

        println!("\nInnermost expr index containing cursor: {:?}", best_idx);

        if let Some(idx) = best_idx {
            if let Some(ctx) = flow.expr_contexts.get(&idx) {
                let var = internment::Intern::<String>::from_ref(w);
                if let Some(constraint) = ctx.get_constraint(&var) {
                    println!("  narrowing: {:?}", constraint);
                } else {
                    println!("  narrowing: (none)");
                }
                if let Some(val) = ctx.get_constant(&var) {
                    println!("  constant: {}", val.to_hover_string());
                } else {
                    println!("  constant: (none)");
                }
            } else {
                println!("  (no context at this index)");
            }
        }

        // Also check ALL spans containing cursor
        println!("\nAll spans containing cursor ({cursor_byte}):");
        let mut all_spans: Vec<_> = flow
            .expr_spans
            .iter()
            .filter(|(span_id, _)| {
                let span = span_table.get(**span_id);
                cursor_byte >= span.start && cursor_byte <= span.end
            })
            .collect();
        all_spans.sort_by_key(|(_, idx)| **idx);
        for (span_id, idx) in &all_spans {
            let span = span_table.get(**span_id);
            let len = span.end - span.start;
            println!(
                "  index {:>3}: span {}..{} (len {})",
                idx, span.start, span.end, len
            );
            if let Some(ctx) = flow.expr_contexts.get(idx) {
                let var = internment::Intern::<String>::from_ref(w);
                if let Some(constraint) = ctx.get_constraint(&var) {
                    println!("    -> narrowing: {:?}", constraint);
                }
                if let Some(val) = ctx.get_constant(&var) {
                    println!("    -> constant: {}", val.to_hover_string());
                }
            }
        }
    }

    let result = hover_at(&cleaned, &ast, cursor_byte);
    println!("\n=== Hover Result ===");
    println!("{:?}", result);

    result
}

#[test]
fn debug_while_i_inside_loop() {
    // i: i + 1 inside while i < len — should show i < len
    debug_hover_at_marker(indoc::indoc! {"
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while i < len
                if buf.(i) = target
                return i
                †i: i + 1
            loop
        return -1
    "});
}

#[test]
fn debug_while_simple_inside() {
    // Simpler while loop without inner if
    debug_hover_at_marker(indoc::indoc! {"
        count(n Int) Int:
            i: 0
            while †i < n
                i: i + 1
            loop
        return i
    "});
}

#[test]
fn debug_while_i_after_loop() {
    // After the while loop, i should be >= len
    debug_hover_at_marker(indoc::indoc! {"
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while i < len
                if buf.(i) = target
                return i
                i: i + 1
            loop
            †i
        return -1
    "});
}

#[test]
fn debug_while_i_on_condition() {
    // On the condition itself
    debug_hover_at_marker(indoc::indoc! {"
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while †i < len
                if buf.(i) = target
                return i
                i: i + 1
            loop
        return -1
    "});
}
