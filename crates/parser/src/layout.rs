//! Block layout follows significant source lines, not comment or list indentation.
//!
//! The source lexer emits physical newlines, not indentation tokens.
//! Derive body transitions here so comments and lists cannot open or close a body.
//! Newlines remain visible as expression/list separators.

use ast::{Span, SpanTable, Spanned};
use lexer::TokenKind as Token;

use crate::{ParseFailure, ParseLimit, ParseOptions};

mod recovery;

pub(crate) struct Normalized {
    pub tokens: Vec<Spanned<Token>>,
    pub statement_starts: Vec<bool>,
}

impl Normalized {
    fn finish(
        mut tokens: Vec<Spanned<Token>>,
        mut statement_starts: Vec<bool>,
        depth: usize,
        end: ast::SpanId,
    ) -> Self {
        assert!(depth > 0, "root indentation always exists");
        assert_eq!(tokens.len(), statement_starts.len());
        for _ in 1..depth {
            tokens.push(Spanned::new(Token::Dedent, end));
            statement_starts.push(false);
        }
        assert_eq!(tokens.len(), statement_starts.len());
        Self {
            tokens,
            statement_starts,
        }
    }
}

pub(crate) fn normalize(
    source: &str,
    tokens: Vec<Spanned<Token>>,
    spans: &mut SpanTable,
    options: ParseOptions<'_>,
) -> Result<Normalized, ParseFailure> {
    let recovery = recovery::RecoveryIndex::new(source, &tokens, spans, options)?;
    let mut result = Vec::with_capacity(tokens.len());
    let mut statement_starts = Vec::with_capacity(tokens.len());
    let mut indents = vec![0];
    let mut opens = Vec::new();
    let mut owner_indent = 0;
    let mut line_indent = 0;
    let mut line_start = true;
    // A line after a trailing `or` continues the one before it: the next result variant.
    let mut last = None;
    // A union declaration (`T is A or`) continues by indentation instead.
    let mut declares_union = false;
    // A type's trait list (`T has A and B`) goes on over lines that begin with `and`.
    let mut lists_traits = false;
    // TODO(header-layout): keep a function header open across a same-indent `where` and
    // terminating `:` line. Indented contract items are header continuations, not a body.
    // Emit body indentation only after `:`; malformed headers must not swallow the next declaration.
    // A `use` line, or a line with a `where` clause, goes on after a trailing comma onto lines
    // indented deeper than it: one claim or import member per line.
    let mut list_head = false;
    let mut list_indent = 0;
    for (index, token) in tokens.iter().enumerate() {
        options.checkpoint(spans.get(token.span_id))?;
        let mut boundary = false;
        let mut comma_continues = false;
        if line_start && token.value != Token::Newline {
            line_indent = recovery::indentation(source, spans.get(token.span_id).start());
            comma_continues = last == Some(Token::Comma)
                && list_head
                && opens.is_empty()
                && line_indent > list_indent;
            if line_indent == 0 && !(last == Some(Token::Or) && declares_union) {
                declares_union = false;
            }
            if line_indent == 0 {
                lists_traits = false;
            }
            // Inside an open delimiter, a statement keyword indented deeper than the line that
            // opened it belongs to the expression (a conditional argument), not a new statement.
            boundary = !comma_continues
                && recovery.starts_unambiguous_statement(index)
                && (opens.is_empty() || line_indent <= owner_indent);
            let unclosed = opens.last().is_none_or(|open| recovery.is_unclosed(*open));
            if !comma_continues && (opens.is_empty() || unclosed && line_indent <= owner_indent) {
                boundary |= recovery.starts_declaration(index);
            }
            if boundary {
                opens.clear();
            }
        }
        match token.value {
            Token::Indent | Token::Dedent => continue,
            Token::Newline => line_start = true,
            // A stray close is a syntax error, not evidence that its enclosing
            // body ended. The next real statement supplies the indentation.
            Token::ParenClose | Token::BracketClose => line_start = false,
            _ => {
                // A `then` or `else` indented deeper than its block continues the conditional above.
                // A chain arm's `then` may also sit level with its condition, one line below.
                let continues = (matches!(token.value, Token::Then | Token::Else)
                    || (token.value == Token::And && lists_traits))
                    && indents.last().is_some_and(|block| line_indent > *block)
                    || token.value == Token::Then
                        && indents.last().is_some_and(|block| line_indent == *block)
                        && *indents.last().expect("indent") > 0
                    || comma_continues;
                if line_start && opens.is_empty() && !continues {
                    list_head = token.value == Token::Use;
                    list_indent = line_indent;
                }
                if continues && line_start {
                    // The conditional goes on: drop the line break before it.
                    while result.last().is_some_and(|last: &Spanned<Token>| last.value == Token::Newline) {
                        result.pop();
                    }
                    statement_starts.truncate(result.len());
                }
                if line_start
                    && opens.is_empty()
                    && !continues
                    && !(last == Some(Token::Or) && !declares_union)
                {
                    update_indent(
                        line_indent,
                        token,
                        spans,
                        &mut indents,
                        &mut result,
                        options,
                    )?;
                }
                line_start = false;
            }
        }
        match token.value {
            Token::ParenOpen | Token::BracketOpen => {
                if opens.is_empty() {
                    owner_indent = line_indent;
                }
                check_parentheses(opens.len(), spans.get(token.span_id), options)?;
                opens.push(index);
            }
            Token::ParenClose | Token::BracketClose
                if opens
                    .last()
                    .is_some_and(|open| closes_delimiter(tokens[*open].value, token.value)) =>
            {
                opens.pop();
            }
            _ => {}
        }
        if !matches!(token.value, Token::Newline | Token::Indent | Token::Dedent) {
            last = Some(token.value);
            if token.value == Token::Is && opens.is_empty() && line_indent == 0 {
                declares_union = true;
            }
            if token.value == Token::Where && opens.is_empty() {
                list_head = true;
            }
            if token.value == Token::Has && opens.is_empty() && line_indent == 0 {
                lists_traits = true;
            }
        }
        statement_starts.resize(result.len(), false);
        result.push(token.clone());
        statement_starts.push(boundary);
    }
    let end = spans.insert(Span::new(source.len(), source.len()));
    let depth = indents.len();
    Ok(Normalized::finish(result, statement_starts, depth, end))
}

