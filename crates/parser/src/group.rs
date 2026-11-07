//! Written group paths only; no binding, authority, lifetime, or disjointness inference.

use ast::{GroupPath, Spanned};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};

impl Cursor<'_, '_> {
    pub(crate) fn parse_optional_group(&mut self) -> ParseResult<Option<Spanned<GroupPath>>> {
        if self.current() != Some(Token::CurlyOpen) {
            return Ok(None);
        }
        let open = self.advance()?.span_id;
        let mut group = GroupPath {
            open,
            components: Vec::new(),
            close: None,
        };
        let mut end = open;
        let mut component_required = true;
        for _ in self.position..=self.tokens.len() {
            self.checkpoint()?;
            match self.current() {
                Some(Token::CurlyClose) => {
                    if component_required {
                        self.report("expected a group path component")?;
                    }
                    end = self.advance()?.span_id;
                    group.close = Some(end);
                    return Ok(Some(self.node(group, open, end)));
                }
                Some(token)
                    if component_required
                        && (matches!(token, Token::Id | Token::Tag)
                            || token.keyword().is_some()) =>
                {
                    self.parse_group_component(&mut group)?;
                    end = self.last_span();
                    component_required = false;
                }
                Some(Token::Dot) => {
                    if component_required {
                        self.report("expected a group path component before '.'")?;
                    }
                    end = self.advance()?.span_id;
                    component_required = true;
                }
                _ => {
                    let message = if component_required {
                        "expected a group path component and '}'"
                    } else {
                        "expected '}' after group path"
                    };
                    self.report(message)?;
                    return Ok(Some(self.node(group, open, end)));
                }
            }
        }
        unreachable!("a group step consumes a token or returns");
    }

    fn parse_group_component(&mut self, group: &mut GroupPath) -> ParseResult<()> {
        let root = group.components.is_empty();
        match self.current() {
            Some(Token::Id) => {}
            Some(Token::Tag) if !root => {}
            Some(Token::Tag) => self.report("a group path root must be a lowercase identifier")?,
            Some(token) if token.keyword().is_some() => {
                self.report("keywords are not group path identifiers")?;
            }
            _ => unreachable!("group component dispatch"),
        }
        let text = self.current_span().extract(self.source).to_owned();
        let span = self.advance()?.span_id;
        group.components.push(Spanned::new(text, span));
        Ok(())
    }
}
