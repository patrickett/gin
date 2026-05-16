use lexer::Token;

use ast::{
    Expr, FnCall, ForInLoop, IfCondition, IfExpr, Loop, ModPath, Return, Spanned, TypeExpr, Typed,
    WhenArm, WhenExpr, WhileLoop,
};

use super::ExprFn;
use crate::cursor::TokenCursor;
use crate::expr::literal::parse_literal;
use crate::tag::parse_pattern_type_expr;

pub(crate) fn can_start_expr(token: &Token) -> bool {
    // Note: `Token::Return` is intentionally excluded — `return` after `if` belongs to
    // `parse_if_expr`'s trailing `parse_return`, not the indented `parse_body_exprs` block.
    matches!(
        token,
        Token::Id(_)
            | Token::Tag(_)
            | Token::Int(_)
            | Token::Float(_)
            | Token::String(_)
            | Token::SelfInstance
            | Token::Minus
            | Token::At
            | Token::Caret
            | Token::Star
            | Token::ParenOpen
            | Token::If
            | Token::When
            | Token::For
            | Token::While
            | Token::FormatStringDelim
    )
}

/// Parse a `for` binder using the same token shape as before (ids and `(id, …)` only).
/// Produces `Expr` so the AST matches the unified-syntax direction without routing
/// through the full expression parser (keeps `for` headers cheap on hot paths).
fn parse_for_pattern(cursor: &mut TokenCursor) -> Option<Typed<Expr>> {
    match cursor.peek()? {
        Token::Id(name) => {
            let name = *name;
            let start_span = cursor.peek_span()?;
            let id = cursor.intern(name);
            cursor.advance();
            let end_span = cursor.last_consumed_span();
            let span = cursor.merge_span(start_span, end_span);
            Some(Typed::infer(
                Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(id, Vec::new()), start_span),
                    args: None,
                }),
                span,
            ))
        }
        Token::ParenOpen => {
            let start_span = cursor.peek_span()?;
            cursor.advance();
            let mut elems: Vec<Typed<Expr>> = Vec::new();
            loop {
                match cursor.peek() {
                    Some(Token::Id(name)) => {
                        let name = *name;
                        let id_span = cursor.peek_span()?;
                        let id = cursor.intern(name);
                        cursor.advance();
                        let end_id = cursor.last_consumed_span();
                        let elem_span = cursor.merge_span(id_span, end_id);
                        elems.push(Typed::infer(
                            Expr::FnCall(FnCall {
                                path: Spanned::new(ModPath::new(id, Vec::new()), id_span),
                                args: None,
                            }),
                            elem_span,
                        ));
                        if cursor.is_at(&Token::Comma) {
                            cursor.advance();
                        } else {
                            break;
                        }
                    }
                    Some(Token::ParenClose) => break,
                    _ => break,
                }
            }
            cursor.expect(&Token::ParenClose)?;
            let end_span = cursor.last_consumed_span();
            let merged = cursor.merge_span(start_span, end_span);
            match elems.len() {
                0 => Some(Typed::infer(Expr::TupleLit(Vec::new()), merged)),
                1 => {
                    let Typed { value: e, .. } = elems.pop().unwrap();
                    Some(Typed::infer(e, merged))
                }
                _ => Some(Typed::infer(Expr::TupleLit(elems), merged)),
            }
        }
        _ => None,
    }
}

