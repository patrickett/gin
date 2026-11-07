//! The printer. It re-emits the token stream of the source line by line: layout (indentation,
//! spacing, blank lines, field alignment) is decided here; tokens, comments and line breaks are
//! never invented, dropped or reordered. Spacing is invisible to the parser, which sees only
//! token kinds and the column at which a line starts, so the indentation map below is the one
//! step that can change meaning. It is order preserving, and `verify` checks the result anyway.

use std::collections::HashSet;

use ast::{SourceFile, TypeBody, TypeMember};
use lexer::TokenKind as K;

use crate::scan::{Tok, scan};
use crate::wrap::{self, Cell};
use crate::{Align, Options};

/// Columns per nesting level. `modules/` uses 4 throughout.
pub const INDENT: usize = 4;

struct Line {
    /// Original indentation in columns (a tab counts 4, as in the parser's layout).
    column: usize,
    /// Code and comment tokens of this line; no `Newline`.
    tokens: Vec<Tok>,
}

impl Line {
    fn is_blank(&self) -> bool {
        self.tokens.is_empty()
    }

    fn is_comment_only(&self) -> bool {
        !self.tokens.is_empty() && self.tokens.iter().all(Tok::is_comment)
    }
}

fn split_lines(source: &str, tokens: &[Tok]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current: Vec<Tok> = Vec::new();
    for token in tokens {
        if token.kind == K::Newline {
            lines.push(finish(source, std::mem::take(&mut current)));
        } else {
            current.push(*token);
        }
    }
    if !current.is_empty() {
        lines.push(finish(source, current));
    }
    lines
}

fn finish(source: &str, tokens: Vec<Tok>) -> Line {
    let column = tokens.first().map_or(0, |first| {
        let line_start = source[..first.start].rfind('\n').map_or(0, |at| at + 1);
        source[line_start..first.start]
            .bytes()
            .map(|byte| if byte == b'\t' { 4 } else { 1 })
            .sum()
    });
    Line { column, tokens }
}

/// Maps original line columns to canonical ones: a line deeper than the nearest enclosing line
/// is one level in; a line at an enclosing line's column takes that line's place. Equal columns
/// stay equal and deeper stays deeper, which is all the parser's layout compares.
struct Indents {
    stack: Vec<(usize, usize)>,
}

impl Indents {
    fn new() -> Self {
        Self { stack: vec![(0, 0)] }
    }

    fn place_code(&mut self, column: usize) -> usize {
        while self.stack.len() > 1 && self.stack.last().is_some_and(|top| top.0 > column) {
            self.stack.pop();
        }
        let top = *self.stack.last().expect("root entry");
        if top.0 == column {
            top.1
        } else {
            let new = top.1 + INDENT;
            self.stack.push((column, new));
            new
        }
    }

    /// Comments never open or close a body, so they only look the stack up.
    fn place_comment(&self, column: usize) -> usize {
        let at = self.stack.iter().rev().find(|entry| entry.0 <= column).copied().expect("root entry");
        if at.0 == column { at.1 } else { at.1 + INDENT }
    }
}

fn operand_end(kind: K) -> bool {
    matches!(
        kind,
        K::Id | K::Tag | K::Int | K::Float | K::String | K::FormatStringDelim
            | K::ParenClose | K::BracketClose | K::SelfInstance | K::SelfTag
    )
}

/// Tokens whose spacing is not decided here: it stays as written, one space or none.
fn as_written(kind: K) -> bool {
    matches!(
        kind,
        K::CurlyOpen | K::CurlyClose | K::Infer | K::Question | K::SlashOr | K::Unknown
    )
}

fn is_prefix(tokens: &[Tok], index: usize) -> bool {
    match tokens[index].kind {
        K::At | K::Pound => true,
        K::Minus => index == 0 || !operand_end(tokens[index - 1].kind),
        _ => false,
    }
}

