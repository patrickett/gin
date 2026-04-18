use lexer::Token;

use ast::{
    Expr, FnCall, ForInLoop, IfCondition, IfExpr, Loop, ModPath, Return, Spanned, WhenArm,
    WhenExpr, WhileLoop,
};

use super::ExprFn;
use crate::cursor::TokenCursor;

fn can_start_expr(token: &Token) -> bool {
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
fn parse_for_pattern(cursor: &mut TokenCursor) -> Option<Spanned<Expr>> {
    match cursor.peek()? {
        Token::Id(name) => {
            let name = *name;
            let start_span = cursor.peek_span()?;
            let id = cursor.intern(name);
            cursor.advance();
            let end_span = cursor.last_consumed_span();
            let span = cursor.merge_span(start_span, end_span);
            Some(Spanned(
                Expr::FnCall(FnCall {
                    path: ModPath::new(id, Vec::new(), start_span),
                    args: None,
                }),
                span,
            ))
        }
        Token::ParenOpen => {
            let start_span = cursor.peek_span()?;
            cursor.advance();
            let mut elems: Vec<Spanned<Expr>> = Vec::new();
            loop {
                match cursor.peek() {
                    Some(Token::Id(name)) => {
                        let name = *name;
                        let id_span = cursor.peek_span()?;
                        let id = cursor.intern(name);
                        cursor.advance();
                        let end_id = cursor.last_consumed_span();
                        let elem_span = cursor.merge_span(id_span, end_id);
                        elems.push(Spanned(
                            Expr::FnCall(FnCall {
                                path: ModPath::new(id, Vec::new(), id_span),
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
                0 => Some(Spanned(Expr::TupleLit(Vec::new()), merged)),
                1 => {
                    let Spanned(e, _) = elems.pop().unwrap();
                    Some(Spanned(e, merged))
                }
                _ => Some(Spanned(Expr::TupleLit(elems), merged)),
            }
        }
        _ => None,
    }
}

fn parse_body_exprs(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Spanned<Expr>> {
    let mut exprs = Vec::new();
    loop {
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
                            exprs.push(Spanned(
                                Expr::Bind(Box::new(bind)),
                                cursor.merge_span(start_span, end_span),
                            ));
                            continue;
                        }
                        // parse_bind failed on Id: — extremely rare, rewind and fall through
                        cursor.rewind(start_pos);
                    }
                }

                let pos_before = cursor.pos();
                let expr = expr_parser(cursor);
                exprs.push(expr);
                if cursor.pos() == pos_before {
                    cursor.error("expression parser made no progress", cursor.current_span());
                    cursor.advance();
                }
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

    let condition = if cursor.eat(&Token::Is) {
        let tag = crate::tag::parse_tag(cursor, expr_parser)?;
        IfCondition::Pattern {
            subject: Box::new(cond_expr),
            tag,
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

    Some(IfExpr {
        condition,
        body,
        ret,
    })
}

pub fn parse_when_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<WhenExpr> {
    if !cursor.eat(&Token::When) {
        return None;
    }

    let initial_expr = expr_parser(cursor);

    match cursor.peek() {
        Some(Token::Then) => {
            cursor.advance();
            let first_result = expr_parser(cursor);

            let mut arms = vec![WhenArm::Cond {
                condition: Box::new(initial_expr),
                body: Box::new(first_result),
            }];

            if cursor.eat(&Token::Indent) {
                parse_when_boolean_arms(cursor, expr_parser, &mut arms);
                cursor.eat(&Token::Dedent);
            } else if cursor.is_at(&Token::Else) {
                cursor.advance();
                let body = expr_parser(cursor);
                arms.push(WhenArm::Else(Box::new(body)));
            }

            Some(WhenExpr {
                subject: None,
                arms,
            })
        }
        Some(Token::Is) => {
            let arms = parse_when_is_arms(cursor, expr_parser)?;
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
            })
        }
        Some(Token::Indent) => {
            cursor.advance();
            let arms = parse_when_is_arms(cursor, expr_parser)?;
            cursor.eat(&Token::Dedent);
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
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
            arms.push(WhenArm::Else(Box::new(body)));
            break;
        }

        let cond = expr_parser(cursor);
        if !cursor.eat(&Token::Then) {
            cursor.error("expected 'then'", cursor.current_span());
            break;
        }
        let body = expr_parser(cursor);
        arms.push(WhenArm::Cond {
            condition: Box::new(cond),
            body: Box::new(body),
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
            arms.push(WhenArm::Else(Box::new(body)));
            break;
        }

        if !cursor.eat(&Token::Is) {
            if arms.is_empty() {
                return None;
            }
            break;
        }

        let tag = crate::tag::parse_tag(cursor, expr_parser)?;

        let indented = cursor.eat(&Token::Indent);

        if !cursor.eat(&Token::Then) {
            cursor.error("expected 'then'", cursor.current_span());
            return None;
        }

        let body = expr_parser(cursor);
        arms.push(WhenArm::Is {
            pattern: tag,
            body: Box::new(body),
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
        return Some(Return(None));
    }

    let expr = expr_parser(cursor);
    Some(Return(Some(Box::new(expr))))
}
