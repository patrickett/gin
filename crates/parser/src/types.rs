//! Source type occurrences, never assembled definitions or synthesized methods.

use ast::{SpanId, Spanned, TypeBody, TypeDeclaration, TypeMember};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};
use crate::declaration::{BindingContext, ParsedBinding};

pub(crate) enum ParsedType {
    Complete(Spanned<TypeDeclaration>),
    Body(PendingHas),
}

pub(crate) struct PendingHas {
    pub declaration: TypeDeclaration,
    pub keyword: SpanId,
    pub members: Vec<Spanned<TypeMember>>,
}

impl Cursor<'_, '_> {
    pub fn starts_type(&self) -> bool {
        if self.current() != Some(Token::Tag) {
            return false;
        }
        let after = if self.peek(1) == Some(Token::ParenOpen) {
            match self.parenthesis_closes[self.position + 1] {
                Some(close) => close + 1,
                None => return false,
            }
        } else {
            self.position + 1
        };
        matches!(
            self.tokens.get(after).map(|token| token.value),
            Some(Token::Is | Token::Has)
        )
    }

    pub fn parse_type(&mut self) -> ParseResult<ParsedType> {
        assert!(self.starts_type());
        let name = self.parse_name()?;
        let start = name.span_id;
        let parameters = if self.current() == Some(Token::ParenOpen) {
            Some(self.parse_parameters(false)?)
        } else {
            None
        };
        let keyword = self.advance()?;
        assert!(matches!(keyword.value, Token::Is | Token::Has));
        if keyword.value == Token::Is {
            let body = self.parse_is_body(keyword.span_id)?;
            let end = body.span_id;
            let declaration = TypeDeclaration {
                name,
                parameters,
                body,
            };
            return Ok(ParsedType::Complete(self.node(declaration, start, end)));
        }
        let declaration = TypeDeclaration {
            name,
            parameters,
            body: Spanned::new(TypeBody::Has(Vec::new()), keyword.span_id),
        };
        let mut body = PendingHas {
            declaration,
            keyword: keyword.span_id,
            members: Vec::new(),
        };
        // `Type has TraitA and TraitB` names the traits the type provides; its own members
        // follow on the indented lines below.
        if self.current() == Some(Token::Tag) {
            body.members = self.parse_provisions()?;
        }
        if self.current() == Some(Token::Newline) {
            if self.open_type_body()? {
                return Ok(ParsedType::Body(body));
            }
        } else if self.current().is_some() {
            if !body.members.is_empty() {
                return Err(
                    self.error("put the members of a type that provides traits on indented lines")
                );
            }
            body.members = self.parse_inline_members()?;
        }
        self.require_line_end()?;
        Ok(ParsedType::Complete(self.close_has(body)))
    }

    fn open_type_body(&mut self) -> ParseResult<bool> {
        assert_eq!(self.current(), Some(Token::Newline));
        let next = self.tokens[self.position..]
            .iter()
            .position(|token| token.value != Token::Newline)
            .map(|offset| self.position + offset);
        let Some(next) = next else {
            return Ok(false);
        };
        if self.tokens[next].value == Token::Indent {
            self.position = next;
            self.advance()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn parse_provisions(&mut self) -> ParseResult<Vec<Spanned<TypeMember>>> {
        let mut provisions = Vec::new();
        for _ in self.position..self.tokens.len() {
            if self.current() != Some(Token::Tag) {
                return Err(self.error("expected a trait name"));
            }
            if self.peek(1) == Some(Token::ParenOpen) {
                let applied = self.parse_header_part(crate::expr::ExpressionEnd::HeaderBase)?;
                let span = applied.span_id;
                provisions.push(Spanned::new(TypeMember::ProvisionApplied(applied), span));
                if self.current() != Some(Token::And) {
                    return Ok(provisions);
                }
                self.advance()?;
                continue;
            }
            let name = self.parse_name()?;
            let span = name.span_id;
            provisions.push(Spanned::new(TypeMember::Provision(name), span));
            if self.current() != Some(Token::And) {
                return Ok(provisions);
            }
            self.advance()?;
        }
        Err(self.error("expected a trait name after 'and'"))
    }

    fn parse_inline_members(&mut self) -> ParseResult<Vec<Spanned<TypeMember>>> {
        let mut members = Vec::new();
        for _ in self.position..self.tokens.len() {
            if self.current() != Some(Token::Id) {
                return Err(self.error("expected a simple named has member"));
            }
            let ParsedBinding::Complete(binding) = self.parse_binding(BindingContext::Member)?
            else {
                return Err(
                    self.error("put a member with an indented body on its own has-body line")
                );
            };
            members.push(Spanned::new(
                TypeMember::Binding(Box::new(binding.value)),
                binding.span_id,
            ));
            if self.current() != Some(Token::Comma) {
                return Ok(members);
            }
            self.advance()?;
        }
        Err(self.error("expected a has member after ','"))
    }

    pub fn close_has(&mut self, mut body: PendingHas) -> Spanned<TypeDeclaration> {
        let start = body.declaration.name.span_id;
        let end = body
            .members
            .last()
            .map_or(body.keyword, |member| member.span_id);
        body.declaration.body = self.node(TypeBody::Has(body.members), body.keyword, end);
        self.node(body.declaration, start, end)
    }
}
