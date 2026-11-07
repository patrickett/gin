//! Breaking lines that are longer than the width, where the grammar allows it.
//!
//! The parser reads a line break only in a few places, so only these breaks are made:
//!
//! * inside the brackets of a call, parameter list, array or index: after the opening bracket,
//!   one item per line one level in (newlines separate items; commas are dropped, except in
//!   `[...]`, whose separators are recorded by the syntax tree), the closing
//!   bracket on its own line at the level of the line that opened it;
//! * inside such an item, after a binary operator (`and`/`or` first, else arithmetic), the
//!   rest one level further in. A line may begin with an operator only outside call lists, so
//!   the operator stays at the end;
//! * after the commas of a `use` list and of a `where` clause (also between the parentheses of a
//!   result variant's claims), one item per line, one level in;
//! * before `then` and `else` of a conditional, one level in;
//! * in a type declaration at column 0, before `and` of a `has` list and before `or` of an
//!   `is` union, one level in.
//!
//! Everything else stays on its line, however long. Width counts code, not a trailing comment.

use lexer::TokenKind as K;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub kind: K,
    pub text: String,
    /// Spaces before the cell; ignored for the first cell of a line.
    pub gap: usize,
    /// A unary operator (`-`, `@`, `#`): never a place to break.
    pub prefix: bool,
}

struct Piece {
    indent: usize,
    cells: Vec<Cell>,
    /// An item of a bracketed list, where an operator may end a line.
    in_list: bool,
    /// Inside a declaration's list, where `and` is a refinement and a line may not end in it.
    declaration: bool,
}

const INDENT: usize = crate::print::INDENT;

fn is_comment(cell: &Cell) -> bool {
    matches!(cell.kind, K::Comment | K::DocComment | K::ModuleDocComment)
}

fn code_width(piece: &Piece) -> usize {
    let mut width = piece.indent;
    for (at, cell) in piece.cells.iter().enumerate() {
        if is_comment(cell) {
            break;
        }
        width += if at == 0 { 0 } else { cell.gap } + cell.text.chars().count();
    }
    width
}

fn depths(cells: &[Cell]) -> Vec<usize> {
    let mut depth = 0usize;
    cells
        .iter()
        .map(|cell| {
            if matches!(cell.kind, K::ParenClose | K::BracketClose) {
                depth = depth.saturating_sub(1);
            }
            let here = depth;
            if matches!(cell.kind, K::ParenOpen | K::BracketOpen) {
                depth += 1;
            }
            here
        })
        .collect()
}

fn operand_end(kind: K) -> bool {
    matches!(
        kind,
        K::Id | K::Tag | K::ParenClose | K::BracketClose | K::SelfInstance | K::SelfTag | K::Dot
    )
}

/// Matched bracket pairs as (open, close) cell indexes, ordered by open.
fn pairs(cells: &[Cell]) -> Vec<(usize, usize)> {
    let mut stack = Vec::new();
    let mut found = Vec::new();
    for (at, cell) in cells.iter().enumerate() {
        match cell.kind {
            K::ParenOpen | K::BracketOpen => stack.push(at),
            K::ParenClose | K::BracketClose => {
                if let Some(open) = stack.pop() {
                    found.push((open, at));
                }
            }
            _ => {}
        }
    }
    found.sort_unstable();
    found
}

fn first(mut cells: Vec<Cell>) -> Vec<Cell> {
    if let Some(head) = cells.first_mut() {
        head.gap = 0;
    }
    cells
}

/// Break before the depth-zero cells of `kinds` that are not first on the line.
fn split_before(piece: &Piece, kinds: &[K]) -> Option<Vec<Piece>> {
    let depth = depths(&piece.cells);
    let cuts: Vec<usize> = (1..piece.cells.len())
        .filter(|&at| depth[at] == 0 && kinds.contains(&piece.cells[at].kind) && !is_comment(&piece.cells[at]))
        .collect();
    if cuts.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (n, &cut) in cuts.iter().chain(std::iter::once(&piece.cells.len())).enumerate() {
        out.push(Piece {
            indent: piece.indent + if n == 0 { 0 } else { INDENT },
            cells: first(piece.cells[start..cut].to_vec()),
            in_list: piece.in_list,
            declaration: piece.declaration,
        });
        start = cut;
    }
    Some(out)
}