fn parse_body_exprs(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Typed<Expr>> {
    let mut exprs = Vec::new();
    loop {
        super::body_trivia::skip_expr_body_trivia(cursor);
        match cursor.peek() {
            Some(t) if can_start_expr(t) => {
                // Fast path: in body context, Id followed by : or := is always a bind.
                // This bypasses the speculative looks_like_bind + checkpoint/rewind in parse_atom,
                // eliminating speculative parse_bind calls for body binds.
                if matches!(t, Token::Id(_)) {
                    cursor.skip_newlines();
                    if matches!(cursor.peek_at(1), Some(Token::Colon) | Some(Token::ColonEq)) {
                        let start_pos = cursor.pos();
                        if let Some(bind) = super::bind::parse_bind(cursor, expr_parser) {
                            let start_span = cursor.span_at(start_pos);
                            cursor.consume_trailing_newline();
                            let end_span = cursor.last_consumed_span();
                            exprs.push(Typed::infer(
                                Expr::Bind(Box::new(bind)),
                                cursor.merge_span(start_span, end_span),
                            ));
                            continue;
                        }
                        // parse_bind failed on Id: — extremely rare, rewind and fall through
                        cursor.rewind(start_pos);
                    }
                }

                cursor.advance_push();
                let expr = expr_parser(cursor);
                exprs.push(expr);
                cursor.advance_pop();
            }
            _ => break,
        }
    }
    exprs
}

pub fn parse_if_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<IfExpr> {
    if !cursor.eat(&Token::If) {
        return None;
    }

    let cond_expr = expr_parser(cursor);
    let cond_span = cond_expr.span_id;

    let condition = if cursor.eat(&Token::Is) {
        let pattern = parse_pattern_type_expr(cursor, expr_parser)?;
        IfCondition::Pattern {
            subject: Box::new(cond_expr),
            pattern: Box::new(pattern),
        }
    } else {
        IfCondition::Bool(Box::new(cond_expr))
    };

    let has_indent = cursor.eat(&Token::Indent);
    let body = parse_body_exprs(cursor, expr_parser);

    if has_indent {
        cursor.expect(&Token::Dedent);
    } else {
        cursor.eat(&Token::Dedent);
    }

    let ret = parse_return(cursor, expr_parser);
    if ret.is_none() {
        cursor.error("expected 'return' after if body", cursor.current_span());
    }
    let ret = ret?;

    let end_span = cursor.last_consumed_span();

    Some(IfExpr {
        condition,
        body,
        ret,
        span: cursor.merge_span(cond_span, end_span),
    })
}

pub fn parse_when_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<WhenExpr> {
    if !cursor.eat(&Token::When) {
        return None;
    }

    let initial_expr = expr_parser(cursor);

    // Check for a continuation indent before `then`/`else`:
    //   when day == Friday
    //                 then 'yes'
    //                 else 'no'
    // The `then`/`else` are at a higher column than `when`, so the lexer
    // emits an Indent token. We consume it here and eat the matching Dedent
    // after parsing both branches.
    cursor.skip_newlines();
    let when_start_span = cursor.current_span();
    let continuation_cp = cursor.checkpoint();
    let had_continuation_indent =
        cursor.eat(&Token::Indent) && matches!(cursor.peek(), Some(Token::Then));
    if !had_continuation_indent {
        cursor.rewind(continuation_cp);
    }

    match cursor.peek() {
        Some(Token::Then) => {
            cursor.advance();
            let first_result = expr_parser(cursor);

            let mut arms = vec![WhenArm::Cond {
                condition: Box::new(initial_expr),
                body: Box::new(first_result),
                span: cursor.merge_span(when_start_span, cursor.current_span()),
            }];

            // Look for the else arm (newlines auto-skipped by peek)
            cursor.skip_newlines();
            if cursor.eat(&Token::Indent) {
                parse_when_boolean_arms(cursor, expr_parser, &mut arms);
                cursor.eat(&Token::Dedent);
            } else if cursor.is_at(&Token::Else) {
                cursor.advance();
                let body = expr_parser(cursor);
                arms.push(WhenArm::Else(Box::new(body), cursor.last_consumed_span()));
            }

            // If we consumed a continuation Indent, eat the matching Dedent
            if had_continuation_indent {
                cursor.skip_newlines();
                cursor.eat(&Token::Dedent);
            }

            let end_span = cursor.last_consumed_span();
            Some(WhenExpr {
                subject: None,
                arms,
                span: cursor.merge_span(when_start_span, end_span),
            })
        }
        Some(Token::Is) => {
            // Revert any continuation-indent consumption (is-pattern form)
            // before proceeding with the normal is-arm parsing.
            if had_continuation_indent {
                cursor.rewind(continuation_cp);
            }
            let arms = parse_when_is_arms(cursor, expr_parser)?;
            let end_span = cursor.last_consumed_span();
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
                span: cursor.merge_span(when_start_span, end_span),
            })
        }
        Some(Token::Indent) => {
            // Revert any continuation-indent consumption before parsing the
            // indented is-pattern form (`when subject\n    is ...`).
            if had_continuation_indent {
                cursor.rewind(continuation_cp);
            }
            cursor.advance();
            let arms = parse_when_is_arms(cursor, expr_parser)?;
            cursor.eat(&Token::Dedent);
            let end_span = cursor.last_consumed_span();
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
                span: cursor.merge_span(when_start_span, end_span),
            })
        }
        _ => None,
    }
}

