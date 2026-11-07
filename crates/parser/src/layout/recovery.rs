use ast::{SpanTable, Spanned};
use lexer::TokenKind as Token;

use crate::{ParseFailure, ParseOptions};

/// Provisional matching for exactly one borrowed token slice.
///
/// This is evidence that a colon line belongs to a closed list, not recovered
/// nesting or an AST depth check. A hard boundary severs pending opens so a later
/// orphan `)` cannot retroactively turn an earlier declaration into an argument.
pub(super) struct RecoveryIndex<'tokens> {
    tokens: &'tokens [Spanned<Token>],
    closes: Vec<Option<usize>>,
    modified_headers: Vec<bool>,
}

struct RawOpen {
    index: usize,
    may_be_parameters: bool,
}

impl<'tokens> RecoveryIndex<'tokens> {
    /// Both temporary arrays are bounded by the checked raw token budget.
    pub fn new(
        source: &str,
        tokens: &'tokens [Spanned<Token>],
        spans: &SpanTable,
        options: ParseOptions<'_>,
    ) -> Result<Self, ParseFailure> {
        let mut result = Self {
            tokens,
            closes: vec![None; tokens.len()],
            modified_headers: vec![false; tokens.len()],
        };
        let mut opens: Vec<RawOpen> = Vec::new();
        let mut line_start = true;
        let mut inline_members = false;
        for (index, token) in tokens.iter().enumerate() {
            options.checkpoint(spans.get(token.span_id))?;
            if line_start && opens.is_empty() {
                inline_members = false;
            }
            if line_start {
                let expression = opens.last().is_some_and(|open| !open.may_be_parameters);
                result.modified_headers[index] =
                    expression && result.has_header_modifier(index, source, spans);
                if result.starts_unambiguous_statement(index) {
                    opens.clear();
                }
            }
            match token.value {
                Token::ParenOpen | Token::BracketOpen => {
                    let may_be_parameters = token.value == Token::ParenOpen
                        && opens.is_empty()
                        && result.may_be_parameters(index, inline_members, spans);
                    opens.push(RawOpen {
                        index,
                        may_be_parameters,
                    });
                }
                Token::ParenClose | Token::BracketClose => {
                    if opens.last().is_some_and(|open| {
                        super::closes_delimiter(tokens[open.index].value, token.value)
                    }) && let Some(open) = opens.pop()
                    {
                        assert!(open.index < index);
                        result.closes[open.index] = Some(index);
                    }
                }
                Token::Has if opens.is_empty() => inline_members = true,
                _ => {}
            }
            line_start = token.value == Token::Newline;
        }
        assert_eq!(tokens.len(), result.closes.len());
        Ok(result)
    }

    fn may_be_parameters(&self, open: usize, inline_members: bool, spans: &SpanTable) -> bool {
        assert_eq!(self.tokens[open].value, Token::ParenOpen);
        let Some(name) = open.checked_sub(1) else {
            return false;
        };
        if !matches!(self.tokens[name].value, Token::Id | Token::Tag) {
            return false;
        }
        if self.tokens[name].value == Token::Id
            && spans.get(self.tokens[name].span_id).end()
                != spans.get(self.tokens[open].span_id).start()
        {
            return false;
        }
        match name
            .checked_sub(1)
            .map(|previous| self.tokens[previous].value)
        {
            None | Some(Token::Newline | Token::Has) => true,
            Some(Token::Comma) => inline_members,
            _ => false,
        }
    }

    fn has_header_modifier(&self, index: usize, source: &str, spans: &SpanTable) -> bool {
        self.tokens[index].value == Token::Id
            && self.tokens.get(index + 1).is_some_and(|token| {
                matches!(
                    token.value,
                    Token::Own | Token::Ref | Token::Mut | Token::At
                ) || (token.value == Token::Id && spans.get(token.span_id).extract(source) == "end")
            })
    }

    /// Both matching and normalization use these same hard boundaries.
    /// `name own/ref/mut/@/end` is hard only inside an expression list, not parameters.
    pub fn starts_unambiguous_statement(&self, index: usize) -> bool {
        assert!(index < self.tokens.len());
        if self.modified_headers[index] {
            return true;
        }
        match self.tokens[index].value {
            Token::Return
            | Token::Private
            | Token::Use
            | Token::If
            | Token::When
            | Token::While
            | Token::For => true,
            Token::Pound => self
                .tokens
                .get(index + 1)
                .is_some_and(|token| token.value == Token::Private),
            Token::Id => self
                .tokens
                .get(index + 1)
                .is_some_and(|token| token.value == Token::ColonEq),
            Token::Tag => self
                .tokens
                .get(index + 1)
                .is_some_and(|token| matches!(token.value, Token::Is | Token::Has)),
            _ => false,
        }
    }

    pub fn is_unclosed(&self, open: usize) -> bool {
        assert!(open < self.tokens.len());
        assert!(matches!(
            self.tokens[open].value,
            Token::ParenOpen | Token::BracketOpen
        ));
        self.closes[open].is_none()
    }

    pub fn starts_declaration(&self, index: usize) -> bool {
        assert!(index < self.tokens.len());
        let after = if self
            .tokens
            .get(index + 1)
            .is_some_and(|token| token.value == Token::ParenOpen)
        {
            match self.closes[index + 1] {
                Some(close) => close + 1,
                None => return false,
            }
        } else {
            index + 1
        };
        let after = self.tokens.get(after).map(|token| token.value);
        match self.tokens[index].value {
            Token::Id => matches!(
                after,
                Some(
                    Token::Colon
                        | Token::ColonEq
                        | Token::Id
                        | Token::Tag
                        | Token::SelfTag
                        | Token::Is
                        | Token::In
                        | Token::Where
                        | Token::Own
                        | Token::Ref
                        | Token::Mut
                        | Token::At
                        | Token::ParenOpen
                        | Token::BracketOpen
                )
            ),
            Token::Tag => {
                matches!(after, Some(Token::Is | Token::Has))
                    || (after == Some(Token::ColonEq)
                        && self
                            .tokens
                            .get(index + 1)
                            .is_some_and(|token| token.value == Token::ParenOpen))
            }
            _ => false,
        }
    }
}

pub(super) fn indentation(source: &str, start: usize) -> usize {
    assert!(start <= source.len());
    assert!(source.is_char_boundary(start));
    source[..start]
        .rsplit('\n')
        .next()
        .expect("source line")
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .map(|byte| if byte == b'\t' { 4 } else { 1 })
        .sum()
}