fn conditional(piece: &Piece) -> Option<Vec<Piece>> {
    let depth = depths(&piece.cells);
    let has_when = piece.cells.iter().zip(&depth).any(|(c, &d)| d == 0 && c.kind == K::When);
    // `then` ending the line already breaks there; only a `then` with more after it is cut.
    let last = piece.cells.iter().rposition(|c| !is_comment(c))?;
    let cuts: Vec<usize> = (1..=last)
        .filter(|&at| {
            depth[at] == 0
                && match piece.cells[at].kind {
                    K::Then => has_when && at < last,
                    K::Else => true,
                    _ => false,
                }
        })
        .collect();
    if cuts.is_empty() || !has_when {
        return None;
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (n, &cut) in cuts.iter().chain(std::iter::once(&piece.cells.len())).enumerate() {
        out.push(Piece {
            indent: piece.indent + if n == 0 { 0 } else { INDENT },
            cells: first(piece.cells[start..cut].to_vec()),
            in_list: piece.in_list,
            declaration: piece.declaration,
        });
        start = cut;
    }
    Some(out)
}

fn declaration(piece: &Piece) -> Option<Vec<Piece>> {
    if piece.indent != 0 || piece.in_list {
        return None;
    }
    let depth = depths(&piece.cells);
    let at_zero = |kind: K| piece.cells.iter().zip(&depth).any(|(c, &d)| d == 0 && c.kind == kind);
    if at_zero(K::Has) {
        split_before(piece, &[K::And])
    } else if at_zero(K::Is) {
        split_before(piece, &[K::Or])
    } else {
        None
    }
}

/// Open the first bracket pair, outermost first, that reaches past the width.
fn open_list(piece: &Piece, width: usize) -> Option<Vec<Piece>> {
    let mut column = piece.indent;
    let ends: Vec<usize> = piece
        .cells
        .iter()
        .enumerate()
        .map(|(at, cell)| {
            column += if at == 0 { 0 } else { cell.gap } + cell.text.chars().count();
            column
        })
        .collect();
    let all = pairs(&piece.cells);
    let depth = depths(&piece.cells);
    let list_like = |open: usize| {
        piece.cells[open].kind == K::BracketOpen || (open > 0 && operand_end(piece.cells[open - 1].kind))
    };
    // `#name(...)` takes its argument on the same line.
    let directive = |open: usize| open >= 2 && piece.cells[open - 2].kind == K::Pound;
    let items = |open: usize, close: usize| {
        1 + (open + 1..close).filter(|&at| piece.cells[at].kind == K::Comma && depth[at] == depth[open] + 1).count()
    };
    let usable: Vec<(usize, usize)> =
        all.iter().copied().filter(|&(o, c)| list_like(o) && !directive(o) && c > o + 1).collect();
    // The leftmost list of several items, as Wadler's outermost group; else one that overflows.
    let chosen = usable
        .iter()
        .copied()
        .find(|&(o, c)| items(o, c) > 1)
        .or_else(|| usable.iter().copied().find(|&(_, c)| ends[c] > width));
    if let Some((open, close)) = chosen {
        let base = depth[open];
        let declaration = piece.declaration || open == 1;
        let mut out = vec![Piece {
            indent: piece.indent,
            cells: piece.cells[..=open].to_vec(),
            in_list: piece.in_list,
            declaration: piece.declaration,
        }];
        let mut start = open + 1;
        for (at, &level) in depth.iter().enumerate().take(close + 1).skip(open + 1) {
            let at_comma = at < close && piece.cells[at].kind == K::Comma && level == base + 1;
            if at == close || at_comma {
                if at > start {
                    // An array keeps its commas: its separators are part of its syntax tree.
                    let keep = at_comma && piece.cells[open].kind == K::BracketOpen;
                    out.push(Piece {
                        indent: piece.indent + INDENT,
                        cells: first(piece.cells[start..at + usize::from(keep)].to_vec()),
                        in_list: true,
                        declaration,
                    });
                }
                start = at + 1;
            }
        }
        out.push(Piece {
            indent: piece.indent,
            cells: first(piece.cells[close..].to_vec()),
            in_list: piece.in_list,
            declaration: piece.declaration,
        });
        return Some(out);
    }
    None
}

/// End the line after the last binary operator that still fits.
fn break_after_operator(piece: &Piece, width: usize) -> Option<Vec<Piece>> {
    if !piece.in_list || piece.declaration {
        return None;
    }
    let depth = depths(&piece.cells);
    let logical = |k: K| matches!(k, K::And | K::Or);
    let arithmetic = |k: K| {
        matches!(k, K::Plus | K::Minus | K::Star | K::Slash | K::Percent | K::Ampersand | K::Pipe | K::Caret | K::ShiftLeft | K::ShiftRight)
    };
    let candidates = |test: &dyn Fn(K) -> bool| -> Vec<usize> {
        (1..piece.cells.len().saturating_sub(1))
            .filter(|&at| depth[at] == 0 && test(piece.cells[at].kind) && !piece.cells[at].prefix && !is_comment(&piece.cells[at + 1]))
            .collect()
    };
    let mut cuts = candidates(&logical);
    if cuts.is_empty() {
        cuts = candidates(&arithmetic);
    }
    if cuts.is_empty() {
        return None;
    }
    // The last cut whose line fits; failing that, the first.
    let mut column = piece.indent;
    let mut fitting = None;
    for (at, cell) in piece.cells.iter().enumerate() {
        column += if at == 0 { 0 } else { cell.gap } + cell.text.chars().count();
        if cuts.contains(&at) && column <= width {
            fitting = Some(at);
        }
    }
    let cut = fitting.unwrap_or(cuts[0]) + 1;
    Some(vec![
        Piece { indent: piece.indent, cells: piece.cells[..cut].to_vec(), in_list: true, declaration: false },
        Piece { indent: piece.indent + INDENT, cells: first(piece.cells[cut..].to_vec()), in_list: true, declaration: false },
    ])
}

/// Split after the given comma cells; the pieces after the first are `levels` in. (One level
/// everywhere: the indentation map would flatten a deeper `where` list on the next pass.)
fn split_after(piece: &Piece, commas: &[usize], levels: usize) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut start = 0;
    for (n, &end) in commas.iter().chain(std::iter::once(&(piece.cells.len() - 1))).enumerate() {
        out.push(Piece {
            indent: piece.indent + if n == 0 { 0 } else { INDENT * levels },
            cells: first(piece.cells[start..=end].to_vec()),
            in_list: false,
            declaration: piece.declaration,
        });
        start = end + 1;
    }
    out
}

