use lexer::Token;

use ast::{
    Expr, FnCall, ForInLoop, IfExpr, Loop, ModPath, Return, SpanId, Spanned, SubSpan, TypeExpr,
    Typed, WhenArm, WhenExpr, WhileLoop,
};

use super::ExprFn;
use crate::cursor::TokenCursor;

impl TokenCursor<'_, '_> {
    /// Parse a `for` binder using the same token shape as before (ids and `(id, …)` only).
    /// Produces `Expr` so the AST matches the unified-syntax direction without routing
    /// through the full expression parser (keeps `for` headers cheap on hot paths).
    fn parse_for_pattern(&mut self) -> Option<Typed<Expr>> {
        match self.peek()? {
            Token::Id(name) => {
                let name = *name;
                let start_span = self.peek_span()?;
                let id = self.intern(name);
                self.advance();
                let end_span = self.last_consumed_span();
                let span = self.merge_span(start_span, end_span);
                Some(Typed::infer(
                    Expr::FnCall(FnCall {
                        path: Spanned::new(ModPath::new(id, Vec::new()), start_span),
                        args: None,
                    }),
                    span,
                ))
            }
            Token::ParenOpen => {
                let start_span = self.peek_span()?;
                self.advance();
                let mut elems: Vec<Typed<Expr>> = Vec::new();
                loop {
                    match self.peek() {
                        Some(Token::Id(name)) => {
                            let name = *name;
                            let id_span = self.peek_span()?;
                            let id = self.intern(name);
                            self.advance();
                            let end_id = self.last_consumed_span();
                            let elem_span = self.merge_span(id_span, end_id);
                            elems.push(Typed::infer(
                                Expr::FnCall(FnCall {
                                    path: Spanned::new(ModPath::new(id, Vec::new()), id_span),
                                    args: None,
                                }),
                                elem_span,
                            ));
                            if self.is_at(&Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        Some(Token::ParenClose) => break,
                        _ => break,
                    }
                }
                self.expect(&Token::ParenClose)?;
                let end_span = self.last_consumed_span();
                let merged = self.merge_span(start_span, end_span);
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

    pub fn parse_if_expr(&mut self, expr_parser: ExprFn) -> Option<IfExpr> {
        if !self.eat(&Token::If) {
            return None;
        }

        let subject = expr_parser(self);
        let cond_span = subject.span_id;

        let pattern = if self.eat(&Token::Is) {
            Some(Box::new(self.parse_pattern_type_expr(expr_parser)?))
        } else {
            None
        };

        let has_indent = self.eat(&Token::Indent);
        let body = self.parse_body_exprs(expr_parser);

        // Detect improperly indented return inside the if body.
        if has_indent && self.is_at(&Token::Return) {
            let span = self.peek_span().unwrap_or(SpanId::INVALID);
            self.hint("__gin_hint__:indented-return", span);
        }

        if has_indent {
            self.expect(&Token::Dedent);
        } else {
            self.eat(&Token::Dedent);
        }

        let ret = self.parse_return(expr_parser);
        if ret.is_none() {
            self.error(
                "parse-expected-return",
                "expected 'return' after if body",
                self.current_span(),
            );
        }
        let ret = ret?;

        let end_span = self.last_consumed_span();

        Some(IfExpr {
            subject: Box::new(subject),
            pattern,
            body,
            ret,
            body_span: SubSpan::new(self.merge_span(cond_span, end_span)),
        })
    }

    pub fn parse_when_expr(&mut self, expr_parser: ExprFn) -> Option<WhenExpr> {
        let start_cp = self.checkpoint();
        if !self.eat(&Token::When) {
            return None;
        }

        let initial_expr = expr_parser(self);

        // Check for a continuation indent before `then`/`else`:
        //   when day == Friday
        //                 then 'yes'
        //                 else 'no'
        // The `then`/`else` are at a higher column than `when`, so the lexer
        // emits an Indent token. We consume it here and eat the matching Dedent
        // after parsing both branches.
        self.skip_newlines();
        let when_start_span = self.current_span();
        let continuation_cp = self.checkpoint();
        let had_continuation_indent =
            self.eat(&Token::Indent) && matches!(self.peek(), Some(Token::Then));
        if !had_continuation_indent {
            self.rewind(continuation_cp);
        }

        match self.peek() {
            Some(Token::Then) => {
                self.advance();
                let first_result = expr_parser(self);

                let mut arms = vec![WhenArm::Cond {
                    condition: Box::new(initial_expr),
                    body: Box::new(first_result),
                    arm_span: SubSpan::new(self.merge_span(when_start_span, self.current_span())),
                }];

                // Look for the else arm (newlines auto-skipped by peek)
                self.skip_newlines();
                if self.eat(&Token::Indent) {
                    self.parse_when_boolean_arms(expr_parser, &mut arms);
                    self.eat(&Token::Dedent);
                } else if self.is_at(&Token::Else) {
                    let else_start = self.current_span();
                    self.advance();
                    let body = expr_parser(self);
                    arms.push(WhenArm::Else(
                        Box::new(body),
                        SubSpan::new(self.merge_span(else_start, self.last_consumed_span())),
                    ));
                }

                // If we consumed a continuation Indent, eat the matching Dedent
                if had_continuation_indent {
                    self.skip_newlines();
                    self.eat(&Token::Dedent);
                }

                let end_span = self.last_consumed_span();
                Some(WhenExpr {
                    subject: None,
                    arms,
                    body_span: SubSpan::new(self.merge_span(when_start_span, end_span)),
                })
            }
            Some(Token::Is) => {
                // Revert any continuation-indent consumption (is-pattern form)
                // before proceeding with the normal is-arm parsing.
                if had_continuation_indent {
                    self.rewind(continuation_cp);
                }
                let Some(arms) = self.parse_when_is_arms(expr_parser) else {
                    self.rewind(start_cp);
                    return None;
                };
                let end_span = self.last_consumed_span();
                Some(WhenExpr {
                    subject: Some(Box::new(initial_expr)),
                    arms,
                    body_span: SubSpan::new(self.merge_span(when_start_span, end_span)),
                })
            }
            Some(Token::Indent) => {
                // Revert any continuation-indent consumption before parsing the
                // indented is-pattern form (`when subject\n    is ...`).
                if had_continuation_indent {
                    self.rewind(continuation_cp);
                }
                self.advance();
                let Some(arms) = self.parse_when_is_arms(expr_parser) else {
                    self.rewind(start_cp);
                    return None;
                };
                self.eat(&Token::Dedent);
                let end_span = self.last_consumed_span();
                Some(WhenExpr {
                    subject: Some(Box::new(initial_expr)),
                    arms,
                    body_span: SubSpan::new(self.merge_span(when_start_span, end_span)),
                })
            }
            _ => {
                self.rewind(start_cp);
                None
            }
        }
    }

    fn parse_when_boolean_arms(&mut self, expr_parser: ExprFn, arms: &mut Vec<WhenArm>) {
        loop {
            if self.is_at(&Token::Dedent) || self.is_eof() {
                break;
            }

            if self.is_at(&Token::Else) {
                let else_start = self.current_span();
                self.advance();
                let body = expr_parser(self);
                arms.push(WhenArm::Else(
                    Box::new(body),
                    SubSpan::new(self.merge_span(else_start, self.last_consumed_span())),
                ));
                continue;
            }

            let cond = expr_parser(self);
            if !self.eat(&Token::Then) {
                self.error(
                    "parse-expected-then",
                    "expected 'then'",
                    self.current_span(),
                );
                break;
            }
            let body = expr_parser(self);
            let span = self.merge_span(cond.span_id, body.span_id);
            arms.push(WhenArm::Cond {
                condition: Box::new(cond),
                body: Box::new(body),
                arm_span: SubSpan::new(span),
            });
        }
    }

    fn can_start_when_is_pattern(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::String(_))
                | Some(Token::Int(_))
                | Some(Token::Float(_))
                | Some(Token::Tag(_))
                | Some(Token::In)
                | Some(Token::BracketOpen)
                | Some(Token::ParenOpen)
        )
    }

    /// One `is` pattern (literal or type expression).
    fn parse_when_is_pattern(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        if matches!(
            self.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) && let Some(Spanned {
            value: lit,
            span_id: span,
        }) = self.parse_literal()
        {
            return Some(Spanned {
                value: TypeExpr::Literal(lit, span),
                span_id: span,
            });
        }
        self.parse_pattern_type_expr(expr_parser)
    }

    /// `is 'a' or 'b' or 'c'` — multiple literal patterns before `then` (one arm body).
    fn parse_when_is_patterns(&mut self, expr_parser: ExprFn) -> Option<Vec<Spanned<TypeExpr>>> {
        let first = self.parse_when_is_pattern(expr_parser)?;
        let mut patterns = vec![first];

        loop {
            while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
            if !self.eat(&Token::Or) {
                break;
            }
            while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
            if !matches!(
                self.peek(),
                Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
            ) {
                self.error(
                    "parse-expected-literal-pattern",
                    "expected literal pattern after 'or'",
                    self.current_span(),
                );
                break;
            }
            let Some(Spanned {
                value: lit,
                span_id: span,
            }) = self.parse_literal()
            else {
                self.error(
                    "parse-expected-literal-pattern",
                    "expected literal pattern after 'or'",
                    self.current_span(),
                );
                break;
            };
            patterns.push(Spanned {
                value: TypeExpr::Literal(lit, span),
                span_id: span,
            });
        }

        Some(patterns)
    }

    fn parse_when_is_arms(&mut self, expr_parser: ExprFn) -> Option<Vec<WhenArm>> {
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            self.skip_indents();

            if self.is_at(&Token::Dedent) || self.is_eof() {
                break;
            }

            if self.is_at(&Token::Else) {
                let else_start = self.current_span();
                self.advance();
                let body = expr_parser(self);
                arms.push(WhenArm::Else(
                    Box::new(body),
                    SubSpan::new(self.merge_span(else_start, self.last_consumed_span())),
                ));
                // After `else`, check for any remaining arms (which should be an error).
                self.skip_newlines();
                self.skip_indents();
                if !self.is_at(&Token::Dedent) && !self.is_eof() {
                    self.error(
                        "parse-misplaced-else",
                        "`else` must be the last `when` arm",
                        self.current_span(),
                    );
                }
                break;
            }

            if !self.eat(&Token::Is) {
                if arms.is_empty() {
                    if !self.can_start_when_is_pattern() {
                        return None;
                    }
                } else if !self.can_start_when_is_pattern() {
                    let condition = expr_parser(self);
                    if !self.eat(&Token::Then) {
                        self.error(
                            "parse-expected-then",
                            "expected 'then'",
                            self.current_span(),
                        );
                        break;
                    }
                    let body = expr_parser(self);
                    let span = self.merge_span(condition.span_id, body.span_id);
                    arms.push(WhenArm::Cond {
                        condition: Box::new(condition),
                        body: Box::new(body),
                        arm_span: SubSpan::new(span),
                    });
                    continue;
                }
                // Continuation arm after `when subject is` (no repeated `is` per line).
            }

            // After `is` (or a continuation arm), patterns may start on the next indented line.
            self.skip_newlines();
            self.skip_indents();

            let patterns = self.parse_when_is_patterns(expr_parser)?;

            let indented = self.eat(&Token::Indent);

            // Accept either `then` (single-line `is x then y`) or `:` (multi-line `is x: y`)
            if !self.eat(&Token::Then) && !self.eat(&Token::Colon) {
                self.error(
                    "parse-expected-then-or-colon",
                    "expected 'then' or ':'",
                    self.current_span(),
                );
                return None;
            }

            let body = expr_parser(self);
            let first_span = patterns.first().map(|p| p.span_id).unwrap_or(body.span_id);
            let arm_span = self.merge_span(first_span, body.span_id);
            for pattern in patterns {
                arms.push(WhenArm::Is {
                    pattern: Box::new(pattern),
                    body: Box::new(body.clone()),
                    arm_span: SubSpan::new(arm_span),
                });
            }

            if indented {
                self.eat(&Token::Dedent);
            }
        }
        // `when subject is` arms are often indented; leave no trailing `Dedent` for top-level.
        while self.is_at(&Token::Dedent) {
            self.advance();
        }
        Some(arms)
    }

    pub fn parse_loop_expr(&mut self, expr_parser: ExprFn) -> Option<Loop> {
        if self.is_at(&Token::For) {
            self.advance();

            let pat = self.parse_for_pattern()?;

            if !self.eat(&Token::In) {
                self.error("parse-expected-in", "expected 'in'", self.current_span());
                return None;
            }

            let iter = expr_parser(self);

            self.eat(&Token::Indent);

            let exprs = self.parse_body_exprs(expr_parser);

            self.eat(&Token::Dedent);

            if !self.eat(&Token::Loop) {
                self.error(
                    "parse-expected-loop",
                    "expected 'loop'",
                    self.current_span(),
                );
                return None;
            }

            Some(Loop::ForIn(ForInLoop {
                pat: Box::new(pat),
                iter: Box::new(iter),
                exprs,
                keyword_span: SubSpan::new(self.current_span()),
            }))
        } else if self.is_at(&Token::While) {
            self.advance();

            let cond = expr_parser(self);

            self.eat(&Token::Indent);

            let exprs = self.parse_body_exprs(expr_parser);

            self.eat(&Token::Dedent);

            if !self.eat(&Token::Loop) {
                self.error(
                    "parse-expected-loop",
                    "expected 'loop'",
                    self.current_span(),
                );
                return None;
            }

            Some(Loop::While(WhileLoop {
                cond: Box::new(cond),
                exprs,
                keyword_span: SubSpan::new(self.current_span()),
            }))
        } else {
            None
        }
    }

    pub fn parse_return(&mut self, expr_parser: ExprFn) -> Option<Return> {
        if !self.eat(&Token::Return) {
            return None;
        }

        // Use raw peek_at(0) to check the token immediately after `return`
        // without skipping newlines. If it's a newline or dedent, this is a
        // bare return — the return value must be on the same line.
        if matches!(self.peek_at(0), Some(Token::Newline) | Some(Token::Dedent))
            || self.peek().is_none()
        {
            let span = self.current_span();
            return Some(Return {
                value: None,
                span_id: span,
            });
        }

        let expr = expr_parser(self);
        let span = self.merge_span(expr.span_id, self.last_consumed_span());
        Some(Return {
            value: Some(Box::new(expr)),
            span_id: span,
        })
    }
}

#[cfg(test)]
mod when_parse_tests {
    use crate::cursor::TokenCursor;

    use lexer::{Lexer, Token};

    fn parse_when_source(source: &str) -> Option<ast::WhenExpr> {
        let mut lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer
            .by_ref()
            .filter(|(t, _)| !matches!(t, Token::Comment(_)))
            .collect();
        let mut span_table = lexer.take_span_table();
        let mut cursor = TokenCursor::new(&tokens, &mut span_table);
        cursor.parse_when_expr(|c: &mut TokenCursor| c.parse_expression())
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