/// Whether a space separates `tokens[index - 1]` from `tokens[index]`.
fn space_before(tokens: &[Tok], index: usize) -> bool {
    debug_assert!(index > 0);
    let (prev, next) = (tokens[index - 1], tokens[index]);
    let adjacent = prev.end == next.start;
    if next.is_comment() {
        return true;
    }
    if is_prefix(tokens, index - 1) && next.kind != K::Minus {
        return false;
    }
    if matches!(
        next.kind,
        K::Comma | K::ColonSemi | K::Colon | K::ParenClose | K::BracketClose | K::Dot
    ) {
        return false;
    }
    if matches!(prev.kind, K::ParenOpen | K::BracketOpen | K::Dot) {
        return false;
    }
    if as_written(prev.kind) || as_written(next.kind) {
        return !adjacent;
    }
    if matches!(next.kind, K::ParenOpen | K::BracketOpen)
        && (operand_end(prev.kind) || prev.kind.keyword().is_some())
    {
        return !adjacent;
    }
    true
}

struct FieldLines {
    /// Start offsets of the name token of each field, one per line of a `has` body.
    names: HashSet<usize>,
}

fn field_lines(file: &SourceFile) -> FieldLines {
    let mut names = HashSet::new();
    let text: &str = &file.text;
    for item in &file.items {
        let ast::Item::Type(declaration) = &item.value else { continue };
        let TypeBody::Has(members) = &declaration.body.value else { continue };
        for member in members {
            let TypeMember::Binding(binding) = &member.value else { continue };
            let name = binding.name.resolve_span(&file.span_table);
            let end = member.resolve_span(&file.span_table).end();
            let single_line = !text[name.start()..end.max(name.start())].contains('\n');
            if single_line
                && binding.parameters.is_none()
                && binding.value.is_none()
                && binding.contract.is_none()
                && binding.annotation.is_some()
            {
                names.insert(name.start());
            }
        }
    }
    FieldLines { names }
}

/// The `=` of each discriminant (`Name = literal` in the `or` list of an `is` declaration), by
/// the start offset of the operator, with the index of its declaration.
fn discriminant_operators(file: &SourceFile) -> Vec<(usize, usize)> {
    use ast::{BinaryOperator, Expr};
    let mut found = Vec::new();
    for (index, item) in file.items.iter().enumerate() {
        let ast::Item::Type(declaration) = &item.value else { continue };
        let TypeBody::Is(root) = &declaration.body.value else { continue };
        let mut pending = vec![root];
        while let Some(expression) = pending.pop() {
            if let Expr::Binary { left, operator, right } = &expression.value {
                match operator.value {
                    BinaryOperator::Or => {
                        pending.push(left);
                        pending.push(right);
                    }
                    BinaryOperator::Equal if matches!(left.value, Expr::Name(_)) => {
                        found.push((operator.resolve_span(&file.span_table).start(), index));
                    }
                    _ => {}
                }
            }
        }
    }
    found
}

/// Pad the `=` of discriminants on adjacent lines of one declaration to one column, as fields are.
fn align_discriminants(lines: &[Line], placed: &[usize], built: &mut [Option<Vec<Cell>>], operators: &[(usize, usize)]) {
    let position = |at: usize| -> Option<(usize, usize)> {
        let index = lines[at].tokens.iter().position(|t| operators.iter().any(|(start, _)| *start == t.start))?;
        let declaration = operators.iter().find(|(start, _)| *start == lines[at].tokens[index].start)?.1;
        Some((index, declaration))
    };
    let column = |cells: &[Cell], indent: usize, index: usize| -> usize {
        indent + cells[..index].iter().enumerate().map(|(n, c)| if n == 0 { 0 } else { c.gap } + c.text.chars().count()).sum::<usize>()
    };
    let mut at = 0;
    while at < lines.len() {
        let Some((_, declaration)) = position(at) else {
            at += 1;
            continue;
        };
        let mut end = at;
        while end + 1 < lines.len() && position(end + 1).is_some_and(|(_, d)| d == declaration) {
            end += 1;
        }
        if end > at {
            let widest = (at..=end)
                .map(|line| {
                    let (index, _) = position(line).expect("in the run");
                    column(built[line].as_ref().expect("code line"), placed[line], index)
                })
                .max()
                .unwrap_or(0);
            for line in at..=end {
                let (index, _) = position(line).expect("in the run");
                let cells = built[line].as_mut().expect("code line");
                let before = column(cells, placed[line], index);
                cells[index].gap = widest - before + 1;
            }
        }
        at = end + 1;
    }
}

