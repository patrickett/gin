use ast::{DefMap, ImplBlock};
use lexer::Token;
use std::collections::HashMap;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

pub fn parse_impl_block(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<ImplBlock> {
    // Parse first Tag (type_name)
    let (type_name, type_name_span) = match cursor.peek()? {
        Token::Tag(name) => {
            let interned = cursor.intern(name);
            let span = cursor.peek_span()?;
            cursor.advance();
            (interned, span)
        }
        _ => return None,
    };

    // Expect dot
    if !cursor.eat(&Token::Dot) {
        cursor.error(
            "expected '.' after type name in impl block header",
            cursor.current_span(),
        );
        return None;
    }

    // Parse second Tag (trait_name)
    let trait_name = match cursor.peek()? {
        Token::Tag(name) => {
            let interned = cursor.intern(name);
            cursor.advance();
            interned
        }
        _ => {
            cursor.error(
                "expected trait name after '.' in impl block header",
                cursor.current_span(),
            );
            return None;
        }
    };

    // Expect '('
    if !cursor.eat(&Token::ParenOpen) {
        cursor.error(
            "expected '(' after impl block header",
            cursor.current_span(),
        );
        return None;
    }

    // Optional indent
    cursor.eat(&Token::Indent);

    // Parse zero or more binds until ')'
    let mut methods: DefMap = HashMap::new();
    loop {
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
            methods.insert(bind.name(), bind);
        } else {
            // If we can't parse a bind and we're not at ')', emit error and break
            if !cursor.is_at(&Token::ParenClose) {
                cursor.error(
                    "expected bind or ')' in impl block body",
                    cursor.current_span(),
                );
            }
            break;
        }
    }

    // Skip optional dedent, newlines
    cursor.eat(&Token::Dedent);

    // Expect ')'
    if !cursor.eat(&Token::ParenClose) {
        cursor.error("expected ')' to close impl block", cursor.current_span());
        return None;
    }

    Some(ImplBlock {
        type_name,
        type_name_span,
        trait_name,
        methods,
    })
}
