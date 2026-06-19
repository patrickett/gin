use crate::cursor::TokenCursor;
use ast::{ModPath, Spanned};
use internment::Intern;
use lexer::Token;

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_id(&mut self) -> Option<Intern<String>> {
        match self.peek()? {
            Token::Id(name) => {
                let id = self.intern(name);
                self.advance();
                Some(id)
            }
            _ => None,
        }
    }

    /// Internal: parse a dotted path where the root token is accepted by
    /// `accept_root` and subsequent path segments (after `.`) are accepted by
    /// `accept_segment`.
    fn parse_path_matching(
        &mut self,
        mut accept_root: impl FnMut(Token<'src>) -> Option<&'src str>,
        mut accept_segment: impl FnMut(Token<'src>) -> Option<&'src str>,
    ) -> Option<Spanned<ModPath>> {
        let start_span = self.peek_span()?;

        let t = *self.peek()?;
        let root = {
            let name = accept_root(t)?;
            let id = self.intern(name);
            self.advance();
            id
        };
        let root_span = start_span;

        let mut segments = Vec::new();
        let mut segment_spans = Vec::new();
        let mut end_span = start_span;

        while self.is_at(&Token::Dot) {
            if let Some(next) = self.peek_at(1)
                && let Some(name) = accept_segment(*next)
            {
                self.advance(); // consume Dot
                if let Some((_, span)) = self.advance() {
                    segments.push(self.intern(name));
                    segment_spans.push(span);
                    end_span = span;
                }
            } else {
                break;
            }
        }

        let span = if end_span != start_span {
            self.merge_span(start_span, end_span)
        } else {
            start_span
        };

        Some(Spanned::new(
            ModPath::new_with_spans(root, root_span, segments, segment_spans),
            span,
        ))
    }

    /// Parse a path starting with a lowercase identifier (`foo.bar.Baz`).
    pub fn parse_path(&mut self) -> Option<Spanned<ModPath>> {
        self.parse_path_matching(
            |t| match t {
                Token::Id(n) => Some(n),
                _ => None,
            },
            |t| match t {
                Token::Id(n) | Token::Tag(n) => Some(n),
                _ => None,
            },
        )
    }

    /// Parse `Tag.id[.id...]` — a tag-rooted method or module path.
    /// Returns `None` if there is no segment after the root tag.
    pub fn parse_tag_path(&mut self) -> Option<Spanned<ModPath>> {
        let result = self.parse_path_matching(
            |t| match t {
                Token::Tag(n) => Some(n),
                _ => None,
            },
            |t| match t {
                Token::Id(n) => Some(n),
                _ => None,
            },
        )?;
        // Require at least one segment (e.g. `Map.keys`, not bare `Map`).
        if result.value.segments.is_empty() {
            return None;
        }
        Some(result)
    }

    /// Parse `Tag.Tag[.Tag...]` — a qualified variant path.
    /// Returns `None` if there is no segment after the root tag.
    pub fn parse_tag_variant_path(&mut self) -> Option<Spanned<ModPath>> {
        let result = self.parse_path_matching(
            |t| match t {
                Token::Tag(n) => Some(n),
                _ => None,
            },
            |t| match t {
                Token::Tag(n) => Some(n),
                _ => None,
            },
        )?;
        // Require at least one segment (e.g. `Http.Status`, not bare `Http`).
        if result.value.segments.is_empty() {
            return None;
        }
        Some(result)
    }
}