/// Print `source`, which must use `\n` line endings and parse to `file`.
pub fn print(source: &str, file: &SourceFile, options: Options) -> String {
    let tokens = scan(source);
    let lines = split_lines(source, &tokens);
    let fields = field_lines(file);
    let mut indents = Indents::new();
    // New indentation of every line, decided in order because it depends on the lines above.
    let placed: Vec<usize> = lines
        .iter()
        .map(|line| {
            if line.is_blank() {
                0
            } else if line.is_comment_only() {
                indents.place_comment(line.column)
            } else {
                indents.place_code(line.column)
            }
        })
        .collect();
    // Alignment width of each field line: the longest name in its run of adjacent fields. An
    // attribute line (`#private`) between fields does not end a run; blank lines, comments and
    // anything else do.
    let mut name_width = vec![0usize; lines.len()];
    if options.align != Align::Off {
        let is_field = |at: usize| {
            let tokens = &lines[at].tokens;
            tokens.len() > 1
                && !tokens[0].is_comment()
                && !tokens[1].is_comment()
                && fields.names.contains(&tokens[0].start)
        };
        let is_attribute = |at: usize| lines[at].tokens.first().is_some_and(|t| t.kind == K::Pound);
        let mut run: Vec<usize> = Vec::new();
        let mut flush = |run: &mut Vec<usize>| {
            let width = run.iter().map(|&at| lines[at].tokens[0].end - lines[at].tokens[0].start).max().unwrap_or(0);
            for &at in run.iter() {
                name_width[at] = width;
            }
            run.clear();
        };
        for at in 0..lines.len() {
            if is_field(at) && run.last().is_none_or(|&last| placed[last] == placed[at]) {
                run.push(at);
            } else if options.align == Align::ThroughAttributes && is_attribute(at) && !run.is_empty() {
                // Transparent: keeps the run open.
            } else {
                flush(&mut run);
                if is_field(at) {
                    run.push(at);
                }
            }
        }
        flush(&mut run);
    }
    // Cells of every line first: discriminant alignment looks across lines.
    let mut built: Vec<Option<Vec<Cell>>> = Vec::with_capacity(lines.len());
    for (at, line) in lines.iter().enumerate() {
        if line.is_blank() {
            built.push(None);
            continue;
        }
        let mut cells: Vec<Cell> = Vec::with_capacity(line.tokens.len());
        for (index, token) in line.tokens.iter().enumerate() {
            let gap = if index == 0 {
                0
            } else if index == 1 && name_width[at] > 0 {
                let first = line.tokens[0];
                name_width[at] - (first.end - first.start) + 1
            } else {
                usize::from(space_before(&line.tokens, index))
            };
            let text = token.text(source);
            cells.push(Cell {
                kind: token.kind,
                text: if token.is_comment() { text.trim_end().to_owned() } else { text.to_owned() },
                gap,
                prefix: is_prefix(&line.tokens, index),
            });
        }
        built.push(Some(cells));
    }
    if options.align != Align::Off {
        align_discriminants(&lines, &placed, &mut built, &discriminant_operators(file));
    }
    let mut out = String::with_capacity(source.len());
    let mut pending_blank = false;
    let mut wrote_any = false;
    for (at, cells) in built.into_iter().enumerate() {
        let Some(cells) = cells else {
            pending_blank = wrote_any;
            continue;
        };
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        wrote_any = true;
        for (indent, cells) in wrap::wrap(placed[at], cells, options.width) {
            for _ in 0..indent {
                out.push(' ');
            }
            for cell in &cells {
                for _ in 0..cell.gap {
                    out.push(' ');
                }
                out.push_str(&cell.text);
            }
            out.push('\n');
        }
    }
    out
}
