//! Written domains and predicates, without normalization to semantic bounds.

use diagnostic::Diagnostic;
use diagnostic::code::SyntaxCode;
use ast::{
    BinaryOperator, DomainBase, Expr, Expression, Refinement, RefinementPredicate, SpanId, Spanned,
    TypeBody, TypeDomain,
};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseError, ParseResult};
use crate::expr::{ExpressionEnd, TypeLayout};
use crate::operators::{binary_operator, is_comparison, precedence};

/// The same domain grammar has different newline ownership in type bodies
/// and headers. Headers never consume the next parameter or binding line.
pub(crate) enum DomainContext<'layout> {
    Type(&'layout mut TypeLayout),
    Header,
}

impl DomainContext<'_> {
    pub fn at_token(&mut self, cursor: &mut Cursor<'_, '_>, token: Token) -> ParseResult<bool> {
        match self {
            Self::Type(layout) => layout.at_token(cursor, token),
            Self::Header => Ok(cursor.current() == Some(token)),
        }
    }

    fn peek_token(&self, cursor: &Cursor<'_, '_>) -> ParseResult<Option<Token>> {
        match self {
            Self::Type(layout) => layout.peek_token(cursor),
            Self::Header => Ok(cursor.current()),
        }
    }

    pub fn expression(
        &mut self,
        cursor: &mut Cursor<'_, '_>,
        end: ExpressionEnd,
    ) -> ParseResult<Expression> {
        match self {
            Self::Type(layout) => cursor.parse_type_part(layout, end),
            Self::Header => cursor.parse_header_part(end.for_header()),
        }
    }
}

impl Cursor<'_, '_> {
    pub fn parse_is_body(&mut self, keyword: SpanId) -> ParseResult<Spanned<TypeBody>> {
        let mut layout = TypeLayout::default();
        let mut context = DomainContext::Type(&mut layout);
        let body = if context.at_token(self, Token::In)? {
            let in_keyword = self.advance()?.span_id;
            TypeBody::Domain(self.parse_written_domain(&mut context, in_keyword)?)
        } else {
            let expression_start = self.position;
            let expression = context.expression(self, ExpressionEnd::TypeValue)?;
            if context.at_token(self, Token::To)? {
                self.require_plain_domain_bound(expression_start)?;
                TypeBody::Domain(self.parse_domain_tail(&mut context, None, expression)?)
            } else {
                TypeBody::Is(expression)
            }
        };
        let end = match &body {
            TypeBody::Is(expression) => expression.span_id,
            TypeBody::Domain(domain) => domain.span_id,
            TypeBody::Has(_) => unreachable!("is has no member list"),
        };
        layout.close(self)?;
        Ok(self.node(body, keyword, end))
    }