/// `use a X, Y, Z`: a member per line after the first.
fn use_list(piece: &Piece) -> Option<Vec<Piece>> {
    if piece.cells.first()?.kind != K::Use {
        return None;
    }
    let depth = depths(&piece.cells);
    let commas: Vec<usize> = (0..piece.cells.len().saturating_sub(1))
        .filter(|&at| depth[at] == 0 && piece.cells[at].kind == K::Comma && !is_comment(&piece.cells[at + 1]))
        .collect();
    (!commas.is_empty()).then(|| split_after(piece, &commas, 1))
}

/// `where a, b, c:` (the claims run to the `:` that opens the body): a claim per line after the
/// first. `where (a, b) or`: the claims of a result variant inside the parentheses.
// TODO(header-layout): after parser support lands, keep the complete header on one line when
// it fits the configured width. Otherwise put `where` and the body-opening `:` on separate
// lines at declaration indentation, with further complete clauses one INDENT deeper.
// Wrap the parameters independently if the signature alone is too wide. Cover exact-width
// boundaries, nested methods, comments, recovery and formatter idempotence before enabling output.
fn where_list(piece: &Piece) -> Option<Vec<Piece>> {
    let depth = depths(&piece.cells);
    let at = piece.cells.iter().zip(&depth).position(|(c, &d)| d == 0 && c.kind == K::Where)?;
    let comma_after = |from: usize, to: usize, level: usize| -> Vec<usize> {
        (from..to)
            .filter(|&i| piece.cells[i].kind == K::Comma && depth[i] == level && i + 1 < piece.cells.len() && !is_comment(&piece.cells[i + 1]))
            .collect()
    };
    if piece.cells.get(at + 1).is_some_and(|c| c.kind == K::ParenOpen) {
        let close = pairs(&piece.cells).into_iter().find(|&(o, _)| o == at + 1)?.1;
        let commas = comma_after(at + 2, close, 1);
        return (!commas.is_empty()).then(|| split_after(piece, &commas, 1));
    }
    let end = (at + 1..piece.cells.len())
        .find(|&i| depth[i] == 0 && piece.cells[i].kind == K::Colon)
        .unwrap_or(piece.cells.len());
    let commas = comma_after(at + 1, end, 0);
    (!commas.is_empty()).then(|| split_after(piece, &commas, 1))
}

