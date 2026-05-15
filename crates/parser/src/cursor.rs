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
    /// Stack of saved positions for [`Self::advance_pop`] progress assertions.
    /// See the section "Loop progress assertions" below.
    advance_stack: Vec<usize>,
    /// Optional cancellation hook invoked at every successful loop iteration
    /// (i.e. inside [`Self::advance_pop`]). The hook may panic to abort the
    /// parse — Salsa's `Cancelled` is the canonical use case. See "Cooperative
    /// cancellation" below.
    cancel_check: Option<&'t (dyn Fn() + 't)>,
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
            advance_stack: Vec::new(),
            cancel_check: None,
            errors: Vec::new(),
        }
    }

    /// Builder: install a cancellation hook to be invoked once per parse-loop
    /// iteration. The hook should panic (typically with `salsa::Cancelled`) to
    /// abort the parse mid-flight — the hook itself never returns a value.
    ///
    /// # Cooperative cancellation
    ///
    /// Layer 1 (progress assertions) guarantees the parser terminates on any
    /// input. Layer 2 (this hook) makes it *interruptible* — when a new edit
    /// arrives mid-parse, the salsa write triggers cancellation, and the very
    /// next loop iteration unwinds out of the parser via the hook's panic.
    /// Without it, an in-flight parse on stale input runs to completion before
    /// the new edit's compute can start.
    pub fn with_cancel_check(mut self, check: &'t (dyn Fn() + 't)) -> Self {
        self.cancel_check = Some(check);
        self
    }

    /// Invoke the cancellation hook (if installed). Called from
    /// [`Self::advance_pop`]; if the hook panics, the panic propagates up
    /// through the parser stack to the caller of `parse_*`.
    #[inline]
    fn check_cancellation(&self) {
        if let Some(check) = self.cancel_check {
            check();
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

    /// Peek at the next token past any intervening newlines AND indents,
    /// without consuming them. Used for Indent-as-continuation in expressions.
    #[inline]
    pub fn peek_past_indent(&self) -> Option<&Token<'src>> {
        let mut p = self.pos;
        let tokens = self.tokens;
        while p < tokens.len() && matches!(tokens[p].0, Token::Newline | Token::Indent) {
            p += 1;
        }
        tokens.get(p).map(|(t, _)| t)
    }

    /// Skip past all leading newline and indent tokens.
    /// Used when Indent should be treated as expression continuation.
    #[inline]
    pub fn skip_indents(&mut self) {
        let tokens = self.tokens;
        while self.pos < tokens.len()
            && matches!(tokens[self.pos].0, Token::Newline | Token::Indent)
        {
            self.pos += 1;
        }
        self.invalidate_cache();
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

    pub fn checkpoint(&self) -> usize {
        self.pos
    }

    pub fn rewind(&mut self, pos: usize) {
        self.pos = pos;
        self.invalidate_cache();
    }

    // ── Loop progress assertions ──────────────────────────────────────────
    //
    // Materializes the implicit "this loop body always consumes a token"
    // contract that recursive-descent parsers depend on. Wrap each parser
    // production called inside a loop:
    //
    //   loop {
    //       cursor.advance_push();
    //       match parse_thing(cursor) {
    //           Some(x) => { cursor.advance_pop(); /* consume x */ }
    //           None    => { cursor.advance_drop(); break; }
    //       }
    //   }
    //
    // `advance_pop` panics if the cursor did not move past the saved position.
    // `advance_drop` discards the saved position without checking — use it on
    // the "no match, exit loop" branch, where not advancing is expected.
    //
    // Rationale: an infinite parse loop is by far the worst failure mode for
    // an editor — silent, blocks the LSP, and recovery is impossible without
    // killing the process. Asserting progress at the call site converts that
    // class of bug into a panic with a precise stack trace that `catch_unwind`
    // in the LSP can surface as an LSP error, and that tests can lock in.
    // Ref: https://matklad.github.io/2025/12/28/parsing-advances.html

    /// Record the current position so a later [`Self::advance_pop`] can assert
    /// the cursor moved past it.
    #[inline]
    pub fn advance_push(&mut self) {
        self.advance_stack.push(self.pos);
    }

    /// Pop the most recent [`Self::advance_push`] and panic if the cursor did
    /// not advance past it. Use after a parser production succeeded inside a
    /// loop body.
    ///
    /// After the progress assertion succeeds, this also invokes the
    /// cancellation hook (if installed). This is the canonical place for the
    /// check: every productive parser iteration funnels through here, so the
    /// hook fires at most once per loop iteration but at least once per
    /// iteration of every loop guarded by a push/pop pair.
    #[inline]
    #[track_caller]
    pub fn advance_pop(&mut self) {
        let prev = self
            .advance_stack
            .pop()
            .expect("advance_pop called without matching advance_push");
        assert!(
            self.pos > prev,
            "parse loop made no progress at pos={prev} \
             (token={:?}); this is a parser bug — every iteration must consume at least one token",
            self.tokens.get(prev).map(|(t, _)| t),
        );
        self.check_cancellation();
    }

    /// Pop the most recent [`Self::advance_push`] without asserting progress.
    /// Use on the "no match, exit loop" branch where not advancing is intended.
    #[inline]
    pub fn advance_drop(&mut self) {
        self.advance_stack
            .pop()
            .expect("advance_drop called without matching advance_push");
    }

    // ── Interning ─────────────────────────────────────────────────────────

    #[inline(always)]
    pub fn intern(&mut self, s: &'src str) -> Intern<String> {
        Intern::<String>::from_ref(s)
    }
}
