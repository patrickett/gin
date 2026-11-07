//! Flat consuming-pattern syntax only; resolution and ownership belong later.

use ast::{
    AssignStatement, BinaryOperator, BindingOperator, DestructureStatement, Expr, Expression, LiteralKind,
    MatchPattern, PatternFields, Spanned, Statement, UnaryOperator, VariantPattern,
};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};
use crate::operators::binary_operator;
use ast::SpanId;

/// The operators that can be written before the colon of `place +: value`: arithmetic and
/// bitwise ones, not comparisons or `and`/`or`, which give no value to store back.
fn compound_operator(token: Token) -> Option<BinaryOperator> {
    binary_operator(token).filter(|operator| {
        !matches!(
            operator,
            BinaryOperator::Equal
                | BinaryOperator::Less
                | BinaryOperator::LessOrEqual
                | BinaryOperator::Greater
                | BinaryOperator::GreaterOrEqual
                | BinaryOperator::And
                | BinaryOperator::Or
        )
    })
}

impl Cursor<'_, '_> {
    pub(crate) fn parse_match_pattern(&mut self) -> ParseResult<(Spanned<MatchPattern>, u16)> {
        self.parse_match_pattern_nested(false)
    }

    /// A pattern; `nested` allows a binder position to hold a pattern of its own (a case arm).
    pub(crate) fn parse_match_pattern_nested(&mut self, nested: bool) -> ParseResult<(Spanned<MatchPattern>, u16)> {
        // `in Domain`: a membership test, `x is in Byte` or `x is in 0 to 9`.
        if self.current() == Some(Token::In) {
            let in_keyword = self.advance()?.span_id;
            let domain = self.parse_domain_expression()?;
            let end = domain.span_id;
            self.check_expression_depth(2)?;
            let pattern = self.node(
                MatchPattern::Domain { in_keyword, domain },
                in_keyword,
                end,
            );
            return Ok((pattern, 2));
        }
        if self.current() == Some(Token::Tag) {
            let pattern = self.parse_variant_pattern_nested(nested)?;
            return Ok((
                Spanned::new(MatchPattern::Variant(pattern.value), pattern.span_id),
                0,
            ));
        }
        // A text literal arm, for a case over compile-time text.
        if self.current() == Some(Token::String) {
            let literal = Spanned::new(Expr::Literal(LiteralKind::String), self.advance()?.span_id);
            let span = literal.span_id;
            return Ok((Spanned::new(MatchPattern::Text(literal), span), 1));
        }
        let negative = self.current() == Some(Token::Minus);
        let operator = if negative {
            Some(Spanned::new(UnaryOperator::Negate, self.advance()?.span_id))
        } else {
            None
        };
        if self.current() != Some(Token::Int) {
            let message = if negative || self.current() == Some(Token::Plus) {
                "expected an integer literal after a single '-'; other signed patterns are unsupported"
            } else {
                "expected a constructor or integer literal pattern; computed patterns and wildcards are unsupported"
            };
            return Err(self.error(message));
        }
        let literal = Spanned::new(Expr::Literal(LiteralKind::Integer), self.advance()?.span_id);
        let expression = if let Some(operator) = operator {
            let start = operator.span_id;
            let end = literal.span_id;
            self.node(
                Expr::Unary {
                    operator,
                    operand: Box::new(literal),
                },
                start,
                end,
            )
        } else {
            literal
        };
        if !matches!(
            self.current(),
            None | Some(
                Token::Then
                    | Token::Newline
                    | Token::Dedent
                    | Token::Do
                    | Token::And
                    | Token::Or
                    | Token::Else
                    | Token::ParenClose
                    | Token::Comma
            )
        ) {
            return Err(self.error(
                "expected 'then' after integer literal pattern; computed patterns, ranges, and guards are unsupported",
            ));
        }
        let span = expression.span_id;
        Ok((
            Spanned::new(MatchPattern::Integer(expression), span),
            if negative { 2 } else { 1 },
        ))
    }

