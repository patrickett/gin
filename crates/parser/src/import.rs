use ast::{Import, ImportEntry, ImportMember, ImportSource, Name, Spanned};
use lexer::TokenKind as Token;

use crate::cursor::{Cursor, ParseResult};

impl Cursor<'_, '_> {
    pub fn parse_import(&mut self) -> ParseResult<Spanned<Import>> {
        assert_eq!(self.current(), Some(Token::Use));
        let start = self.advance()?.span_id;
        let mut entries = Vec::new();
        for _ in 0..self.tokens.len() {
            let entry = self.parse_import_entry()?;
            entries.push(entry);
            let ended_block = self.tokens[self.position - 1].value == Token::Dedent;
            if ended_block || self.current() != Some(Token::Comma) {
                let end = entries.last().expect("at least one import").span_id;
                if !ended_block {
                    self.require_line_end()?;
                }
                return Ok(self.node(Import { entries }, start, end));
            }
            self.advance()?;
        }
        unreachable!("each import entry consumes at least one token");
    }

    fn parse_import_entry(&mut self) -> ParseResult<Spanned<ImportEntry>> {
        let source = match self.current() {
            Some(Token::String) => {
                let span = self.advance()?.span_id;
                Spanned::new(ImportSource::Path, span)
            }
            Some(Token::Id | Token::SelfInstance) => {
                let path = self.parse_import_path(true)?;
                let first = path.first().expect("nonempty path").span_id;
                let last = path.last().expect("nonempty path").span_id;
                self.node(ImportSource::Module(path), first, last)
            }
            Some(Token::Tag) => {
                let name = self.parse_name()?;
                let span = name.span_id;
                Spanned::new(ImportSource::Current(name), span)
            }
            _ => return Err(self.error("expected a path or name after `use`")),
        };
        let start = source.span_id;
        let mut members = Vec::new();
        let mut block = false;
        if !matches!(source.value, ImportSource::Current(_)) {
            let next = self.tokens[self.position..]
                .iter()
                .position(|token| token.value != Token::Newline)
                .map(|offset| self.position + offset);
            block = next.is_some_and(|next| self.tokens[next].value == Token::Indent);
            if block {
                self.position = next.expect("confirmed indentation");
                self.advance()?;
                members = self.parse_import_members(true)?;
            } else if matches!(self.current(), Some(Token::Id | Token::Tag)) {
                members = self.parse_import_members(false)?;
            }
        }
        // A consumed dedent ends this statement. `as` on the next top-level
        // line must not be attached to the preceding import.
        let alias = if block {
            None
        } else {
            if !members.is_empty() && self.current() == Some(Token::As) {
                return Err(self.error("a selected import list cannot have a namespace alias"));
            }
            self.parse_import_entry_alias(&source.value)?
        };
        let end = alias.as_ref().map_or_else(
            || {
                members
                    .last()
                    .map_or(source.span_id, |member| member.span_id)
            },
            |alias| alias.span_id,
        );
        let entry = ImportEntry {
            source,
            members,
            alias,
        };
        Ok(self.node(entry, start, end))
    }

    fn parse_import_members(&mut self, block: bool) -> ParseResult<Vec<Spanned<ImportMember>>> {
        let mut members = Vec::new();
        for _ in 0..self.tokens.len() {
            if block {
                self.skip_newlines();
            }
            let path = self.parse_import_path(false)?;
            let start = path.first().expect("nonempty member path").span_id;
            let alias = self.parse_import_alias()?;
            let end = self.last_span();
            members.push(self.node(ImportMember { path, alias }, start, end));
            if self.current() == Some(Token::Comma) {
                self.advance()?;
                continue;
            }
            if block {
                if self.current() == Some(Token::Newline) {
                    self.skip_newlines();
                    if self.current() != Some(Token::Dedent) {
                        continue;
                    }
                }
                if self.current() != Some(Token::Dedent) {
                    return Err(self.error("expected a newline or ',' between import members"));
                }
                self.advance()?;
            }
            return Ok(members);
        }
        unreachable!("each selected member consumes at least one token");
    }

    fn parse_import_path(&mut self, module: bool) -> ParseResult<Vec<Name>> {
        let first = if module && self.current() == Some(Token::SelfInstance) {
            let span = self.advance()?.span_id;
            Spanned::new("self".to_owned(), span)
        } else {
            self.parse_name()?
        };
        let mut path = vec![first];
        for _ in 0..self.tokens.len() {
            if self.current() != Some(Token::Dot) {
                assert!(!path.is_empty());
                return Ok(path);
            }
            self.advance()?;
            path.push(self.parse_name()?);
        }
        unreachable!("each path component consumes at least one token");
    }

    fn parse_import_alias(&mut self) -> ParseResult<Option<Name>> {
        if self.current() != Some(Token::As) {
            return Ok(None);
        }
        self.advance()?;
        self.parse_name().map(Some)
    }

    fn parse_import_entry_alias(&mut self, source: &ImportSource) -> ParseResult<Option<Name>> {
        if self.current() == Some(Token::As)
            && !matches!(source, ImportSource::Current(_))
            && self.peek(1) != Some(Token::Id)
        {
            return Err(self.error("expected a lowercase namespace alias after `as`"));
        }
        self.parse_import_alias()
    }
}