fn step(piece: &Piece, width: usize) -> Option<Vec<Piece>> {
    if code_width(piece) <= width {
        return None;
    }
    // A `where` that starts within the width is broken before the parameter list is opened.
    let where_first = {
        let mut column = piece.indent;
        let mut fits = false;
        for (at, cell) in piece.cells.iter().enumerate() {
            column += if at == 0 { 0 } else { cell.gap } + cell.text.chars().count();
            if cell.kind == K::Where {
                fits = column <= width;
                break;
            }
        }
        fits
    };
    conditional(piece)
        .or_else(|| declaration(piece))
        .or_else(|| use_list(piece))
        .or_else(|| if where_first { where_list(piece) } else { None })
        .or_else(|| open_list(piece, width))
        .or_else(|| where_list(piece))
        .or_else(|| break_after_operator(piece, width))
}

/// The physical lines (indent and cells) for one logical line.
pub fn wrap(indent: usize, cells: Vec<Cell>, width: Option<usize>) -> Vec<(usize, Vec<Cell>)> {
    let Some(width) = width else {
        return vec![(indent, cells)];
    };
    let mut done = Vec::new();
    let mut pending = vec![Piece { indent, cells, in_list: false, declaration: false }];
    pending.reverse();
    let mut budget = 100_000usize;
    while let Some(piece) = pending.pop() {
        budget = budget.saturating_sub(1);
        match if budget == 0 { None } else { step(&piece, width) } {
            Some(pieces) => pending.extend(pieces.into_iter().rev()),
            None => done.push((piece.indent, piece.cells)),
        }
    }
    done
}

/// Lines of `formatted` that are over `width` as code and that the wrapper could still break.
/// Empty for text the formatter produced.
pub fn breakable_lines(formatted: &str, width: usize) -> Vec<usize> {
    use crate::scan::scan;
    let mut found = Vec::new();
    for (number, line) in formatted.lines().enumerate() {
        if line.chars().count() <= width {
            continue;
        }
        let tokens = scan(line);
        let mut cells: Vec<Cell> = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            cells.push(Cell {
                kind: token.kind,
                text: token.text(line).to_owned(),
                gap: usize::from(at > 0 && tokens[at - 1].end < token.start),
                prefix: false,
            });
        }
        let indent = line.len() - line.trim_start().len();
        if !cells.is_empty() {
            let piece = Piece { indent, cells: first(cells), in_list: false, declaration: false };
            if step(&piece, width).is_some() {
                found.push(number + 1);
            }
        }
    }
    found
}
