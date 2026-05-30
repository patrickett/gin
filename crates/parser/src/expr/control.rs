use lexer::Token;

use ast::{
    Expr, FnCall, ForInLoop, IfCondition, IfExpr, Loop, ModPath, Return, SpanId, Spanned, SubSpan,
    TypeExpr, Typed, WhenArm, WhenExpr, WhileLoop,
};

use super::ExprFn;
use super::body::parse_body_exprs;
use crate::cursor::TokenCursor;
use crate::expr::literal::parse_literal;
use crate::tag::parse_pattern_type_expr;

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

    // Detect improperly indented return inside the if body.
    if has_indent && cursor.is_at(&Token::Return) {
        let span = cursor.peek_span().unwrap_or(SpanId::INVALID);
        cursor.error("__gin_hint__:indented-return", span);
    }

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
        body_span: SubSpan::new(cursor.merge_span(cond_span, end_span)),
    })
}

pub fn parse_when_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<WhenExpr> {
    let start_cp = cursor.checkpoint();
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
                arm_span: SubSpan::new(cursor.merge_span(when_start_span, cursor.current_span())),
            }];

            // Look for the else arm (newlines auto-skipped by peek)
            cursor.skip_newlines();
            if cursor.eat(&Token::Indent) {
                parse_when_boolean_arms(cursor, expr_parser, &mut arms);
                cursor.eat(&Token::Dedent);
            } else if cursor.is_at(&Token::Else) {
                let else_start = cursor.current_span();
                cursor.advance();
                let body = expr_parser(cursor);
                arms.push(WhenArm::Else(
                    Box::new(body),
                    SubSpan::new(cursor.merge_span(else_start, cursor.last_consumed_span())),
                ));
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
                body_span: SubSpan::new(cursor.merge_span(when_start_span, end_span)),
            })
        }
        Some(Token::Is) => {
            // Revert any continuation-indent consumption (is-pattern form)
            // before proceeding with the normal is-arm parsing.
            if had_continuation_indent {
                cursor.rewind(continuation_cp);
            }
            let Some(arms) = parse_when_is_arms(cursor, expr_parser) else {
                cursor.rewind(start_cp);
                return None;
            };
            let end_span = cursor.last_consumed_span();
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
                body_span: SubSpan::new(cursor.merge_span(when_start_span, end_span)),
            })
        }
        Some(Token::Indent) => {
            // Revert any continuation-indent consumption before parsing the
            // indented is-pattern form (`when subject\n    is ...`).
            if had_continuation_indent {
                cursor.rewind(continuation_cp);
            }
            cursor.advance();
            let Some(arms) = parse_when_is_arms(cursor, expr_parser) else {
                cursor.rewind(start_cp);
                return None;
            };
            cursor.eat(&Token::Dedent);
            let end_span = cursor.last_consumed_span();
            Some(WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
                body_span: SubSpan::new(cursor.merge_span(when_start_span, end_span)),
            })
        }
        _ => {
            cursor.rewind(start_cp);
            None
        }
    }
}

fn parse_when_boolean_arms(cursor: &mut TokenCursor, expr_parser: ExprFn, arms: &mut Vec<WhenArm>) {
    loop {
        if cursor.is_at(&Token::Dedent) || cursor.is_eof() {
            break;
        }

        if cursor.is_at(&Token::Else) {
            let else_start = cursor.current_span();
            cursor.advance();
            let body = expr_parser(cursor);
            arms.push(WhenArm::Else(
                Box::new(body),
                SubSpan::new(cursor.merge_span(else_start, cursor.last_consumed_span())),
            ));
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
            arm_span: SubSpan::new(span),
        });
    }
}

fn can_start_when_is_pattern(token: Option<&Token<'_>>) -> bool {
    matches!(
        token,
        Some(Token::String(_))
            | Some(Token::Int(_))
            | Some(Token::Float(_))
            | Some(Token::Tag(_))
            | Some(Token::BracketOpen)
            | Some(Token::ParenOpen)
    )
}

