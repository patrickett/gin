use std::cell::Cell;

use ast::span::{HasSpanId, Span, SpanId, SpanTable};
use internment::Intern;
use lexer::Token;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: SpanId,
}

impl HasSpanId for ParseError {
    fn span_id(&self) -> SpanId {
        self.span
    }
}

pub struct TokenCursor<'src, 't> {
    tokens: &'t [(Token<'src>, SpanId)],
    span_table: &'t mut SpanTable,
    pos: usize,
    last_consumed_pos: Option<usize>,
    sig_pos_cache: Cell<Option<usize>>,
    pub errors: Vec<ParseError>,
}

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn new(tokens: &'t [(Token<'src>, SpanId)], span_table: &'t mut SpanTable) -> Self {
        Self {
            tokens,
            span_table,
            pos: 0,
            last_consumed_pos: None,
            sig_pos_cache: Cell::new(None),
            errors: Vec::new(),
        }
    }

    // ── Significant access (auto-skip newlines on peek) ──────────────────

    #[inline(always)]
    fn invalidate_cache(&self) {
        self.sig_pos_cache.set(None);
    }

    fn significant_pos(&self) -> usize {
        if let Some(cached) = self.sig_pos_cache.get() {
            return cached;
        }
        let mut p = self.pos;
        while p < self.tokens.len() && matches!(self.tokens[p].0, Token::Newline) {
            p += 1;
        }
        self.sig_pos_cache.set(Some(p));
        p
    }

    #[inline(always)]
    pub fn peek(&self) -> Option<&Token<'src>> {
        let pos = self.significant_pos();
        self.tokens.get(pos).map(|(t, _)| t)
    }

    #[inline(always)]
    pub fn peek_span(&self) -> Option<SpanId> {
        let pos = self.significant_pos();
        self.tokens.get(pos).map(|(_, s)| *s)
    }

    /// Skip leading newlines, consume one token. Does NOT skip trailing
    /// newlines so that consume_trailing_newline can extend spans.
    #[inline(always)]
    pub fn advance(&mut self) -> Option<(Token<'src>, SpanId)> {
        let pos = self.significant_pos(); // uses cache, no redundant scan
        let item = self.tokens.get(pos)?;
        self.last_consumed_pos = Some(pos);
        self.pos = pos + 1;
        self.invalidate_cache();
        Some((item.0, item.1))
    }

    /// If the raw token at the current position is a newline, consume it so
    /// that last_consumed_span includes the newline span.
    #[inline(always)]
    pub fn consume_trailing_newline(&mut self) {
        if self.pos < self.tokens.len() && matches!(self.tokens[self.pos].0, Token::Newline) {
            self.last_consumed_pos = Some(self.pos);
            self.pos += 1;
            while self.pos < self.tokens.len() && matches!(self.tokens[self.pos].0, Token::Newline)
            {
                self.pos += 1;
            }
            self.invalidate_cache();
        }
    }

    #[inline(always)]
    pub fn is_at(&self, token: &Token<'src>) -> bool {
        self.peek() == Some(token)
    }

    #[inline(always)]
    pub fn eat(&mut self, token: &Token<'src>) -> bool {
        if self.is_at(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, token: &Token<'src>) -> Option<(Token<'src>, SpanId)> {
        if self.is_at(token) {
            self.advance()
        } else {
            let span = self.peek_span().unwrap_or(SpanId::INVALID);
            let found = self
                .peek()
                .map(|t| format!("{t:?}"))
                .unwrap_or_else(|| "end of input".to_string());
            self.errors.push(ParseError {
                message: format!("expected {:?}, found {found}", token),
                span,
            });
            None
        }
    }

    // ── Lookahead (raw offsets from current pos) ──────────────────────────

    #[inline(always)]
    pub fn peek_at(&self, offset: usize) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + offset).map(|(t, _)| t)
    }

    // ── Position / span helpers ───────────────────────────────────────────

    #[inline(always)]
    pub fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Get actual span data from a SpanId using the span_table
    pub fn get_span(&self, id: SpanId) -> Span {
        self.span_table.get(id)
    }

    #[inline(always)]
    pub fn span_at(&self, pos: usize) -> SpanId {
        self.tokens
            .get(pos)
            .map(|(_, s)| *s)
            .unwrap_or_else(|| self.eof_span())
    }

    #[inline(always)]
    pub fn span_table(&self) -> &SpanTable {
        self.span_table
    }

    #[inline(always)]
    pub fn span_table_mut(&mut self) -> &mut SpanTable {
        self.span_table
    }

    /// Merge two span IDs into a new span ID that covers both.
    /// The merged span is automatically added to the span table.
    pub fn merge_span(&mut self, a: SpanId, b: SpanId) -> SpanId {
        self.span_table.merge(a, b)
    }

    #[inline(always)]
    pub fn current_span(&self) -> SpanId {
        self.span_at(self.pos)
    }

    pub fn last_consumed_span(&self) -> SpanId {
        match self.last_consumed_pos {
            Some(p) => self.tokens[p].1,
            None => self.eof_span(),
        }
    }

    pub fn eof_span(&self) -> SpanId {
        self.tokens
            .last()
            .map(|(_, id)| {
                // Return a zero-length span at the end of the last span
                let _span = self.span_table.get(*id);
                // We need to insert a new span for the EOF position
                // For now, return INVALID since we can't insert into span_table here
                SpanId::INVALID
            })
            .unwrap_or(SpanId::INVALID)
    }

    #[allow(dead_code)]
    fn source_len(&self) -> usize {
        self.tokens
            .last()
            .map(|(_, id)| {
                let span = self.span_table.get(*id);
                span.end
            })
            .unwrap_or(0)
    }

    #[inline(always)]
    pub fn skip_newlines(&mut self) {
        let tokens = self.tokens;
        while self.pos < tokens.len() && matches!(tokens[self.pos].0, Token::Newline) {
            self.pos += 1;
        }
        self.invalidate_cache();
    }

    // ── Error handling ────────────────────────────────────────────────────

    pub fn error(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors.push(ParseError {
            message: message.into(),
            span,
        });
    }

    // ── Checkpoint / rewind ───────────────────────────────────────────────

    // TODO: Add advance_push/advance_pop/advance_drop assertion API to catch
    // infinite parse loops at the exact call site rather than via fuel or timeouts.
    //
    // Design: Before each loop iteration or recursive Pratt call, call advance_push()
    // to record the current position. After the body, advance_pop() asserts that
    // position increased (i.e., at least one token was consumed). If parsing bails
    // without consuming (e.g., no infix operator found), advance_drop() discards the
    // assertion without failing.
    //
    //   // In Pratt loop:
    //   cursor.advance_push();
    //   let rhs = parse_expression(cursor, right_binding_power);
    //   cursor.advance_pop(); // panics if cursor didn't advance
    //
    //   // In "no match" branch:
    //   cursor.advance_drop(); // OK, we didn't expect to advance
    //
    // Rationale: The current parser has a single progress check in parse_body_exprs
    // (control.rs:92-98) that detects stalls after the fact and forces an advance.
    // This works but produces unhelpful "expression parser made no progress" errors
    // far from the actual buggy parse function. The push/pop pattern materializes
    // the implicit "does this function always consume a token?" contract directly
    // in the source, making infinite loops fail fast with an accurate stack trace.
    //
    // Implementation: Add a `advance_stack: Vec<usize>` field to TokenCursor.
    // - advance_push(): push self.pos()
    // - advance_pop(): pop and assert popped < self.pos()
    // - advance_drop(): pop without assertion
    // Ref: https://matklad.github.io/2025/12/28/parsing-advances.html

    pub fn checkpoint(&self) -> usize {
        self.pos
    }

    pub fn rewind(&mut self, pos: usize) {
        self.pos = pos;
        self.invalidate_cache();
    }

    // ── Interning ─────────────────────────────────────────────────────────

    #[inline(always)]
    pub fn intern(&mut self, s: &'src str) -> Intern<String> {
        Intern::<String>::from_ref(s)
    }
}