    pub(crate) fn starts_destructure(&self) -> bool {
        // Lowercase `name(args) := value` remains a local function binding.
        // A written tag head selects a pattern without resolving its meaning.
        if self.current() != Some(Token::Tag) {
            return false;
        }
        if self.peek(1) == Some(Token::ColonEq) {
            return true;
        }
        if self.peek(1) != Some(Token::ParenOpen) {
            return false;
        }
        let Some(close) = self.parenthesis_closes[self.position + 1] else {
            return false;
        };
        self.tokens.get(close + 1).map(|token| token.value) == Some(Token::ColonEq)
    }

    /// `name`, or a field path `a.b.c`, followed by `::` or by an operator and a colon.
    pub(crate) fn starts_assignment(&self) -> bool {
        if !matches!(self.current(), Some(Token::Id | Token::SelfInstance)) {
            return false;
        }
        let mut offset = 1;
        loop {
            match self.peek(offset) {
                Some(Token::ColonColon) => return true,
                Some(token)
                    if compound_operator(token).is_some()
                        && self.peek(offset + 1) == Some(Token::Colon) =>
                {
                    return true;
                }
                Some(Token::Dot) if self.peek(offset + 1) == Some(Token::Id) => offset += 2,
                // `.[index]`: step over the balanced brackets.
                Some(Token::Dot) if self.peek(offset + 1) == Some(Token::BracketOpen) => {
                    let mut depth = 0usize;
                    offset += 1;
                    loop {
                        match self.peek(offset) {
                            Some(Token::BracketOpen) => depth += 1,
                            Some(Token::BracketClose) => {
                                depth -= 1;
                                if depth == 0 {
                                    offset += 1;
                                    break;
                                }
                            }
                            Some(Token::Newline | Token::Dedent) | None => return false,
                            _ => {}
                        }
                        offset += 1;
                    }
                }
                _ => return false,
            }
        }
    }

    /// The place before `::`: a local, then any mix of `.field` and `.[index]`, as ordinary name,
    /// member and select nodes.
    pub(crate) fn parse_place(&mut self) -> ParseResult<Expression> {
        // The root is a local, or `self` inside a method.
        let spelling = self.peek_spelling(0).unwrap_or_default().to_owned();
        let start = self.advance()?.span_id;
        let mut place = self.node(Expr::Name(spelling), start, start);
        while self.current() == Some(Token::Dot) {
            let dot = self.advance()?.span_id;
            if self.current() == Some(Token::BracketOpen) {
                let open = self.advance()?.span_id;
                let index = self.parse_expression()?;
                if self.current() != Some(Token::BracketClose) {
                    return Err(self.error("expected ']' after the index of a place"));
                }
                let close = self.advance()?.span_id;
                place = self.node(
                    Expr::Select(Box::new(ast::SelectExpression {
                        base: place,
                        dot,
                        open,
                        index,
                        length: None,
                        close,
                    })),
                    start,
                    close,
                );
                continue;
            }
            let name = self.parse_name()?;
            let end = name.span_id;
            place = self.node(
                Expr::Member {
                    base: Box::new(place),
                    name,
                },
                start,
                end,
            );
        }
        Ok(place)
    }

    /// The `::` after a place, or the operator and colon of a compound assignment.
    pub(crate) fn parse_assignment_operator(
        &mut self,
    ) -> ParseResult<(Option<Spanned<BinaryOperator>>, SpanId)> {
        let Some(operator) = self.current().and_then(compound_operator) else {
            return Ok((None, self.advance()?.span_id));
        };
        let compound = Spanned::new(operator, self.advance()?.span_id);
        Ok((Some(compound), self.advance()?.span_id))
    }

    pub(crate) fn parse_assignment(&mut self) -> ParseResult<Spanned<Statement>> {
        assert!(self.starts_assignment());
        let place = self.parse_place()?;
        let start = place.span_id;
        let (compound, operator) = self.parse_assignment_operator()?;
        let value = self.parse_expression()?;
        let end = value.span_id;
        Ok(self.node(
            Statement::Assign(AssignStatement {
                place,
                compound,
                operator,
                value,
            }),
            start,
            end,
        ))
    }

