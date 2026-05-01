pub(crate) mod bind;
pub(crate) mod body_trivia;
pub(crate) mod control;
pub(crate) mod r#import;
mod literal;

// TODO: Split `control.rs` into per-construct files for better co-location of
// parsing logic with its corresponding AST type. Target structure:
//
//   body.rs        ← shared: parse_body_exprs, can_start_expr, parse_pattern, parse_simple_tag
//   for_in.rs      ← pub fn parse() -> Option<ForInLoop>
//   while_loop.rs  ← pub fn parse() -> Option<WhileLoop>
//   loop_expr.rs   ← pub fn parse() -> Option<Loop>  (dispatches to for_in / while_loop)
//   if_expr.rs     ← pub fn parse() -> Option<IfExpr>
//   when_expr.rs   ← pub fn parse() -> Option<WhenExpr>
//   return_expr.rs ← pub fn parse() -> Option<Return>
//
// This also deduplicates `parse_body_exprs` and `can_start_expr` that currently
// exist in both control.rs and bind.rs. See conversation history for full plan.

use internment::Intern;
use lexer::{Lexer, Token};

use ast::span::{SpanId, SpanTable};
use ast::{Expr, FileAst, Spanned};

use crate::cursor::{self, ParseError, TokenCursor};

use crate::unescape::unescape;
use ast::ModPath;
use ast::{AsmExpr, BinOp, Binary, FnCall, FormatPart, FormatString, Range, TagCall};

pub type ExprFn = fn(&mut TokenCursor) -> Spanned<Expr>;

