//! The guarantees, as checks over source text. `format` runs all of them on its own output
//! before returning it; the tests run them over the corpus and against deliberately broken
//! printers.

use ast::{SourceFile, SpanId};

use crate::scan::{Tok, scan};
use lexer::TokenKind;

/// Parse and refuse any flaw.
pub fn parse_clean(source: &str) -> Result<SourceFile, String> {
    match parser::parse_strict(source) {
        Ok(file) => Ok(file),
        Err(diagnostics) => Err(diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.code.slug(), d.message))
            .collect::<Vec<_>>()
            .join("; ")),
    }
}

fn spelling(source: &str, token: &Tok) -> String {
    let text = token.text(source);
    if token.is_comment() { text.trim_end().to_owned() } else { text.to_owned() }
}

/// Every token but newlines, comments included, as written.
pub fn token_stream(source: &str) -> Vec<String> {
    scan(source)
        .iter()
        .filter(|token| token.kind != TokenKind::Newline)
        .map(|token| spelling(source, token))
        .collect()
}

/// Commas are not compared anywhere below: a comma and a line break separate list items alike
/// and the wrapper trades one for the other. The AST still shows every item.
///
/// The AST with each span replaced by the tokens it covers: equal for two sources exactly when
/// they parse to the same tree over the same spelling, wherever it sits and however it is spaced.
pub fn fingerprint(source: &str, file: &SourceFile) -> String {
    let debug = format!("{:?}", file.items);
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug.as_str();
    while let Some(at) = rest.find("SpanId(") {
        out.push_str(&rest[..at]);
        let digits = &rest[at + "SpanId(".len()..];
        let length = digits.find(')').expect("SpanId debug closes");
        let id: u32 = digits[..length].parse().expect("SpanId debug holds a number");
        let span = file.span_table.get(SpanId::from(id));
        let covered = &source[span.start()..span.end()];
        out.push('«');
        for token in scan(covered).iter().filter(|t| t.kind != TokenKind::Newline && t.kind != TokenKind::Comma && !t.is_comment()) {
            out.push_str(token.text(covered));
            out.push(' ');
        }
        out.push('»');
        rest = &digits[length + 1..];
    }
    out.push_str(rest);
    out
}

/// Comments in source order, each with the number of non-comment tokens before it and whether it
/// shares its line with code. A comment that moved past code, or onto or off a code line, differs.
pub fn comments(source: &str) -> Vec<(String, usize, bool)> {
    let tokens = scan(source);
    let mut found = Vec::new();
    let mut code_before = 0;
    let mut line_has_code = false;
    for token in &tokens {
        match token.kind {
            TokenKind::Newline => line_has_code = false,
            TokenKind::Comma => {}
            _ if token.is_comment() => found.push((spelling(source, token), code_before, line_has_code)),
            _ => {
                code_before += 1;
                line_has_code = true;
            }
        }
    }
    found
}

/// (b) `after` parses without flaws to the same AST as `before`, ignoring spans and trivia.
pub fn check_semantics(before: &str, after: &str) -> Result<(), String> {
    let first = parse_clean(before).map_err(|e| format!("input does not parse: {e}"))?;
    let second = parse_clean(after).map_err(|e| format!("output does not parse: {e}"))?;
    if token_stream_without_comments(before) != token_stream_without_comments(after) {
        return Err("output spells different tokens".to_owned());
    }
    if fingerprint(before, &first) == fingerprint(after, &second) {
        Ok(())
    } else {
        Err("output parses to a different AST".to_owned())
    }
}

fn token_stream_without_comments(source: &str) -> Vec<String> {
    scan(source)
        .iter()
        .filter(|t| t.kind != TokenKind::Newline && t.kind != TokenKind::Comma && !t.is_comment())
        .map(|t| spelling(source, t))
        .collect()
}

/// (c) Same comments, same order, same place among the code.
pub fn check_comments(before: &str, after: &str) -> Result<(), String> {
    let (first, second) = (comments(before), comments(after));
    if first == second {
        return Ok(());
    }
    if first.len() != second.len() {
        return Err(format!("{} comments became {}", first.len(), second.len()));
    }
    let at = first.iter().zip(&second).position(|(a, b)| a != b).expect("lists differ");
    Err(format!("comment {at} changed: {:?} became {:?}", first[at], second[at]))
}

/// (a) Formatting the formatted text changes nothing.
pub fn check_idempotent(format: &dyn Fn(&str) -> String, source: &str) -> Result<(), String> {
    let once = format(source);
    let twice = format(&once);
    if once == twice {
        Ok(())
    } else {
        let line = once.lines().zip(twice.lines()).position(|(a, b)| a != b).unwrap_or_else(|| once.lines().count());
        Err(format!("second pass differs from the first at line {}", line + 1))
    }
}
