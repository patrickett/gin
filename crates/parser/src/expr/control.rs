use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode};
use lexer::Token;

use ast::{
    Condition, Expr, ForInLoop, Loop, ModPath, Pattern, Return, Spanned, SubSpan, Typed, WhileLoop,
};

use super::ExprFn;
use super::path_call;
use crate::cursor::TokenCursor;

impl TokenCursor<'_, '_> {
    pub(super) fn parse_condition_atom(&mut self, expr_parser: ExprFn) -> Option<Condition> {
        if self.eat(&Token::Not) {
            return Some(Condition::Not(Box::new(
                self.parse_condition_atom(expr_parser)?,
            )));
        }
        if self.parenthesized_when_condition() {
            self.expect(&Token::ParenOpen)?;
            let condition = self.parse_condition(expr_parser)?;
            self.expect(&Token::ParenClose)?;
            return Some(condition);
        }
        let subject = self.parse_expr_min_prec(3);
        self.parse_condition_subject(expr_parser, subject)
    }

    pub(super) fn parse_condition_subject(
        &mut self,
        expr_parser: ExprFn,
        subject: Typed<Expr>,
    ) -> Option<Condition> {
        if self.eat(&Token::Is) {
            return Some(Condition::Is {
                subject: Box::new(subject),
                pattern: Box::new(self.parse_condition_pattern(expr_parser)?),
            });
        }
        if self.is_at(&Token::In) {
            return Some(Condition::Is {
                subject: Box::new(subject),
                pattern: Box::new(self.parse_is_pattern_tag(expr_parser)?),
            });
        }
        if let Expr::Binary(binary) = &subject.value
            && let Some(relation) = ast::ProofRelation::for_bin_op(&binary.op)
        {
            return Some(Condition::Compare {
                left: binary.lhs.clone(),
                relation,
                right: binary.rhs.clone(),
            });
        }
        Some(Condition::Proposition {
            subject: Box::new(subject),
        })
    }

    fn parse_condition_and(&mut self, expr_parser: ExprFn) -> Option<Condition> {
        let mut condition = self.parse_condition_atom(expr_parser)?;
        while self.eat(&Token::And) {
            condition = Condition::And(
                Box::new(condition),
                Box::new(self.parse_condition_atom(expr_parser)?),
            );
        }
        Some(condition)
    }

    fn parse_condition(&mut self, expr_parser: ExprFn) -> Option<Condition> {
        let mut condition = self.parse_condition_and(expr_parser)?;
        while self.eat(&Token::Or) {
            condition = Condition::Or(
                Box::new(condition),
                Box::new(self.parse_condition_and(expr_parser)?),
            );
        }
        Some(condition)
    }

    pub(super) fn parse_condition_pattern(
        &mut self,
        expr_parser: ExprFn,
    ) -> Option<Spanned<Pattern>> {
        if matches!(
            self.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) {
            let Spanned {
                value: lit,
                span_id: span,
            } = self.parse_literal()?;
            return Some(Spanned {
                value: Pattern::Literal(lit, span),
                span_id: span,
            });
        }
        self.parse_is_pattern_tag(expr_parser)
    }

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
                    path_call(Spanned::new(ModPath::new(id, Vec::new()), start_span)),
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
                                path_call(Spanned::new(ModPath::new(id, Vec::new()), id_span)),
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
                    0 => {
                        // parse-positional-destructure-removed
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Removed(
                                RemovedSyntaxCode::PositionalDestructure,
                            )),
                            "empty parenthesized binders are not supported",
                            merged,
                        );
                        None
                    }
                    1 => {
                        let Typed { value: e, .. } = elems.pop().unwrap();
                        Some(Typed::infer(e, merged))
                    }
                    _ => {
                        // parse-positional-destructure-removed
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Removed(
                                RemovedSyntaxCode::PositionalDestructure,
                            )),
                            "positional destructuring is not supported; use named bindings",
                            merged,
                        );
                        None
                    }
                }
            }
            _ => None,
        }
    }

    pub(super) fn parenthesized_when_condition(&self) -> bool {
        if !self.is_at(&Token::ParenOpen) {
            return false;
        }
        let mut depth = 0_u32;
        for offset in 0..self.tokens_remaining() {
            match self.peek_at(offset) {
                Some(Token::ParenOpen) => depth += 1,
                Some(Token::ParenClose) => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Some(Token::Is | Token::In | Token::And | Token::Or | Token::Not) if depth == 1 => {
                    return true;
                }
                Some(_) => {}
                None => return false,
            }
        }
        false
    }

    pub fn parse_loop_expr(&mut self, expr_parser: ExprFn) -> Option<Loop> {
        let keyword_span = self.current_span();
        if self.is_at(&Token::For) {
            self.advance();

            let pat = self.parse_for_pattern()?;

            if !self.eat(&Token::In) {
                // parse-expected-in
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::In)),
                    "expected 'in'",
                );
                return None;
            }

            let iter = expr_parser(self);

            self.expect(&Token::Indent)?;
            let exprs = self.parse_body_exprs(expr_parser);
            self.expect(&Token::Dedent)?;

            Some(Loop::ForIn(ForInLoop {
                pat: Box::new(pat),
                iter: Box::new(iter),
                exprs,
                keyword_span: SubSpan::new(keyword_span),
            }))
        } else if self.is_at(&Token::While) {
            self.advance();

            let condition = self.parse_condition(expr_parser)?;

            self.expect(&Token::Do)?;
            self.expect(&Token::Indent)?;
            let exprs = self.parse_body_exprs(expr_parser);
            self.expect(&Token::Dedent)?;

            Some(Loop::While(WhileLoop {
                condition,
                exprs,
                keyword_span: SubSpan::new(keyword_span),
            }))
        } else {
            None
        }
    }

    pub fn parse_return(&mut self) -> Option<Return> {
        self.parse_return_with(|cursor| cursor.parse_expression())
    }

    pub(super) fn parse_return_with(&mut self, expr_parser: ExprFn) -> Option<Return> {
        let start = self.current_span();
        if !self.eat(&Token::Return) {
            return None;
        }

        // Use raw peek_at(0) to check the token immediately after `return`
        // without skipping newlines. If it's a newline or dedent, this is a
        // bare return — the return value must be on the same line.
        if matches!(self.peek_at(0), Some(Token::Newline) | Some(Token::Dedent))
            || self.peek().is_none()
        {
            return Some(Return {
                values: Vec::new(),
                postcondition: None,
                span_id: start,
            });
        }

        let mut elems = if self.is_at(&Token::Thus) {
            Vec::new()
        } else {
            vec![self.parse_sequence_element_with(expr_parser)]
        };
        while self.peek_at(0) == Some(&Token::Comma) {
            self.advance();
            self.skip_layout();
            elems.push(self.parse_sequence_element_with(expr_parser));
        }
        let values = if elems.iter().any(|elem| elem.name.is_some()) {
            vec![self.sequence_value(elems, start)]
        } else {
            elems.into_iter().map(|elem| elem.expr).collect()
        };
        let postcondition = if self.peek_at(0) == Some(&Token::Thus) {
            self.advance();
            let proof_start = self.current_span();
            let proposition = self.parse_proof_proposition();
            Some(Box::new(ast::Spanned::new(
                proposition,
                self.merge_span(proof_start, self.last_consumed_span()),
            )))
        } else {
            None
        };
        let span = self.merge_span(start, self.last_consumed_span());
        Some(Return {
            values,
            postcondition,
            span_id: span,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/when_parse_tests.rs"]
mod when_parse_tests;