pub fn parse_tokens(tokens: &[(Token<'_>, SpanId)], span_table: &mut SpanTable) -> FileAst {
    let mut cursor = cursor::TokenCursor::new(tokens, span_table);
    crate::top_level::parse_file(&mut cursor, parse_expression as ExprFn)
}

/// Parse tokens and return both the AST and any parse errors accumulated
/// by the cursor during parsing.
pub fn parse_tokens_with_errors(
    tokens: &[(Token<'_>, SpanId)],
    span_table: &mut SpanTable,
) -> (FileAst, Vec<ParseError>) {
    parse_tokens_with_errors_cancellable(tokens, span_table, &|| {})
}

/// Same as [`parse_tokens_with_errors`] but installs a cancellation hook
/// invoked once per successful parse-loop iteration. The hook should panic
/// (typically with `salsa::Cancelled`) to abort the parse — the panic
/// propagates up out of the parser to the caller, who is responsible for
/// catching it (e.g. via `salsa::Cancelled::catch`).
pub fn parse_tokens_with_errors_cancellable(
    tokens: &[(Token<'_>, SpanId)],
    span_table: &mut SpanTable,
    cancel: &(dyn Fn() + '_),
) -> (FileAst, Vec<ParseError>) {
    let mut cursor = cursor::TokenCursor::new(tokens, span_table).with_cancel_check(cancel);
    let ast = crate::top_level::parse_file(&mut cursor, parse_expression as ExprFn);
    let errors = std::mem::take(&mut cursor.errors);
    (ast, errors)
}

pub fn parse_source(source: &str) -> FileAst {
    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer
        .by_ref()
        .filter_map(|(t, span_id)| {
            if matches!(t, Token::Comment(_)) {
                None
            } else {
                Some((t, span_id))
            }
        })
        .collect();
    let mut span_table = lexer.take_span_table();
    let mut ast = parse_tokens(&tokens, &mut span_table);
    ast.span_table = span_table;
    ast
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn last_consumed_span(cursor: &TokenCursor) -> SpanId {
    cursor.last_consumed_span()
}

fn merge_spans(a: SpanId, b: SpanId, cursor: &mut TokenCursor) -> SpanId {
    cursor.merge_span(a, b)
}

fn parse_paren_args(cursor: &mut TokenCursor) -> Option<Vec<Spanned<Expr>>> {
    if !cursor.is_at(&Token::ParenOpen) {
        return None;
    }
    cursor.advance();

    let mut args = Vec::new();
    if !cursor.is_at(&Token::ParenClose) {
        args.push(parse_expression(cursor));
        while cursor.eat(&Token::Comma) {
            if cursor.is_at(&Token::ParenClose) {
                break;
            }
            args.push(parse_expression(cursor));
        }
    }

    cursor.expect(&Token::ParenClose);
    Some(args)
}

// ─── Main Entry Point ─────────────────────────────────────────────────────────

pub fn parse_expression(cursor: &mut TokenCursor) -> Spanned<Expr> {
    parse_expr_min_prec(cursor, 0)
}

fn parse_expr_min_prec(cursor: &mut TokenCursor, min_prec: u8) -> Spanned<Expr> {
    let mut lhs = match parse_prefix(cursor) {
        Some(expr) => expr,
        None => {
            let span = cursor.current_span();
            cursor.error("expected expression", span);
            return Spanned(Expr::AnonymousTag(cursor.intern("Error"), span), span);
        }
    };

    loop {
        cursor.advance_push();

        lhs = match apply_postfix(cursor, lhs) {
            Ok(postfixed) => {
                cursor.advance_pop();
                lhs = postfixed;
                continue;
            }
            Err(returned) => returned,
        };

        let token = match cursor.peek() {
            Some(t) => *t,
            None => {
                cursor.advance_drop();
                break;
            }
        };

        // Range operator: `...` (precedence 2, left-associative)
        if matches!(token, Token::Infer) {
            if 2 >= min_prec {
                cursor.advance();
                let lhs_span = lhs.1;
                let rhs = parse_expr_min_prec(cursor, 3);
                let rhs_span = rhs.1;
                lhs = Spanned(
                    Expr::Range(Range::new(lhs, rhs)),
                    merge_spans(lhs_span, rhs_span, cursor),
                );
                cursor.advance_pop();
                continue;
            }
            cursor.advance_drop();
            break;
        }

        if let Some((binop, prec)) = try_infix_op(&token)
            && prec >= min_prec
        {
            cursor.advance();
            let lhs_span = lhs.1;
            let rhs = parse_expr_min_prec(cursor, prec + 1);
            let rhs_span = rhs.1;
            lhs = Spanned(
                Expr::Binary(Binary::new(lhs, binop, rhs)),
                merge_spans(lhs_span, rhs_span, cursor),
            );
            cursor.advance_pop();
            continue;
        }

        cursor.advance_drop();
        break;
    }

    lhs
}

// ─── Prefix ───────────────────────────────────────────────────────────────────

fn parse_prefix(cursor: &mut TokenCursor) -> Option<Spanned<Expr>> {
    match cursor.peek()? {
        // Only `-` is a Pratt prefix (precedence 6); @, ^, * are atoms
        Token::Minus => {
            let (_, start_span) = cursor.advance()?;
            let inner = parse_expr_min_prec(cursor, 6);
            let inner_span = inner.1;
            Some(Spanned(
                Expr::Negate(Box::new(inner)),
                merge_spans(start_span, inner_span, cursor),
            ))
        }
        _ => {
            if matches!(
                cursor.peek(),
                Some(Token::If) | Some(Token::When) | Some(Token::For) | Some(Token::While)
            ) && let Some(expr) = parse_loop_or_control(cursor)
            {
                return Some(expr);
            }
            Some(parse_atom(cursor))
        }
    }
}

// ─── Atom Dispatch ────────────────────────────────────────────────────────────

fn parse_atom(cursor: &mut TokenCursor) -> Spanned<Expr> {
    match cursor.peek() {
        Some(Token::At) => parse_take_ptr(cursor),
        Some(Token::Caret) => parse_take_ref(cursor),
        Some(Token::Star) => parse_deref(cursor),
        Some(Token::SelfInstance) => parse_self_ref(cursor),
        Some(Token::ParenOpen) => parse_tuple_lit_or_alloc_or_group(cursor),
        Some(Token::FormatStringDelim) => parse_format_string_expr(cursor),
        Some(Token::Asm) => parse_asm_expr(cursor),
        Some(Token::Id(_)) => {
            // Fast check: Id followed directly by : or := is always a bind.
            // This avoids calling looks_like_bind (which scans ahead for complex
            // patterns like `id Tag:` and `id(...) type:`) for the simple case.
            // After Phase 3 (body fast path), this branch is rarely hit — it
            // mainly serves expression-context binds and sub-expressions.
            let next_tok = cursor.peek_at(1);
            if matches!(next_tok, Some(Token::Colon) | Some(Token::ColonEq))
                || looks_like_bind(cursor)
            {
                let checkpoint = cursor.checkpoint();
                if let Some(bind) = bind::parse_bind(cursor, parse_expression as ExprFn) {
                    let start_span = cursor.span_at(checkpoint);
                    cursor.consume_trailing_newline();
                    let end_span = last_consumed_span(cursor);
                    return Spanned(
                        Expr::Bind(Box::new(bind)),
                        merge_spans(start_span, end_span, cursor),
                    );
                }
                cursor.rewind(checkpoint);
            }
            parse_id_atom(cursor)
        }
        Some(Token::Tag(_)) => parse_tag_atom(cursor),
        _ => {
            if let Some(Spanned(lit, span)) = literal::parse_literal(cursor) {
                return Spanned(Expr::Lit(lit), span);
            }
            let span = cursor.current_span();
            cursor.error("expected expression", span);
            // Advance past the unrecognised token so the caller makes progress
            cursor.advance();
            Spanned(Expr::AnonymousTag(cursor.intern("Error"), span), span)
        }
    }
}

/// Heuristic: does the current position look like the start of a bind?
/// True when we see `id :`, `id :=`, `id(...) :`, `id Tag :`, or `id Tag(...) :` patterns.
fn looks_like_bind(cursor: &TokenCursor) -> bool {
    // id: or id:=  → definitely a bind
    if matches!(cursor.peek_at(1), Some(Token::Colon) | Some(Token::ColonEq)) {
        return true;
    }

    // id Tag or id Tag(...) followed by `:`/`:=`  → typed bind (e.g. `val Maybe(3): Some(3)`)
    if matches!(cursor.peek_at(1), Some(Token::Tag(_))) {
        let mut offset = 2; // skip past id and Tag
        // Skip optional parenthesised args on the Tag
        if cursor.peek_at(offset) == Some(&Token::ParenOpen) {
            let mut depth = 0;
            loop {
                match cursor.peek_at(offset) {
                    Some(Token::ParenOpen) => {
                        depth += 1;
                        offset += 1;
                    }
                    Some(Token::ParenClose) => {
                        depth -= 1;
                        offset += 1;
                    }
                    Some(_) => {
                        offset += 1;
                    }
                    None => return false,
                }
                if depth == 0 {
                    break;
                }
            }
        }
        return matches!(
            cursor.peek_at(offset),
            Some(Token::Colon) | Some(Token::ColonEq)
        );
    }

    // id(...) followed by something bind-like
    if cursor.peek_at(1) == Some(&Token::ParenOpen) {
        let mut depth = 0;
        let mut offset = 1;
        loop {
            match cursor.peek_at(offset) {
                Some(Token::ParenOpen) => {
                    depth += 1;
                    offset += 1;
                }
                Some(Token::ParenClose) => {
                    depth -= 1;
                    offset += 1;
                }
                Some(_) => {
                    offset += 1;
                }
                None => return false,
            }
            if depth == 0 {
                // After closing paren, a bind has: `:`, `:=`, Tag (return type), or Id (named return)
                return matches!(
                    cursor.peek_at(offset),
                    Some(Token::Colon)
                        | Some(Token::ColonEq)
                        | Some(Token::Tag(_))
                        | Some(Token::Id(_))
                );
            }
        }
    }

    false
}

// ─── Id-based Atoms: BufSet, TupleSet, FnCall ─────────────────────────────────

fn parse_id_atom(cursor: &mut TokenCursor) -> Spanned<Expr> {
    // ── BufSet: name.(index): value or BufGet: name.(index) ──
    if matches!(cursor.peek(), Some(Token::Id(_)))
        && cursor.peek_at(1) == Some(&Token::Dot)
        && cursor.peek_at(2) == Some(&Token::ParenOpen)
    {
        // Deterministic: Id.( is already confirmed by lookahead
        let Some((Token::Id(n), id_span)) = cursor.advance() else {
            unreachable!()
        };
        let name = cursor.intern(n);
        cursor.eat(&Token::Dot);
        cursor.eat(&Token::ParenOpen);
        let index = parse_expression(cursor);
        cursor.expect(&Token::ParenClose);

        if cursor.is_at(&Token::Colon) || cursor.is_at(&Token::ColonEq) {
            // BufSet
            let is_const = cursor.eat(&Token::ColonEq);
            if !is_const {
                cursor.eat(&Token::Colon);
            }
            let value = parse_expression(cursor);
            let end_span = last_consumed_span(cursor);
            let base = Spanned(
                Expr::FnCall(FnCall {
                    path: ModPath::new(name, vec![], id_span),
                    args: None,
                }),
                id_span,
            );
            return Spanned(
                Expr::BufSet {
                    buf: Box::new(base),
                    index: Box::new(index),
                    value: Box::new(value),
                },
                merge_spans(id_span, end_span, cursor),
            );
        }
        // BufGet — return as expression
        let base = Spanned(
            Expr::FnCall(FnCall {
                path: ModPath::new(name, vec![], id_span),
                args: None,
            }),
            id_span,
        );
        let end_span = last_consumed_span(cursor);
        return Spanned(
            Expr::BufGet {
                buf: Box::new(base),
                index: Box::new(index),
            },
            merge_spans(id_span, end_span, cursor),
        );
    }

    // ── TupleSet: name.N: value or TupleGet: name.N ──
    if matches!(cursor.peek(), Some(Token::Id(_)))
        && cursor.peek_at(1) == Some(&Token::Dot)
        && matches!(cursor.peek_at(2), Some(Token::Int(_)))
    {
        // Deterministic: Id.N is already confirmed by lookahead
        let Some((Token::Id(n), id_span)) = cursor.advance() else {
            unreachable!()
        };
        let name = cursor.intern(n);
        cursor.eat(&Token::Dot);
        let Some((Token::Int(idx), _idx_span)) = cursor.advance() else {
            unreachable!()
        };

        if cursor.is_at(&Token::Colon) || cursor.is_at(&Token::ColonEq) {
            // TupleSet
            let is_const = cursor.eat(&Token::ColonEq);
            if !is_const {
                cursor.eat(&Token::Colon);
            }
            let value = parse_expression(cursor);
            let end_span = last_consumed_span(cursor);
            let base = Spanned(
                Expr::FnCall(FnCall {
                    path: ModPath::new(name, vec![], id_span),
                    args: None,
                }),
                id_span,
            );
            return Spanned(
                Expr::TupleSet {
                    base: Box::new(base),
                    index: idx as usize,
                    value: Box::new(value),
                },
                merge_spans(id_span, end_span, cursor),
            );
        }
        // TupleGet — return as expression
        let base = Spanned(
            Expr::FnCall(FnCall {
                path: ModPath::new(name, vec![], id_span),
                args: None,
            }),
            id_span,
        );
        let end_span = last_consumed_span(cursor);
        return Spanned(
            Expr::TupleGet {
                base: Box::new(base),
                index: idx as usize,
            },
            merge_spans(id_span, end_span, cursor),
        );
    }

    // ── FnCall: name, name(args), name.path(args) ──
    if let Some(path) = crate::path::parse_path(cursor) {
        let path_span = path.span;
        let args = parse_paren_args(cursor);
        cursor.consume_trailing_newline();
        let end_span = last_consumed_span(cursor);
        return Spanned(
            Expr::FnCall(FnCall { path, args }),
            merge_spans(path_span, end_span, cursor),
        );
    }

    let span = cursor.current_span();
    cursor.error("expected identifier", span);
    Spanned(Expr::AnonymousTag(cursor.intern("Error"), span), span)
}

// ─── Tag-based Atoms: TagCall, AnonymousTag, Tag-rooted FnCall ────────────────

fn parse_tag_atom(cursor: &mut TokenCursor) -> Spanned<Expr> {
    // Single-pass lookahead avoids speculative parsing with rewind.
    // The common case (bare Tag or Tag(args)) skips all path parsing.
    if matches!(cursor.peek(), Some(Token::Tag(_))) && matches!(cursor.peek_at(1), Some(Token::Dot))
    {
        // Qualified variant: Tag.Tag[(args)]
        if matches!(cursor.peek_at(2), Some(Token::Tag(_)))
            && let Some(path) = crate::path::parse_tag_variant_path(cursor)
        {
            let variant_name = *path.segments.last().unwrap_or(&path.root);
            let start_span = path.span;
            let args = if cursor.is_at(&Token::ParenOpen) {
                parse_paren_args(cursor).unwrap_or_default()
            } else {
                Vec::new()
            };
            let end_span = last_consumed_span(cursor);
            let span = merge_spans(start_span, end_span, cursor);
            return Spanned(
                Expr::TagCall(TagCall {
                    name: variant_name,
                    qual_path: Some(path),
                    args,
                    span,
                }),
                span,
            );
        }

        // Tag-rooted method call: Tag.id[.id...][(args)]
        if matches!(cursor.peek_at(2), Some(Token::Id(_)))
            && let Some(path) = crate::path::parse_tag_path(cursor)
        {
            let path_span = path.span;
            let args = parse_paren_args(cursor);
            cursor.consume_trailing_newline();
            let end_span = last_consumed_span(cursor);
            return Spanned(
                Expr::FnCall(FnCall { path, args }),
                merge_spans(path_span, end_span, cursor),
            );
        }
    }

    // Simple Tag(args) → TagCall, or bare Tag → AnonymousTag
    if let Some((Token::Tag(name), tag_span)) = cursor.advance() {
        let name_interned = cursor.intern(name);

        if cursor.is_at(&Token::ParenOpen) {
            let args = parse_paren_args(cursor).unwrap_or_default();
            let end_span = last_consumed_span(cursor);
            let span = merge_spans(tag_span, end_span, cursor);
            return Spanned(
                Expr::TagCall(TagCall {
                    name: name_interned,
                    qual_path: None,
                    args,
                    span,
                }),
                span,
            );
        }

        return Spanned(Expr::AnonymousTag(name_interned, tag_span), tag_span);
    }

    let span = cursor.current_span();
    cursor.error("expected tag", span);
    Spanned(Expr::AnonymousTag(cursor.intern("Error"), span), span)
}

// ─── Specific Atom Parsers (stub-mandated signatures) ─────────────────────────

fn parse_tuple_lit_or_alloc_or_group(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let start_span = cursor.peek_span().unwrap_or_else(|| cursor.current_span());
    cursor.advance(); // consume (

    cursor.skip_newlines();

    // Empty parens → unit or alloc placeholder
    if cursor.is_at(&Token::ParenClose) {
        cursor.advance();
        let end_span = last_consumed_span(cursor);
        return Spanned(
            Expr::TupleLit(Vec::new()),
            merge_spans(start_span, end_span, cursor),
        );
    }

    let first = parse_expression(cursor);

    cursor.skip_newlines();

    // TupleAlloc: (init; size)
    if cursor.eat(&Token::ColonSemi) {
        let size = match cursor.advance() {
            Some((Token::Int(n), _)) => n as usize,
            _ => {
                cursor.error("expected integer size after ';'", cursor.current_span());
                0
            }
        };
        cursor.expect(&Token::ParenClose);
        let end_span = last_consumed_span(cursor);
        return Spanned(
            Expr::TupleAlloc {
                init: Box::new(first),
                size,
            },
            merge_spans(start_span, end_span, cursor),
        );
    }

    // Single element with no comma → grouped expression
    if cursor.is_at(&Token::ParenClose) {
        cursor.advance();
        return first;
    }

    // Multiple elements
    let mut elems = vec![first];
    while cursor.eat(&Token::Comma) {
        cursor.skip_newlines();
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        elems.push(parse_expression(cursor));
        cursor.skip_newlines();
    }

    cursor.expect(&Token::ParenClose);
    let end_span = last_consumed_span(cursor);
    Spanned(
        Expr::TupleLit(elems),
        merge_spans(start_span, end_span, cursor),
    )
}

fn parse_take_ptr(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let (_, start_span) = cursor.advance().unwrap(); // @
    let inner = parse_expression(cursor);
    let end_span = inner.1;
    Spanned(
        Expr::TakePtr(Box::new(inner)),
        merge_spans(start_span, end_span, cursor),
    )
}

fn parse_take_ref(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let (_, start_span) = cursor.advance().unwrap(); // ^
    let inner = parse_expression(cursor);
    let end_span = inner.1;
    Spanned(
        Expr::TakeRef(Box::new(inner)),
        merge_spans(start_span, end_span, cursor),
    )
}

fn parse_deref(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let (_, start_span) = cursor.advance().unwrap(); // *
    let inner = parse_expression(cursor);
    let end_span = inner.1;
    Spanned(
        Expr::Deref(Box::new(inner)),
        merge_spans(start_span, end_span, cursor),
    )
}

fn parse_self_ref(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let (_, span) = cursor.advance().unwrap(); // self
    let start = span;

    // self.field
    if cursor.eat(&Token::Dot)
        && let Some((Token::Id(_field), field_span)) = cursor.advance()
    {
        let merged_span = merge_spans(start, field_span, cursor);
        return Spanned(Expr::SelfRef(merged_span), merged_span);
    }

    // Bare self
    Spanned(Expr::SelfRef(start), start)
}

// ─── Infix / Postfix ─────────────────────────────────────────────────────────

fn try_infix_op(token: &Token) -> Option<(BinOp, u8)> {
    match token {
        // Precedence 3: comparisons and bitwise
        Token::EqEq => Some((BinOp::Equal, 3)),
        Token::NotEq => Some((BinOp::NotEqual, 3)),
        Token::Less => Some((BinOp::LessThan, 3)),
        Token::LessEq => Some((BinOp::LessThanOrEqual, 3)),
        Token::Greater => Some((BinOp::GreaterThan, 3)),
        Token::GreaterEq => Some((BinOp::GreaterThanOrEqual, 3)),
        Token::Ampersand => Some((BinOp::BitAnd, 3)),
        Token::Pipe => Some((BinOp::BitOr, 3)),
        Token::Caret => Some((BinOp::BitXor, 3)),
        Token::ShiftLeft => Some((BinOp::ShiftLeft, 3)),
        Token::ShiftRight => Some((BinOp::ShiftRight, 3)),
        // Precedence 4: arithmetic
        Token::Plus => Some((BinOp::Add, 4)),
        Token::Minus => Some((BinOp::Subtract, 4)),
        Token::Star => Some((BinOp::Multiply, 4)),
        Token::Slash => Some((BinOp::Divide, 4)),
        Token::Percent => Some((BinOp::Modulo, 4)),
        _ => None,
    }
}

#[allow(clippy::result_large_err)]
fn apply_postfix(
    cursor: &mut TokenCursor,
    lhs: Spanned<Expr>,
) -> Result<Spanned<Expr>, Spanned<Expr>> {
    // .N → TupleGet
    if cursor.is_at(&Token::Dot)
        && let Some(&Token::Int(n)) = cursor.peek_at(1)
    {
        cursor.advance(); // Dot
        cursor.advance(); // Int
        let end_span = last_consumed_span(cursor);
        let lhs_span = lhs.1;
        return Ok(Spanned(
            Expr::TupleGet {
                base: Box::new(lhs),
                index: n as usize,
            },
            merge_spans(lhs_span, end_span, cursor),
        ));
    }

    // .(expr) → BufGet
    if cursor.is_at(&Token::Dot) && cursor.peek_at(1) == Some(&Token::ParenOpen) {
        cursor.advance(); // Dot
        cursor.advance(); // ParenOpen
        let index = parse_expression(cursor);
        cursor.expect(&Token::ParenClose);
        let end_span = last_consumed_span(cursor);
        let lhs_span = lhs.1;
        return Ok(Spanned(
            Expr::BufGet {
                buf: Box::new(lhs),
                index: Box::new(index),
            },
            merge_spans(lhs_span, end_span, cursor),
        ));
    }

    // as Type → Cast
    if cursor.is_at(&Token::As) {
        cursor.advance(); // As
        match cursor.advance() {
            Some((Token::Tag(name), _)) => {
                let ty = cursor.intern(name);
                let end_span = last_consumed_span(cursor);
                let lhs_span = lhs.1;
                return Ok(Spanned(
                    Expr::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    merge_spans(lhs_span, end_span, cursor),
                ));
            }
            _ => {
                cursor.error("expected type name after 'as'", cursor.current_span());
            }
        }
    }

    Err(lhs)
}

// ─── Control Flow Dispatch ────────────────────────────────────────────────────

fn parse_loop_or_control(cursor: &mut TokenCursor) -> Option<Spanned<Expr>> {
    let start_span = cursor.current_span();

    if let Some(if_expr) = control::parse_if_expr(cursor, parse_expression as ExprFn) {
        let end_span = last_consumed_span(cursor);
        return Some(Spanned(
            Expr::If(if_expr),
            merge_spans(start_span, end_span, cursor),
        ));
    }

    if let Some(when_expr) = control::parse_when_expr(cursor, parse_expression as ExprFn) {
        cursor.consume_trailing_newline();
        let end_span = last_consumed_span(cursor);
        return Some(Spanned(
            Expr::When(when_expr),
            merge_spans(start_span, end_span, cursor),
        ));
    }

    if let Some(loop_expr) = control::parse_loop_expr(cursor, parse_expression as ExprFn) {
        let end_span = last_consumed_span(cursor);
        return Some(Spanned(
            Expr::Loop(loop_expr),
            merge_spans(start_span, end_span, cursor),
        ));
    }

    None
}

// ─── Format String ────────────────────────────────────────────────────────────

fn parse_format_string_expr(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let start_span = cursor.peek_span().unwrap_or_else(|| cursor.current_span());
    cursor.advance(); // consume opening "

    let mut parts = Vec::new();

    loop {
        cursor.advance_push();
        match cursor.peek() {
            Some(Token::FormatStringText(s)) => {
                parts.push(FormatPart::Text(unescape(s)));
                cursor.advance();
                cursor.advance_pop();
            }
            Some(Token::FormatInterpStart) => {
                cursor.advance();
                let expr = parse_expression(cursor);
                parts.push(FormatPart::Expr(Box::new(expr)));
                cursor.expect(&Token::FormatInterpEnd);
                cursor.advance_pop();
            }
            Some(Token::FormatStringDelim) => {
                cursor.advance(); // consume closing "
                cursor.advance_drop();
                let end_span = last_consumed_span(cursor);
                return Spanned(
                    Expr::FormatString(FormatString { parts }),
                    merge_spans(start_span, end_span, cursor),
                );
            }
            Some(Token::UnterminatedFormatString) | None => {
                cursor.advance_drop();
                let span = cursor.current_span();
                cursor.error("unterminated format string", span);
                return Spanned(
                    Expr::FormatString(FormatString { parts }),
                    merge_spans(start_span, span, cursor),
                );
            }
            _ => {
                let span = cursor.current_span();
                cursor.error("unexpected token in format string", span);
                cursor.advance();
                cursor.advance_pop();
            }
        }
    }
}

fn parse_asm_expr(cursor: &mut TokenCursor) -> Spanned<Expr> {
    let start_span = cursor.peek_span().unwrap_or_else(|| cursor.current_span());
    cursor.advance(); // eat Asm

    // expect (
    if !cursor.eat(&Token::ParenOpen) {
        cursor.error("expected '(' after 'asm'", cursor.current_span());
        return Spanned(
            Expr::AnonymousTag(cursor.intern("Error"), start_span),
            start_span,
        );
    }

    // first string literal = template
    let template = match cursor.advance() {
        Some((Token::String(s), _span)) => Intern::<String>::from_ref(s),
        _ => {
            cursor.error("expected assembly template string", cursor.current_span());
            return Spanned(
                Expr::AnonymousTag(cursor.intern("Error"), start_span),
                start_span,
            );
        }
    };

    // expect comma
    if !cursor.eat(&Token::Comma) {
        cursor.error("expected ',' after asm template", cursor.current_span());
        return Spanned(
            Expr::AnonymousTag(cursor.intern("Error"), start_span),
            start_span,
        );
    }

    // second string literal = constraints
    let constraints = match cursor.advance() {
        Some((Token::String(s), _span)) => Intern::<String>::from_ref(s),
        _ => {
            cursor.error("expected constraints string", cursor.current_span());
            return Spanned(
                Expr::AnonymousTag(cursor.intern("Error"), start_span),
                start_span,
            );
        }
    };

    // parse optional comma-separated operand expressions
    let mut operands = Vec::new();
    while cursor.eat(&Token::Comma) {
        // allow trailing comma before )
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        operands.push(parse_expression(cursor));
    }

    // expect )
    if !cursor.eat(&Token::ParenClose) {
        cursor.error("expected ')' after asm operands", cursor.current_span());
        return Spanned(
            Expr::AnonymousTag(cursor.intern("Error"), start_span),
            start_span,
        );
    }

    let end_span = last_consumed_span(cursor);

    Spanned(
        Expr::Asm(AsmExpr {
            template,
            constraints,
            operands,
            span: merge_spans(start_span, end_span, cursor),
        }),
        merge_spans(start_span, end_span, cursor),
    )
}