    fn require_plain_domain_bound(&self, start: usize) -> ParseResult<()> {
        assert!(start <= self.position);
        for token in &self.tokens[start..self.position] {
            self.options.checkpoint(self.spans.get(token.span_id))?;
            if matches!(
                token.value,
                Token::Ref | Token::Mut | Token::Own | Token::At
            ) {
                return Err(ParseError::Syntax(
                    Diagnostic::new(
                        SyntaxCode::UnexpectedToken,
                        "type prefixes are not supported in numeric domain bounds",
                    )
                    .at_span(self.spans.get(token.span_id)),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn parse_written_domain(
        &mut self,
        context: &mut DomainContext<'_>,
        in_keyword: SpanId,
    ) -> ParseResult<Spanned<TypeDomain>> {
        let lower = context.expression(self, ExpressionEnd::DomainBound)?;
        self.parse_domain_tail(context, Some(in_keyword), lower)
    }

    fn parse_domain_tail(
        &mut self,
        context: &mut DomainContext<'_>,
        in_keyword: Option<SpanId>,
        lower: Expression,
    ) -> ParseResult<Spanned<TypeDomain>> {
        let start = lower.span_id;
        let base = if context.at_token(self, Token::To)? {
            let to_keyword = self.advance()?.span_id;
            let upper = context.expression(self, ExpressionEnd::DomainBound)?;
            let end = upper.span_id;
            self.node(
                DomainBase::Range {
                    lower,
                    to_keyword,
                    upper,
                },
                start,
                end,
            )
        } else {
            assert!(in_keyword.is_some(), "only ranges may omit in");
            Spanned::new(DomainBase::Expression(lower), start)
        };
        let refinements = self.parse_domain_refinements(context)?;
        let start = in_keyword.unwrap_or(base.span_id);
        let end = refinements
            .last()
            .map_or(base.span_id, |clause| clause.span_id);
        let domain = TypeDomain {
            in_keyword,
            base,
            refinements,
        };
        Ok(self.node(domain, start, end))
    }

    pub(crate) fn parse_domain_refinements(
        &mut self,
        context: &mut DomainContext<'_>,
    ) -> ParseResult<Vec<Spanned<Refinement>>> {
        let mut refinements = Vec::new();
        for _ in self.position..self.tokens.len() {
            if !context.at_token(self, Token::And)? {
                break;
            }
            let and_keyword = self.advance()?.span_id;
            let predicate = self.parse_domain_predicate(context)?;
            let end = predicate.span_id;
            refinements.push(self.node(
                Refinement {
                    and_keyword,
                    predicate,
                },
                and_keyword,
                end,
            ));
        }
        Ok(refinements)
    }

    pub(crate) fn parse_domain_predicate(
        &mut self,
        context: &mut DomainContext<'_>,
    ) -> ParseResult<Spanned<RefinementPredicate>> {
        if context.at_token(self, Token::In)? {
            let domain = self.parse_implicit_domain(context)?;
            let span = domain.span_id;
            return Ok(Spanned::new(
                RefinementPredicate::ImplicitDomain(domain),
                span,
            ));
        }
        let token = context.peek_token(self)?;
        let comparison = token
            .and_then(binary_operator)
            .filter(|operator| is_comparison(*operator));
        if let Some(operator) = comparison {
            let matched = context.at_token(self, token.expect("comparison token"))?;
            assert!(matched, "the inspected comparison has not moved");
            let operator = Spanned::new(operator, self.advance()?.span_id);
            let start = operator.span_id;
            let right = self.parse_implicit_operand(context, operator.value)?;
            let end = right.span_id;
            return Ok(self.node(
                RefinementPredicate::ImplicitComparison { operator, right },
                start,
                end,
            ));
        }
        let expression = context.expression(self, ExpressionEnd::Refinement)?;
        let span = expression.span_id;
        Ok(Spanned::new(
            RefinementPredicate::Expression(expression),
            span,
        ))
    }

    fn parse_implicit_domain(
        &mut self,
        context: &mut DomainContext<'_>,
    ) -> ParseResult<Spanned<TypeDomain>> {
        assert!(context.at_token(self, Token::In)?);
        let in_keyword = self.advance()?.span_id;
        let lower = context.expression(self, ExpressionEnd::DomainBound)?;
        let start = lower.span_id;
        let base = if context.at_token(self, Token::To)? {
            let to_keyword = self.advance()?.span_id;
            let upper = context.expression(self, ExpressionEnd::DomainBound)?;
            let end = upper.span_id;
            self.node(
                DomainBase::Range {
                    lower,
                    to_keyword,
                    upper,
                },
                start,
                end,
            )
        } else {
            Spanned::new(DomainBase::Expression(lower), start)
        };
        // Only the base is owned here; further `and` clauses stay in the outer list.
        let end = base.span_id;
        Ok(self.node(
            TypeDomain {
                in_keyword: Some(in_keyword),
                base,
                refinements: Vec::new(),
            },
            in_keyword,
            end,
        ))
    }

    fn parse_implicit_operand(
        &mut self,
        context: &mut DomainContext<'_>,
        comparison: BinaryOperator,
    ) -> ParseResult<Expression> {
        assert!(is_comparison(comparison));
        let right = context.expression(self, ExpressionEnd::Refinement)?;
        if let Expr::Binary { operator, .. } = &right.value
            && precedence(operator.value) <= precedence(comparison)
        {
            let message = if is_comparison(operator.value) {
                "comparison chains require explicit grouping"
            } else {
                "abbreviated comparisons require grouping or an explicit subject"
            };
            return Err(ParseError::Syntax(
                Diagnostic::new(SyntaxCode::UnexpectedToken, message)
                    .at_span(self.spans.get(operator.span_id)),
            ));
        }
        Ok(right)
    }
}