fn parse_when_boolean_arms(cursor: &mut TokenCursor, expr_parser: ExprFn, arms: &mut Vec<WhenArm>) {
    loop {
        if cursor.is_at(&Token::Dedent) || cursor.is_eof() {
            break;
        }

        if cursor.is_at(&Token::Else) {
            cursor.advance();
            let body = expr_parser(cursor);
            arms.push(WhenArm::Else(Box::new(body), cursor.last_consumed_span()));
            break;
        }

        let cond = expr_parser(cursor);
        if !cursor.eat(&Token::Then) {
            cursor.error("expected 'then'", cursor.current_span());
            break;
        }
        let body = expr_parser(cursor);
        let span = cursor.merge_span(cond.span_id, body.span_id);
        arms.push(WhenArm::Cond {
            condition: Box::new(cond),
            body: Box::new(body),
            span,
        });
    }
}

fn parse_when_is_arms(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Vec<WhenArm>> {
    let mut arms = Vec::new();
    loop {
        if cursor.is_at(&Token::Dedent) || cursor.is_eof() {
            break;
        }

        if cursor.is_at(&Token::Else) {
            cursor.advance();
            let body = expr_parser(cursor);
            arms.push(WhenArm::Else(Box::new(body), cursor.last_consumed_span()));
            break;
        }

        if !cursor.eat(&Token::Is) {
            if arms.is_empty() {
                return None;
            }
            break;
        }

        // Try literal pattern first (e.g. `is 'debug'`), then tag pattern (e.g. `is Some(x)`)
        let pattern: Spanned<TypeExpr> = if matches!(
            cursor.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) {
            if let Some(Spanned {
                value: lit,
                span_id: span,
            }) = parse_literal(cursor)
            {
                Spanned {
                    value: TypeExpr::Literal(lit, span),
                    span_id: span,
                }
            } else {
                parse_pattern_type_expr(cursor, expr_parser)?
            }
        } else {
            parse_pattern_type_expr(cursor, expr_parser)?
        };

        let indented = cursor.eat(&Token::Indent);

        // Accept either `then` (single-line `is x then y`) or `:` (multi-line `is x: y`)
        if !cursor.eat(&Token::Then) && !cursor.eat(&Token::Colon) {
            cursor.error("expected 'then' or ':'", cursor.current_span());
            return None;
        }

        let body = expr_parser(cursor);
        let arm_span = cursor.merge_span(pattern.span_id, body.span_id);
        arms.push(WhenArm::Is {
            pattern: Box::new(pattern),
            body: Box::new(body),
            span: arm_span,
        });

        if indented {
            cursor.eat(&Token::Dedent);
        }
    }
    Some(arms)
}

pub fn parse_loop_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Loop> {
    if cursor.is_at(&Token::For) {
        cursor.advance();

        let pat = parse_for_pattern(cursor)?;

        if !cursor.eat(&Token::In) {
            cursor.error("expected 'in'", cursor.current_span());
            return None;
        }

        let iter = expr_parser(cursor);

        cursor.eat(&Token::Indent);

        let exprs = parse_body_exprs(cursor, expr_parser);

        cursor.eat(&Token::Dedent);

        if !cursor.eat(&Token::Loop) {
            cursor.error("expected 'loop'", cursor.current_span());
            return None;
        }

        Some(Loop::ForIn(ForInLoop {
            pat: Box::new(pat),
            iter: Box::new(iter),
            exprs,
            span: cursor.current_span(),
        }))
    } else if cursor.is_at(&Token::While) {
        cursor.advance();

        let cond = expr_parser(cursor);

        cursor.eat(&Token::Indent);

        let exprs = parse_body_exprs(cursor, expr_parser);

        cursor.eat(&Token::Dedent);

        if !cursor.eat(&Token::Loop) {
            cursor.error("expected 'loop'", cursor.current_span());
            return None;
        }

        Some(Loop::While(WhileLoop {
            cond: Box::new(cond),
            exprs,
            span: cursor.current_span(),
        }))
    } else {
        None
    }
}

pub fn parse_return(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Return> {
    if !cursor.eat(&Token::Return) {
        return None;
    }

    // Use raw peek_at(0) to check the token immediately after `return`
    // without skipping newlines. If it's a newline or dedent, this is a
    // bare return — the return value must be on the same line.
    if matches!(
        cursor.peek_at(0),
        Some(Token::Newline) | Some(Token::Dedent)
    ) || cursor.peek().is_none()
    {
        let span = cursor.current_span();
        return Some(Return {
            value: None,
            span_id: span,
        });
    }

    let expr = expr_parser(cursor);
    let span = cursor.merge_span(expr.span_id, cursor.last_consumed_span());
    Some(Return {
        value: Some(Box::new(expr)),
        span_id: span,
    })
}
