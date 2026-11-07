//! The head of a `when condition then` statement, shared by the file body stack and the block
//! frames of expressions.

use ast::{
    Block, ElseBranch, Expr, Expression, IsTest, MatchPattern, SpanId, Spanned, Statement,
    WhenIsStatement, WhenStatement,
};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};

pub(crate) struct WhenHead {
    pub when_keyword: SpanId,
    /// The condition, or the subject of a one-armed case.
    pub condition: Expression,
    /// `is Pattern`, for a one-armed case.
    pub pattern: Option<(SpanId, Spanned<MatchPattern>)>,
    pub then_keyword: SpanId,
}

impl WhenHead {
    /// The statement this head starts, once its blocks are read.
    pub fn into_statement(
        self,
        then_block: Spanned<Block>,
        else_branch: Option<ElseBranch>,
    ) -> Statement {
        match self.pattern {
            None => Statement::When(Box::new(WhenStatement {
                when_keyword: self.when_keyword,
                condition: self.condition,
                then_keyword: self.then_keyword,
                then_block,
                else_branch,
            })),
            Some((is_keyword, pattern)) => Statement::WhenIs(Box::new(WhenIsStatement {
                when_keyword: self.when_keyword,
                subject: self.condition,
                is_keyword,
                pattern,
                then_keyword: self.then_keyword,
                then_block,
                else_branch,
            })),
        }
    }
}

impl Cursor<'_, '_> {
    /// Reads `when condition then` when a statement body follows, and otherwise leaves the
    /// position and diagnostics as they were.
    ///
    /// A statement body is a block (a newline after `then`), `return`, an assignment, a
    /// destructure, or an expression with no `else` after it. Anything else, such as
    /// `then value else other` or `when x is`, is the expression form.
    pub(crate) fn parse_when_head(&mut self) -> ParseResult<Option<WhenHead>> {
        let saved = (self.position, self.diagnostics.len());
        let when_keyword = self.advance()?.span_id;
        let condition = self.parse_condition()?;
        let mut head = None;
        // `is` followed by a pattern on the same line is a one-armed case; `is` ending the
        // line starts the arms of a full case, which is an expression.
        let mut pattern = None;
        let mut condition = condition;
        if self.current() == Some(Token::Is) {
            let is_keyword = self.advance()?.span_id;
            if !matches!(self.current(), None | Some(Token::Newline | Token::Dedent)) {
                let (matched, depth) = self.parse_match_pattern()?;
                if !matches!(matched.value, MatchPattern::Variant(_)) {
                    // `x is 3` and `x is in Byte` test a number: a condition, not a case.
                    let start = condition.span_id;
                    let end = matched.span_id;
                    self.check_expression_depth(depth + 1)?;
                    condition = self.node(
                        Expr::IsTest(Box::new(IsTest {
                            subject: condition,
                            is_keyword,
                            pattern: matched,
                        })),
                        start,
                        end,
                    );
                } else {
                    pattern = Some((is_keyword, matched));
                }
                // `when x is P and y is Q then`: the test is the left of a longer condition.
                if pattern.is_none() || matches!(self.current(), Some(Token::And | Token::Or)) {
                    if let Some((is_keyword, matched)) = pattern.take() {
                        let start = condition.span_id;
                        let end = matched.span_id;
                        self.check_expression_depth(depth + 1)?;
                        condition = self.node(
                            Expr::IsTest(Box::new(IsTest {
                                subject: condition,
                                is_keyword,
                                pattern: matched,
                            })),
                            start,
                            end,
                        );
                    }
                    condition = self.continue_condition(condition, depth + 1)?;
                }
            }
        }
        let started = pattern.is_some()
            || matches!(condition.value, Expr::IsTest(_))
            || self.tokens[self.position - 1].value != Token::Is;
        if started && self.current() == Some(Token::Then) {
            let then_keyword = self.advance()?.span_id;
            let statement = matches!(self.current(), Some(Token::Newline | Token::Return))
                || self.starts_assignment()
                || self.starts_destructure()
                || self.expression_statement_follows()?;
            if statement {
                head = Some(WhenHead {
                    when_keyword,
                    condition,
                    pattern,
                    then_keyword,
                });
            }
        }
        if head.is_none() {
            self.position = saved.0;
            self.diagnostics.truncate(saved.1);
        }
        Ok(head)
    }

    /// An expression after `then` is a statement unless `else` follows it.
    fn expression_statement_follows(&mut self) -> ParseResult<bool> {
        let saved = (self.position, self.diagnostics.len());
        let followed = if matches!(self.current(), None | Some(Token::Dedent)) {
            false
        } else {
            self.parse_expression()?;
            self.current() != Some(Token::Else)
        };
        self.position = saved.0;
        self.diagnostics.truncate(saved.1);
        Ok(followed)
    }
}
