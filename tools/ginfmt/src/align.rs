use lexer::{Lexer, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Is,
    Colon,
}

#[derive(Debug)]
struct AlignableLine {
    /// Byte offset in source where the prefix ends (everything before the delimiter gap).
    prefix_end: usize,
    /// Byte offset where the delimiter token starts.
    delimiter_start: usize,
    /// Byte offset where the delimiter token ends.
    _delimiter_end: usize,
    delimiter_kind: DelimiterKind,
    /// Indentation depth (number of Indent tokens seen before first real token).
    indent_level: usize,
    /// The source line number (0-indexed) this token sequence belongs to.
    source_line: usize,
}

/// Format a Gin source string by aligning consecutive lines that share a delimiter.
pub fn format(source: &str) -> String {
    let alignable = find_alignable_lines(source);
    if alignable.is_empty() {
        return source.to_string();
    }
    apply_alignment(source, &alignable)
}

/// Scan the token stream and find lines that have an alignable pattern.
fn find_alignable_lines(source: &str) -> Vec<AlignableLine> {
    let lexer = Lexer::new(source);
    let tokens: Vec<(Token<'_>, chumsky::span::SimpleSpan)> = lexer.collect();

    let mut result = Vec::new();
    let mut i = 0;
    let mut indent_level: usize = 0;
    let mut line_start = true;
    let mut current_line: usize = 0;

    // State for current line scan
    let mut first_token_end: Option<usize> = None;
    let mut saw_paren = false;
    let mut paren_depth: usize = 0;

    while i < tokens.len() {
        let (tok, span) = &tokens[i];
        let end = span.end;

        match tok {
            Token::Newline => {
                line_start = true;
                first_token_end = None;
                saw_paren = false;
                paren_depth = 0;
                current_line = source[..end].matches('\n').count();
                i += 1;
            }
            Token::Indent => {
                indent_level += 1;
                i += 1;
            }
            Token::Dedent => {
                indent_level = indent_level.saturating_sub(1);
                i += 1;
            }
            Token::Is if first_token_end.is_some() && paren_depth == 0 => {
                // Span includes leading whitespace. Actual "is" starts at end - 2.
                let actual_delim_start = end - 2;
                result.push(AlignableLine {
                    prefix_end: first_token_end.unwrap(),
                    delimiter_start: actual_delim_start,
                    _delimiter_end: end,
                    delimiter_kind: DelimiterKind::Is,
                    indent_level,
                    source_line: current_line,
                });
                i += 1;
                skip_to_newline(&tokens, &mut i);
            }
            Token::Colon if first_token_end.is_some() && paren_depth == 0 => {
                // For colon binds, the prefix is "name:" (colon attached).
                // We align by padding after the colon, before the value.
                let colon_end = end;
                // Find where the next real content starts in source
                let value_start = source[colon_end..]
                    .find(|c: char| !c.is_whitespace() && c != '\n')
                    .map(|offset| colon_end + offset)
                    .unwrap_or(colon_end);

                // Only align if there's content after colon on the same line
                if value_start < source.len() && !source[colon_end..value_start].contains('\n') {
                    result.push(AlignableLine {
                        prefix_end: colon_end,
                        delimiter_start: value_start,
                        _delimiter_end: value_start,
                        delimiter_kind: DelimiterKind::Colon,
                        indent_level,
                        source_line: current_line,
                    });
                }
                i += 1;
                skip_to_newline(&tokens, &mut i);
            }
            Token::ParenOpen => {
                if line_start && first_token_end.is_none() {
                    // Line starts with paren — not alignable
                    skip_to_newline(&tokens, &mut i);
                    continue;
                }
                paren_depth += 1;
                saw_paren = true;
                i += 1;
            }
            Token::ParenClose => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 && saw_paren {
                    // Update first_token_end to include the closing paren
                    first_token_end = Some(end);
                }
                i += 1;
            }
            Token::BracketOpen => {
                paren_depth += 1;
                i += 1;
            }
            Token::BracketClose => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    first_token_end = Some(end);
                }
                i += 1;
            }
            _ => {
                line_start = false;
                if first_token_end.is_none() {
                    // This is the first real token on the line — but we need to
                    // keep scanning for a compound prefix (e.g. `self.x.y` or `Type.method`)
                    // We'll update first_token_end as we consume dotted paths
                    first_token_end = Some(end);
                    i += 1;
                    // Consume dotted path: Id.Id.Id or Tag.Id etc
                    while i < tokens.len() {
                        if let (Token::Dot, _) = &tokens[i] {
                            i += 1; // consume dot
                            if i < tokens.len() {
                                match &tokens[i].0 {
                                    Token::Id(_) | Token::Tag(_) | Token::SelfInstance => {
                                        first_token_end = Some(tokens[i].1.end);
                                        i += 1;
                                    }
                                    _ => break,
                                }
                            }
                        } else {
                            break;
                        }
                    }
                } else if paren_depth > 0 {
                    // Inside parens, just advance
                    i += 1;
                } else {
                    // Second token on line that's not a delimiter — not alignable
                    skip_to_newline(&tokens, &mut i);
                    first_token_end = None;
                }
            }
        }
    }

    result
}

