//! Skip doc comments and purely-comment indented regions inside expression bodies.
//!
//! `--` lines are lexed as [`Token::DocComment`]. Without treating them like trivia,
//! `parse_body_exprs` stops before the first doc-comment line and bind/if bodies
//! spuriously report missing `return` even when a valid `return` appears later.

use lexer::Token;

use crate::cursor::TokenCursor;

fn is_skippable_comment(token: Option<&Token<'_>>) -> bool {
    matches!(
        token,
        Some(Token::DocComment(_)) | Some(Token::Comment(_)) | Some(Token::ModuleDocComment(_))
    )
}

/// If the next token is [`Token::Indent`], consume it and skip tokens until the matching
/// [`Token::Dedent`], but only when the region contains comments (and nested comment-only
/// blocks) exclusively. Otherwise rewind and return `false`.
pub(crate) fn try_skip_comment_only_indented_region(cursor: &mut TokenCursor) -> bool {
    let start = cursor.pos();
    if !matches!(cursor.peek(), Some(Token::Indent)) {
        return true;
    }
    cursor.advance();
    loop {
        if let Some(t) = cursor.peek()
            && is_skippable_comment(Some(t))
        {
            cursor.advance();
            continue;
        }
        match cursor.peek() {
            Some(Token::Indent) => {
                if !try_skip_comment_only_indented_region(cursor) {
                    cursor.rewind(start);
                    return false;
                }
            }
            Some(Token::Dedent) => {
                cursor.advance();
                return true;
            }
            None => {
                cursor.rewind(start);
                return false;
            }
            _ => {
                cursor.rewind(start);
                return false;
            }
        }
    }
}

/// Skip leading doc comments, line comments, and comment-only indented blocks (e.g. deeper
/// `--` lines) before the next real body statement.
pub(crate) fn skip_expr_body_trivia(cursor: &mut TokenCursor) {
    loop {
        if let Some(t) = cursor.peek()
            && is_skippable_comment(Some(t))
        {
            cursor.advance();
            continue;
        }
        match cursor.peek() {
            Some(Token::Indent) => {
                if !try_skip_comment_only_indented_region(cursor) {
                    break;
                }
            }
            _ => break,
        }
    }
}