    pub(crate) fn parse_destructure(&mut self) -> ParseResult<Spanned<Statement>> {
        assert!(self.starts_destructure());
        let pattern = self.parse_variant_pattern()?;
        let start = pattern.span_id;
        assert_eq!(self.current(), Some(Token::ColonEq));
        let operator = Spanned::new(BindingOperator::ColonEquals, self.advance()?.span_id);
        let value = self.parse_expression()?;
        let end = value.span_id;
        Ok(self.node(
            Statement::Destructure(DestructureStatement {
                pattern,
                operator,
                value,
            }),
            start,
            end,
        ))
    }

    pub(crate) fn parse_variant_pattern(&mut self) -> ParseResult<Spanned<VariantPattern>> {
        self.parse_variant_pattern_nested(false)
    }

    pub(crate) fn parse_variant_pattern_nested(&mut self, nested: bool) -> ParseResult<Spanned<VariantPattern>> {
        if self.current() != Some(Token::Tag) {
            return Err(self.error(
                "expected a constructor pattern; nested patterns are unsupported",
            ));
        }
        let constructor = self.parse_name()?;
        let start = constructor.span_id;
        if self.current() != Some(Token::ParenOpen) {
            return Ok(Spanned::new(
                VariantPattern {
                    constructor,
                    fields: None,
                },
                start,
            ));
        }
        let open = self.advance()?.span_id;
        let mut binders = Vec::new();
        let mut selectors = Vec::new();
        let mut inner: Vec<Option<Box<Spanned<VariantPattern>>>> = Vec::new();
        let mut binder_required = true;
        for _ in self.position..=self.tokens.len() {
            self.checkpoint()?;
            match self.current() {
                // A line break between binders separates them, as a comma does.
                Some(Token::Newline) if !self.starts_statement(self.position) => {
                    self.advance()?;
                    binder_required = true;
                }
                Some(Token::ParenClose) => break,
                Some(Token::Id) if binder_required => {
                    binders.push(self.parse_name()?);
                    inner.push(None);
                    // `binder: field` binds the local `binder` to the field named `field`.
                    if self.current() == Some(Token::Colon) {
                        self.advance()?;
                        if self.current() == Some(Token::Tag) {
                            return Err(self.error(
                                "in a pattern the pattern comes first: `Done(Full(v): value)`; `value: Full(v)` is a construction",
                            ));
                        }
                        if self.current() != Some(Token::Id) {
                            return Err(self.error("expected a field name after the colon of a pattern binder"));
                        }
                        selectors.push(Some(self.parse_name()?));
                    } else {
                        selectors.push(None);
                    }
                    binder_required = false;
                }
                Some(Token::Comma) if !binder_required => {
                    self.advance()?;
                    binder_required = true;
                }
                // `Flaw(OutOfMemory)`: a pattern in place of a binder, in a case arm only.
                Some(Token::Tag) if nested && binder_required => {
                    let pattern = self.parse_variant_pattern_nested(true)?;
                    binders.push(pattern.value.constructor.clone());
                    inner.push(Some(Box::new(pattern)));
                    // `Full(v): value`: the pattern takes the field named `value`, as a binder does.
                    if self.current() == Some(Token::Colon) {
                        self.advance()?;
                        if self.current() != Some(Token::Id) {
                            return Err(self.error("expected a field name after the colon of a nested pattern"));
                        }
                        selectors.push(Some(self.parse_name()?));
                    } else {
                        selectors.push(None);
                    }
                    binder_required = false;
                }
                None | Some(Token::Then | Token::ColonEq | Token::Dedent) => break,
                _ => return Err(self.error("expected a flat pattern binder or ','; nested patterns are unsupported")),
            }
        }
        let close = if self.current() == Some(Token::ParenClose) {
            self.advance()?.span_id
        } else {
            self.report("expected ')' after pattern binders")?;
            self.missing_span()
        };
        let binders = self.node(binders, open, close);
        Ok(self.node(
            VariantPattern {
                constructor,
                fields: Some(PatternFields {
                    open,
                    binders,
                    selectors,
                    nested: inner,
                    close,
                }),
            },
            start,
            close,
        ))
    }
}
