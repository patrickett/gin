//! Owned layout for `is` expressions and their domain/refinement components.
//! Other expression contexts retain their newline and delimiter rules.

use ast::BinaryOperator;
use lexer::TokenKind as Token;

use super::{Frame, FrameKind, ReductionOperator};
use crate::cursor::{Cursor, ParseResult};

#[derive(Default)]
pub(crate) struct TypeLayout {
    depth: usize,
}

struct LayoutRun {
    next: usize,
    depth: usize,
}

impl TypeLayout {
    pub(super) fn continue_root(
        &mut self,
        cursor: &mut Cursor<'_, '_>,
        frame: &Frame,
    ) -> ParseResult<bool> {
        assert!(matches!(frame.kind, FrameKind::Root));
        assert!(self.depth <= usize::from(cursor.options.limits.body_depth_max));
        if !matches!(
            cursor.current(),
            Some(Token::Newline | Token::Indent | Token::Dedent)
        ) {
            return Ok(false);
        }
        let run = self.peek_layout(cursor)?;
        let next_kind = cursor.tokens.get(run.next).map(|token| token.value);
        let continue_operator = next_kind == Some(Token::Or) && !frame.needs_operand();
        let awaiting_alternative = frame.operators.last().is_none_or(|operator| {
            matches!(
                operator.value,
                ReductionOperator::Binary(BinaryOperator::Or)
            )
        });
        let continue_operand = frame.needs_operand()
            && awaiting_alternative
            && run.depth > 0
            && next_kind.is_some()
            && !cursor.starts_statement(run.next);
        // Trailing trivia must put a missing alternative at the actual EOF
        // insertion point, retaining the type owner for editor queries.
        let continue_eof = frame.needs_operand() && awaiting_alternative && next_kind.is_none();
        if !continue_operator && !continue_operand && !continue_eof {
            return Ok(false);
        }
        self.consume_layout(cursor, run)?;
        Ok(true)
    }

    pub fn at_token(&mut self, cursor: &mut Cursor<'_, '_>, token: Token) -> ParseResult<bool> {
        assert!(!matches!(
            token,
            Token::Newline | Token::Indent | Token::Dedent
        ));
        let run = self.peek_layout(cursor)?;
        if cursor.tokens.get(run.next).map(|token| token.value) != Some(token) {
            return Ok(false);
        }
        self.consume_layout(cursor, run)?;
        assert_eq!(cursor.current(), Some(token));
        Ok(true)
    }

    pub fn peek_token(&self, cursor: &Cursor<'_, '_>) -> ParseResult<Option<Token>> {
        let run = self.peek_layout(cursor)?;
        Ok(cursor.tokens.get(run.next).map(|token| token.value))
    }

    fn peek_layout(&self, cursor: &Cursor<'_, '_>) -> ParseResult<LayoutRun> {
        let mut next = cursor.position;
        let mut depth = self.depth;
        for token in &cursor.tokens[cursor.position..] {
            cursor.options.checkpoint(cursor.spans.get(token.span_id))?;
            match token.value {
                Token::Newline => {}
                Token::Indent => depth += 1,
                Token::Dedent if depth > 0 => depth -= 1,
                _ => break,
            }
            next += 1;
        }
        assert!(next <= cursor.tokens.len());
        assert!(depth <= usize::from(cursor.options.limits.body_depth_max));
        Ok(LayoutRun { next, depth })
    }

    fn consume_layout(&mut self, cursor: &mut Cursor<'_, '_>, run: LayoutRun) -> ParseResult<()> {
        assert!(cursor.position <= run.next);
        assert!(run.next <= cursor.tokens.len());
        for _ in cursor.position..run.next {
            cursor.advance()?;
        }
        self.depth = run.depth;
        assert_eq!(cursor.position, run.next);
        Ok(())
    }

    pub fn close(&mut self, cursor: &mut Cursor<'_, '_>) -> ParseResult<()> {
        assert!(self.depth <= usize::from(cursor.options.limits.body_depth_max));
        cursor.require_line_end()?;
        for _ in cursor.position..cursor.tokens.len() {
            if self.depth == 0 {
                return Ok(());
            }
            match cursor.current() {
                Some(Token::Newline) => {}
                Some(Token::Dedent) => self.depth -= 1,
                _ => return Err(cursor.error("expected the end of the multiline type expression")),
            }
            cursor.advance()?;
        }
        assert_eq!(
            self.depth, 0,
            "normalized indentation closes every type body"
        );
        Ok(())
    }
}
