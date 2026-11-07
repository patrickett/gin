use diagnostic::code::{LexCode, SyntaxCode};
use diagnostic::{Diagnostic, DiagnosticCode};
use ast::{CommentKind, Span, SpanId, SpanTable, Spanned};
use lexer::{Lexer, Token as RawToken, TokenKind as Token};

use crate::{ParseFailure, ParseLimit, ParseOptions};

pub(crate) type ParseResult<T> = Result<T, ParseError>;

/// The extent of a `"` string in the source: from the opening quote to just past the closing one
/// (or to the end of the line when there is none), and what it holds.
struct FormatString {
    start: usize,
    end: usize,
    /// A `\(` opens an interpolation hole, which is not supported yet.
    hole: bool,
    closed: bool,
}

impl FormatString {
    /// `\"` and `\\` are escapes and `\(` opens a hole; the text ends at the next `"` or at the
    /// end of the line.
    fn scan(source: &str, start: usize) -> Self {
        let bytes = source.as_bytes();
        let mut at = start + 1;
        let mut hole = false;
        while let Some(&byte) = bytes.get(at) {
            match byte {
                b'"' => {
                    return Self {
                        start,
                        end: at + 1,
                        hole,
                        closed: true,
                    };
                }
                b'\n' => break,
                b'\\' => match bytes.get(at + 1) {
                    Some(b'"' | b'\\') => at += 2,
                    Some(b'(') => {
                        hole = true;
                        at += 2;
                    }
                    _ => at += 1,
                },
                _ => at += 1,
            }
        }
        Self {
            start,
            end: at,
            hole,
            closed: false,
        }
    }
}

pub(crate) enum ParseError {
    Syntax(Diagnostic),
    Abort(ParseFailure),
}

impl From<ParseFailure> for ParseError {
    fn from(failure: ParseFailure) -> Self {
        Self::Abort(failure)
    }
}

pub(crate) struct Cursor<'source, 'cancel> {
    pub source: &'source str,
    pub tokens: Vec<Spanned<Token>>,
    pub parenthesis_closes: Vec<Option<usize>>,
    pub statement_starts: Vec<bool>,
    pub position: usize,
    pub expression_layout_end: Option<usize>,
    /// The first operand of the next expression: a leading `is` test the caller already read.
    pub(crate) expression_seed: Option<crate::expr::Value>,
    pub spans: SpanTable,
    pub comments: Vec<Spanned<CommentKind>>,
    pub diagnostics: Vec<Diagnostic>,
    pub options: ParseOptions<'cancel>,
}

impl<'source, 'cancel> Cursor<'source, 'cancel> {
    /// Lex into `spans`, an empty table whose id range the caller chose.
    pub fn lex_in(
        source: &'source str,
        options: ParseOptions<'cancel>,
        spans: SpanTable,
    ) -> Result<Self, ParseFailure> {
        options.checkpoint(Span::new(0, 0))?;
        if source.len() > options.limits.source_bytes_max as usize {
            return Err(ParseFailure::LimitExceeded {
                limit: ParseLimit::SourceBytes,
                maximum: options.limits.source_bytes_max,
                span: Span::new(0, 0),
            });
        }
        let mut cursor = Self {
            source,
            tokens: Vec::new(),
            parenthesis_closes: Vec::new(),
            statement_starts: Vec::new(),
            position: 0,
            expression_layout_end: None,
            expression_seed: None,
            spans,
            comments: Vec::new(),
            diagnostics: Vec::new(),
            options,
        };
        let mut lexer = Lexer::new(source);
        for count in 0..=options.limits.tokens_max {
            options.checkpoint(Span::new(0, 0))?;
            let Some(token) = lexer.next_raw() else {
                return cursor.lex_finish();
            };
            if count == options.limits.tokens_max {
                return Err(ParseFailure::LimitExceeded {
                    limit: ParseLimit::Tokens,
                    maximum: options.limits.tokens_max,
                    span: token.span,
                });
            }
            // A `"` opens a format string, whose text is read here, from the source, not lexed:
            // an apostrophe or a character that is no token is just text in it.
            if token.kind == Token::FormatStringDelim {
                let string = FormatString::scan(source, token.span.to_range().start);
                lexer.skip_to(string.end);
                cursor.finish_format_string(string)?;
                continue;
            }
            cursor.accept_raw_token(token)?;
        }
        unreachable!("the token budget either finishes or refuses the parse");
    }

    /// The text of a format string as one string token. A hole is reported, not parsed: the
    /// interpolation grammar is not decided.
    // TODO(interpolation): choose the hole grammar and a streaming, budgeted representation
    // before accepting holes; a source delimiter must not trigger an unbounded token batch.
    fn finish_format_string(&mut self, string: FormatString) -> Result<(), ParseFailure> {
        let written = Span::new(string.start, string.end);
        if !string.closed {
            self.record(
                Diagnostic::new(
                    DiagnosticCode::Lex(LexCode::UnclosedString),
                    "unclosed string literal",
                )
                .at_span(written),
            )?;
        } else if string.hole {
            self.record(
                Diagnostic::new(
                    SyntaxCode::UnexpectedTokenInFormatString,
                    "format string interpolation is not supported yet",
                )
                .at_span(written),
            )?;
        }
        let span = self.spans.insert(written);
        self.tokens.push(Spanned::new(Token::String, span));
        Ok(())
    }