/// Index unmatched opens as well as matched pairs once, so malformed declaration
/// lookahead cannot repeatedly scan the remaining source.
pub(crate) fn parenthesis_closes(
    tokens: &[Spanned<Token>],
    statement_starts: &[bool],
    spans: &SpanTable,
    options: ParseOptions<'_>,
) -> Result<Vec<Option<usize>>, ParseFailure> {
    let mut closes = vec![None; tokens.len()];
    let mut opens = Vec::new();
    assert_eq!(tokens.len(), statement_starts.len());
    for (index, token) in tokens.iter().enumerate() {
        options.checkpoint(spans.get(token.span_id))?;
        if statement_starts[index] {
            opens.clear();
        }
        match token.value {
            Token::Indent | Token::Dedent => opens.clear(),
            Token::ParenOpen | Token::BracketOpen => {
                check_parentheses(opens.len(), spans.get(token.span_id), options)?;
                opens.push(index);
            }
            Token::ParenClose | Token::BracketClose => {
                if opens
                    .last()
                    .is_some_and(|open| closes_delimiter(tokens[*open].value, token.value))
                    && let Some(open) = opens.pop()
                {
                    closes[open] = Some(index);
                }
            }
            _ => {}
        }
    }
    Ok(closes)
}

pub(super) fn closes_delimiter(open: Token, close: Token) -> bool {
    matches!(
        (open, close),
        (Token::ParenOpen, Token::ParenClose) | (Token::BracketOpen, Token::BracketClose)
    )
}

fn update_indent(
    indent: usize,
    token: &Spanned<Token>,
    spans: &mut SpanTable,
    indents: &mut Vec<usize>,
    result: &mut Vec<Spanned<Token>>,
    options: ParseOptions<'_>,
) -> Result<(), ParseFailure> {
    let start = spans.get(token.span_id).start();
    let current = *indents.last().expect("root indentation");
    if indent > current {
        if indents.len() > usize::from(options.limits.body_depth_max) {
            return Err(ParseFailure::LimitExceeded {
                limit: ParseLimit::BodyDepth,
                maximum: u32::from(options.limits.body_depth_max),
                span: spans.get(token.span_id),
            });
        }
        indents.push(indent);
        result.push(Spanned::new(Token::Indent, token.span_id));
    } else {
        // Preserve Gin's existing policy: close levels deeper than the new column.
        for _ in 1..indents.len() {
            if *indents.last().expect("root remains") <= indent {
                break;
            }
            indents.pop();
            let boundary = spans.insert(Span::new(start, start));
            result.push(Spanned::new(Token::Dedent, boundary));
        }
    }
    assert!(!indents.is_empty());
    assert!(indents.len() <= usize::from(options.limits.body_depth_max) + 1);
    Ok(())
}

fn check_parentheses(
    depth: usize,
    span: ast::Span,
    options: ParseOptions<'_>,
) -> Result<(), ParseFailure> {
    if depth == usize::from(options.limits.parentheses_max) {
        return Err(ParseFailure::LimitExceeded {
            limit: ParseLimit::Parentheses,
            maximum: u32::from(options.limits.parentheses_max),
            span,
        });
    }
    assert!(depth < usize::from(options.limits.parentheses_max));
    Ok(())
}