/// One `is` pattern (literal or type expression).
fn parse_when_is_pattern(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Spanned<TypeExpr>> {
    if matches!(
        cursor.peek(),
        Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
    ) && let Some(Spanned {
        value: lit,
        span_id: span,
    }) = parse_literal(cursor)
    {
        return Some(Spanned {
            value: TypeExpr::Literal(lit, span),
            span_id: span,
        });
    }
    parse_pattern_type_expr(cursor, expr_parser)
}

/// `is 'a' or 'b' or 'c'` — multiple literal patterns before `then` (one arm body).
fn parse_when_is_patterns(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Vec<Spanned<TypeExpr>>> {
    let first = parse_when_is_pattern(cursor, expr_parser)?;
    let mut patterns = vec![first];

    loop {
        while cursor.eat(&Token::Indent) || cursor.eat(&Token::Dedent) {}
        if !cursor.eat(&Token::Or) {
            break;
        }
        while cursor.eat(&Token::Indent) || cursor.eat(&Token::Dedent) {}
        if !matches!(
            cursor.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) {
            cursor.error("expected literal pattern after 'or'", cursor.current_span());
            break;
        }
        let Some(Spanned {
            value: lit,
            span_id: span,
        }) = parse_literal(cursor)
        else {
            cursor.error("expected literal pattern after 'or'", cursor.current_span());
            break;
        };
        patterns.push(Spanned {
            value: TypeExpr::Literal(lit, span),
            span_id: span,
        });
    }

    Some(patterns)
}

fn parse_when_is_arms(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Vec<WhenArm>> {
    let mut arms = Vec::new();
    loop {
        cursor.skip_newlines();
        cursor.skip_indents();

        if cursor.is_at(&Token::Dedent) || cursor.is_eof() {
            break;
        }

        if cursor.is_at(&Token::Else) {
            let else_start = cursor.current_span();
            cursor.advance();
            let body = expr_parser(cursor);
            arms.push(WhenArm::Else(
                Box::new(body),
                SubSpan::new(cursor.merge_span(else_start, cursor.last_consumed_span())),
            ));
            break;
        }

        if !cursor.eat(&Token::Is) {
            if arms.is_empty() {
                if !can_start_when_is_pattern(cursor.peek()) {
                    return None;
                }
            } else if !can_start_when_is_pattern(cursor.peek()) {
                break;
            }
            // Continuation arm after `when subject is` (no repeated `is` per line).
        }

        // After `is` (or a continuation arm), patterns may start on the next indented line.
        cursor.skip_newlines();
        cursor.skip_indents();

        let patterns = parse_when_is_patterns(cursor, expr_parser)?;

        let indented = cursor.eat(&Token::Indent);

        // Accept either `then` (single-line `is x then y`) or `:` (multi-line `is x: y`)
        if !cursor.eat(&Token::Then) && !cursor.eat(&Token::Colon) {
            cursor.error("expected 'then' or ':'", cursor.current_span());
            return None;
        }

        let body = expr_parser(cursor);
        let first_span = patterns.first().map(|p| p.span_id).unwrap_or(body.span_id);
        let arm_span = cursor.merge_span(first_span, body.span_id);
        for pattern in patterns {
            arms.push(WhenArm::Is {
                pattern: Box::new(pattern),
                body: Box::new(body.clone()),
                arm_span: SubSpan::new(arm_span),
            });
        }

        if indented {
            cursor.eat(&Token::Dedent);
        }
    }
    // `when subject is` arms are often indented; leave no trailing `Dedent` for top-level.
    while cursor.is_at(&Token::Dedent) {
        cursor.advance();
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
            keyword_span: SubSpan::new(cursor.current_span()),
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
            keyword_span: SubSpan::new(cursor.current_span()),
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

#[cfg(test)]
mod when_parse_tests {
    use super::parse_when_expr;
    use crate::cursor::TokenCursor;
    use crate::expr::parse_expression as expr_parser;
    use lexer::{Lexer, Token};

    fn parse_when_source(source: &str) -> Option<ast::WhenExpr> {
        let mut lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer
            .by_ref()
            .filter(|(t, _)| !matches!(t, Token::Comment(_)))
            .collect();
        let mut span_table = lexer.take_span_table();
        let mut cursor = TokenCursor::new(&tokens, &mut span_table);
        parse_when_expr(&mut cursor, expr_parser as crate::expr::ExprFn)
    }

    #[test]
    fn when_is_multiline_arms_parse() {
        let source = "when target.arch is\n    'x86_64' then BigInt\n    else Int";
        assert!(
            parse_when_source(source).is_some(),
            "multiline is-when should parse"
        );
    }

    #[test]
    fn when_is_single_line_parses() {
        let source = "when target.arch is 'x86_64' then BigInt else Int";
        assert!(parse_when_source(source).is_some());
    }
}