    fn accept_raw_token(&mut self, token: RawToken) -> Result<(), ParseFailure> {
        self.record_lexical_error(token)?;
        // Unknown characters already have their precise lexical diagnostic.
        // Preserve the source snapshot without adding a duplicate parser error.
        if token.kind == Token::Unknown {
            return Ok(());
        }
        let span = self.spans.insert(token.span);
        match token.kind {
            Token::Comment => self
                .comments
                .push(Spanned::new(CommentKind::Ordinary, span)),
            Token::DocComment => self
                .comments
                .push(Spanned::new(CommentKind::Documentation, span)),
            Token::ModuleDocComment => {
                self.comments
                    .push(Spanned::new(CommentKind::ModuleDocumentation, span));
            }
            Token::MalformedInt => self.tokens.push(Spanned::new(Token::Int, span)),
            Token::MalformedFloat => self.tokens.push(Spanned::new(Token::Float, span)),
            Token::UnterminatedString => self.tokens.push(Spanned::new(Token::String, span)),
            kind => self.tokens.push(Spanned::new(kind, span)),
        }
        Ok(())
    }

    fn record_lexical_error(&mut self, token: RawToken) -> Result<(), ParseFailure> {
        let (code, message) = match token.kind {
            Token::MalformedInt => (
                LexCode::InvalidInteger,
                "integer literal has invalid underscore placement",
            ),
            Token::MalformedFloat => (
                LexCode::InvalidFloat,
                "float literal has invalid underscore placement",
            ),
            Token::UnterminatedString => (LexCode::UnclosedString, "unclosed string literal"),
            Token::Unknown => (LexCode::UnexpectedCharacter, "unexpected character"),
            _ => return Ok(()),
        };
        self.record(Diagnostic::new(DiagnosticCode::Lex(code), message).at_span(token.span))
    }

    fn lex_finish(mut self) -> Result<Self, ParseFailure> {
        let raw = std::mem::take(&mut self.tokens);
        let normalized = crate::layout::normalize(self.source, raw, &mut self.spans, self.options)?;
        self.tokens = normalized.tokens;
        self.statement_starts = normalized.statement_starts;
        self.parenthesis_closes = crate::layout::parenthesis_closes(
            &self.tokens,
            &self.statement_starts,
            &self.spans,
            self.options,
        )?;
        Ok(self)
    }

    pub fn starts_statement(&self, index: usize) -> bool {
        self.statement_starts.get(index).copied().unwrap_or(false)
    }

    pub fn current(&self) -> Option<Token> {
        self.tokens.get(self.position).map(|token| token.value)
    }

    pub fn peek(&self, offset: usize) -> Option<Token> {
        self.tokens
            .get(self.position + offset)
            .map(|token| token.value)
    }

    pub fn peek_spelling(&self, offset: usize) -> Option<&'source str> {
        self.tokens
            .get(self.position + offset)
            .map(|token| self.spans.get(token.span_id).extract(self.source))
    }

    pub fn advance(&mut self) -> ParseResult<Spanned<Token>> {
        self.checkpoint()?;
        let token = self.tokens.get(self.position).cloned();
        let token = token.ok_or_else(|| self.error("unexpected end of source"))?;
        self.position += 1;
        Ok(token)
    }

    pub fn error(&self, message: &str) -> ParseError {
        ParseError::Syntax(
            Diagnostic::new(SyntaxCode::UnexpectedToken, message).at_span(self.current_span()),
        )
    }

    pub fn checkpoint(&self) -> ParseResult<()> {
        self.options
            .checkpoint(self.current_span())
            .map_err(ParseError::Abort)?;
        if self.spans.overflowed() {
            return Err(ParseError::Abort(self.span_limit_failure()));
        }
        Ok(())
    }

    pub fn span_limit_failure(&self) -> ParseFailure {
        ParseFailure::LimitExceeded {
            limit: ParseLimit::Spans,
            maximum: SpanId::SPANS_PER_FILE,
            span: self.current_span(),
        }
    }

    pub fn record(&mut self, diagnostic: Diagnostic) -> Result<(), ParseFailure> {
        if self.diagnostics.len() == self.options.limits.diagnostics_max as usize {
            return Err(ParseFailure::LimitExceeded {
                limit: ParseLimit::Diagnostics,
                maximum: self.options.limits.diagnostics_max,
                span: diagnostic.span,
            });
        }
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    pub fn report(&mut self, message: &str) -> ParseResult<()> {
        let diagnostic =
            Diagnostic::new(SyntaxCode::UnexpectedToken, message).at_span(self.current_span());
        self.record(diagnostic).map_err(ParseError::Abort)
    }

    pub fn missing_span(&mut self) -> SpanId {
        let start = self.current_span().start();
        self.spans.insert(Span::new(start, start))
    }

    pub fn current_span(&self) -> Span {
        match self.tokens.get(self.position) {
            Some(token) => self.spans.get(token.span_id),
            None => Span::new(self.source.len(), self.source.len()),
        }
    }

    pub fn node<T>(&mut self, value: T, start: SpanId, end: SpanId) -> Spanned<T> {
        assert!(start.is_valid());
        assert!(end.is_valid());
        Spanned::new(value, self.spans.merge(start, end))
    }

    pub fn skip_newlines(&mut self) {
        for _ in self.position..self.tokens.len() {
            if self.current() != Some(Token::Newline) {
                break;
            }
            self.position += 1;
        }
    }

    pub fn last_span(&self) -> SpanId {
        assert!(self.position > 0);
        self.tokens[self.position - 1].span_id
    }

    pub fn require_line_end(&self) -> ParseResult<()> {
        if matches!(self.current(), None | Some(Token::Newline | Token::Dedent))
            || self.expression_layout_end == Some(self.position)
        {
            return Ok(());
        }
        Err(self.error("unsupported syntax or missing newline in parser"))
    }
}
