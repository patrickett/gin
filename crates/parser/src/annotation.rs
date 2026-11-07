//! Written header roles; no semantic types or inferred refinement subjects.

use diagnostic::Diagnostic;
use diagnostic::code::SyntaxCode;
use ast::{Annotation, SpanId, Spanned};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};
use crate::domain::DomainContext;
use crate::expr::ExpressionEnd;

impl Cursor<'_, '_> {
    pub fn parse_optional_annotation(
        &mut self,
        alternatives: bool,
    ) -> ParseResult<Option<Spanned<Annotation>>> {
        let annotation = match self.current() {
            Some(Token::Is) => self.parse_is_annotation()?,
            Some(Token::In) => self.parse_domain_annotation(None)?,
            Some(Token::Has) => {
                let has_keyword = self.advance()?.span_id;
                let bound = self.parse_header_part(ExpressionEnd::HeaderBase)?;
                let end = bound.span_id;
                self.node(Annotation::Bound { has_keyword, bound }, has_keyword, end)
            }
            Some(Token::And) => self.parse_baseless_refinement()?,
            Some(
                Token::Id
                | Token::Tag
                | Token::SelfTag
                | Token::Ref
                | Token::Mut
                | Token::Own
                | Token::At
                | Token::ParenOpen
                | Token::BracketOpen,
            ) => self.parse_expression_annotation()?,
            _ => return Ok(None),
        };
        if !alternatives && self.current() == Some(Token::Or) {
            return Err(self.error("result alternatives are written only on a function result"));
        }
        Ok(Some(annotation))
    }

    fn parse_baseless_refinement(&mut self) -> ParseResult<Spanned<Annotation>> {
        assert_eq!(self.current(), Some(Token::And));
        let start = self.tokens[self.position].span_id;
        let refinements = self.parse_domain_refinements(&mut DomainContext::Header)?;
        let end = refinements.last().expect("written and clause").span_id;
        Ok(self.node(
            Annotation::Refined {
                base: None,
                refinements,
            },
            start,
            end,
        ))
    }

    fn parse_is_annotation(&mut self) -> ParseResult<Spanned<Annotation>> {
        assert_eq!(self.current(), Some(Token::Is));
        let is_keyword = self.advance()?.span_id;
        if self.current() == Some(Token::In) {
            self.record(
                Diagnostic::new(
                    SyntaxCode::UnexpectedToken,
                    "domain header annotations use `in`, not `is in`",
                )
                .with_help("Write `value in Domain`; type definitions keep `Type is in Domain`.")
                .at_span(self.spans.get(is_keyword)),
            )?;
            return self.parse_domain_annotation(Some(is_keyword));
        }
        let mut context = DomainContext::Header;
        let predicate = self.parse_domain_predicate(&mut context)?;
        let refinements = self.parse_domain_refinements(&mut context)?;
        let end = refinements
            .last()
            .map_or(predicate.span_id, |clause| clause.span_id);
        let value = Annotation::Predicate {
            is_keyword,
            predicate,
            refinements,
        };
        Ok(self.node(value, is_keyword, end))
    }

    fn parse_domain_annotation(
        &mut self,
        is_keyword: Option<SpanId>,
    ) -> ParseResult<Spanned<Annotation>> {
        assert_eq!(self.current(), Some(Token::In));
        let in_keyword = self.advance()?.span_id;
        let domain = self.parse_written_domain(&mut DomainContext::Header, in_keyword)?;
        let start = is_keyword.unwrap_or(in_keyword);
        let end = domain.span_id;
        Ok(self.node(Annotation::Domain { is_keyword, domain }, start, end))
    }

    fn parse_expression_annotation(&mut self) -> ParseResult<Spanned<Annotation>> {
        let base = self.parse_header_part(ExpressionEnd::HeaderBase)?;
        let start = base.span_id;
        if self.current() != Some(Token::And) {
            return Ok(Spanned::new(Annotation::Expression(base), start));
        }
        let refinements = self.parse_domain_refinements(&mut DomainContext::Header)?;
        let end = refinements.last().expect("written and clause").span_id;
        Ok(self.node(
            Annotation::Refined {
                base: Some(base),
                refinements,
            },
            start,
            end,
        ))
    }
}