fn skip_to_newline(tokens: &[(Token<'_>, chumsky::span::SimpleSpan)], i: &mut usize) {
    while *i < tokens.len() {
        if let Token::Newline = tokens[*i].0 {
            return; // Don't consume the newline itself
        }
        *i += 1;
    }
}

/// Group consecutive alignable lines and apply padding.
fn apply_alignment(source: &str, lines: &[AlignableLine]) -> String {
    // Group consecutive lines with same indent_level and delimiter_kind
    // that are on consecutive source lines
    let groups = group_lines(lines);

    // Build a list of edits: (byte_range_to_replace, replacement_string)
    let mut edits: Vec<(usize, usize, String)> = Vec::new();

    for group in &groups {
        if group.len() < 2 {
            continue;
        }

        // Find the maximum prefix length (in characters, for display alignment)
        let max_prefix_len = group
            .iter()
            .map(|line| {
                // Get just the prefix on this line (from line start to prefix_end)
                let line_start_byte = source[..line.prefix_end]
                    .rfind('\n')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                source[line_start_byte..line.prefix_end].len()
            })
            .max()
            .unwrap_or(0);

        for line in group {
            let line_start_byte = source[..line.prefix_end]
                .rfind('\n')
                .map(|p| p + 1)
                .unwrap_or(0);
            let current_prefix_len = source[line_start_byte..line.prefix_end].len();
            let padding_needed = max_prefix_len - current_prefix_len;

            // The gap to replace is from prefix_end to delimiter_start
            let existing_gap = &source[line.prefix_end..line.delimiter_start];

            let new_gap = match line.delimiter_kind {
                DelimiterKind::Is => {
                    // For `is`: prefix<spaces>is → pad with spaces before "is"
                    " ".repeat(padding_needed + 1) // +1 for the minimum single space
                }
                DelimiterKind::Colon => {
                    // For `:`: name:<spaces>value → pad after colon with min 1 space
                    " ".repeat(padding_needed + 1) // +1 for minimum single space
                }
            };

            if new_gap != existing_gap {
                edits.push((line.prefix_end, line.delimiter_start, new_gap));
            }
        }
    }

    if edits.is_empty() {
        return source.to_string();
    }

    // Apply edits in reverse order to preserve byte offsets
    edits.sort_by_key(|b| std::cmp::Reverse(b.0));

    let mut result = source.to_string();
    for (start, end, replacement) in edits {
        result.replace_range(start..end, &replacement);
    }

    result
}

fn group_lines(lines: &[AlignableLine]) -> Vec<Vec<&AlignableLine>> {
    let mut groups: Vec<Vec<&AlignableLine>> = Vec::new();
    let mut current_group: Vec<&AlignableLine> = Vec::new();

    for line in lines {
        if let Some(prev) = current_group.last() {
            let consecutive = line.source_line == prev.source_line + 1;
            let same_kind = line.delimiter_kind == prev.delimiter_kind;
            let same_indent = line.indent_level == prev.indent_level;

            if consecutive && same_kind && same_indent {
                current_group.push(line);
            } else {
                if !current_group.is_empty() {
                    groups.push(current_group);
                }
                current_group = vec![line];
            }
        } else {
            current_group.push(line);
        }
    }

    if !current_group.is_empty() {
        groups.push(current_group);
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_alignment() {
        let input = "Area is 0...999\nGroup is 0...99\nSerial is 0...9999\n";
        let output = format(input);
        assert_eq!(
            output,
            "Area   is 0...999\nGroup  is 0...99\nSerial is 0...9999\n"
        );
    }

    #[test]
    fn test_colon_alignment() {
        let input = "main:\n    start: Instant.now\n    file: File.open\n    reader: BufReader.new\nreturn\n";
        let output = format(input);
        assert!(output.contains("start:  Instant.now"));
        assert!(output.contains("file:   File.open"));
        assert!(output.contains("reader: BufReader.new"));
    }

    #[test]
    fn test_single_line_no_change() {
        let input = "Area is 0...999\n";
        let output = format(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_blank_line_breaks_group() {
        let input = "Area is 0...999\n\nGroup is 0...99\n";
        let output = format(input);
        // Blank line separates them — no alignment
        assert_eq!(output, input);
    }

    #[test]
    fn test_idempotent() {
        let input = "Area   is 0...999\nGroup  is 0...99\nSerial is 0...9999\n";
        let output = format(input);
        assert_eq!(output, input);
        // Second pass should be identical
        let output2 = format(&output);
        assert_eq!(output2, output);
    }

    #[test]
    fn test_mixed_delimiters_separate_groups() {
        let input = "Area is 0...999\nstart: Instant.now\n";
        let output = format(input);
        // Different delimiters, not grouped
        assert_eq!(output, input);
    }

    #[test]
    fn test_different_indent_levels() {
        let input = "main:\n    a: 1\n    bb: 2\nreturn\n";
        let output = format(input);
        assert!(output.contains("a:  1"));
        assert!(output.contains("bb: 2"));
    }
}
