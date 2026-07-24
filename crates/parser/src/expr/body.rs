//! Shared body-parsing helpers — extracted from `control.rs` and `bind.rs` where
//! they were duplicated.
//!
//! [`parse_body_exprs`] parses a sequence of expressions terminated by a dedent or EOF,
//! with a fast-path for binds. [`can_start_expr`] is the token-class check used by
//! `parse_body_exprs` and other expression-context entry points.

use internment::Intern;
use lexer::Token;

use ast::{BindValue, Expr, HasSpanId, Typed};

use super::ExprFn;
use crate::cursor::TokenCursor;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Tokens that can begin an expression in body position.
    ///
    /// `Token::Return` is intentionally excluded — `return` after `if` belongs to
    /// `parse_if_expr`'s trailing `parse_return`, not the indented body block.
    pub fn can_start_expr(token: &Token) -> bool {
        matches!(
            token,
            Token::Id(_)
                | Token::Tag(_)
                | Token::SelfTag
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
    /// Parse a sequence of body expressions until dedent or EOF.
    ///
    /// Each iteration:
    /// 1. Skips body trivia (doc comments, newlines).
    /// 2. If the current token can start an expression, tries a fast path for
    ///    binds when the token is an `Id` followed by `:` or `:=`.
    /// 3. Falls through to the general expression parser.
    ///
    /// A progress guard detects infinite loops — if the expression parser makes
    /// no progress from one iteration to the next, an error is emitted.
    /// After parsing a body statement, try to consume a comma separator
    /// before the next statement. This enables `x : 1, y : 2` on one line.
    /// Does NOT error if no comma is found — newline is the primary separator
    /// and is implicitly handled by `peek()` skipping newlines between iterations.
    fn try_consume_body_comma(&mut self) {
        // Only consume comma if a statement-level separator is present.
        // This is entirely optional — newlines are the normal separator.
        if self.eat(&Token::Comma) {
            self.skip_newlines();
        }
    }

    /// Parse a sequence of body expressions until dedent or EOF.
    ///
    /// Each iteration:
    /// 1. Skips body trivia (doc comments, newlines).
    /// 2. If the current token can start an expression, tries a fast path for
    ///    binds when the token is an `Id` followed by `:` or `:=`.
    /// 3. Falls through to the general expression parser.
    ///
    /// A progress guard detects infinite loops — if the expression parser makes
    /// no progress from one iteration to the next, an error is emitted.
    pub fn parse_body_exprs(&mut self, expr_parser: ExprFn) -> Vec<Typed<Expr>> {
        let mut exprs = Vec::new();
        loop {
            self.skip_expr_body_trivia();
            match self.peek() {
                Some(t) if Self::can_start_expr(t) => {
                    // Fast path: in body context, Id followed by : or := is always a bind.
                    // This bypasses the speculative looks_like_bind + checkpoint/rewind in
                    // parse_atom, eliminating ~100 speculative parse_bind calls for large
                    // files with many functions.
                    if matches!(t, Token::Id(_)) {
                        self.skip_newlines();
                        if matches!(self.peek_at(1), Some(Token::Colon) | Some(Token::ColonEq))
                            || self.looks_like_bind()
                        {
                            let start_pos = self.pos();
                            if let Some(bind) = self.parse_bind(expr_parser) {
                                let start_span = self.span_at(start_pos);
                                self.consume_trailing_newline();
                                let end_span = self.last_consumed_span();
                                exprs.push(Typed::infer(
                                    Expr::Bind(Box::new(bind)),
                                    self.merge_span(start_span, end_span),
                                ));
                                // Optional comma separator between body statements
                                self.try_consume_body_comma();
                                continue;
                            }
                            // parse_bind failed on Id: — extremely rare, rewind and fall through
                            self.rewind(start_pos);
                        }
                    }

                    let pos_before = self.pos();
                    self.advance_push();
                    let expr = expr_parser(self);

                    // Check for destructure bind: Tag(args) := value
                    if let Expr::TagCall(tc) = &expr.value
                        && self.is_at(&Token::ColonEq)
                    {
                        self.advance(); // :=
                        let value = expr_parser(self);
                        let value_span = value.span_id();
                        let field_bindings: Vec<(Intern<String>, Intern<String>)> = tc
                            .args
                            .iter()
                            .map(|arg| match &arg.value {
                                Expr::Bind(b) => {
                                    let field_name = b.name;
                                    let bind_name = match &b.value {
                                        BindValue::Expr(e) => match &e.value {
                                            Expr::FnCall(fc) => fc.path.root,
                                            _ => b.name,
                                        },
                                        _ => b.name,
                                    };
                                    (field_name, bind_name)
                                }
                                // Shorthand: bare Id in arg position
                                Expr::FnCall(fc) => (fc.path.root, fc.path.root),
                                _ => (Intern::new(String::new()), Intern::new(String::new())),
                            })
                            .collect();
                        exprs.push(Typed::infer(
                            Expr::Destructure {
                                tag_name: tc.name,
                                field_bindings,
                                value: Box::new(value),
                            },
                            self.merge_span(expr.span_id, value_span),
                        ));
                        self.advance_pop();
                        self.try_consume_body_comma();
                        continue;
                    }

                    exprs.push(expr);
                    self.advance_pop();
                    self.try_consume_body_comma();
                    if self.pos() == pos_before {
                        self.error(
                            "parse-parser-progress",
                            "expression parser made no progress",
                            self.current_span(),
                        );
                        self.advance();
                    }
                }
                _ => break,
            }
        }
        exprs
    }
}
