use crate::cursor::TokenCursor;
use ast::ModPath;
use internment::Intern;
use lexer::Token;

pub fn parse_id(cursor: &mut TokenCursor) -> Option<Intern<String>> {
    match cursor.peek()? {
        Token::Id(name) => {
            let id = cursor.intern(name);
            cursor.advance();
            Some(id)
        }
        _ => None,
    }
}

pub fn parse_path(cursor: &mut TokenCursor) -> Option<ModPath> {
    let start_span = cursor.peek_span()?;

    let root = match cursor.peek()? {
        Token::Id(name) => {
            let id = cursor.intern(name);
            cursor.advance();
            id
        }
        _ => return None,
    };

    let mut segments = Vec::new();
    let mut _end_span = start_span;

    while cursor.is_at(&Token::Dot) && matches!(cursor.peek_at(1), Some(Token::Id(_))) {
        cursor.advance(); // consume Dot
        if let Some((Token::Id(name), span)) = cursor.advance() {
            segments.push(cursor.intern(name));
            _end_span = span;
        }
    }

    Some(ModPath {
        root,
        segments,
        span: start_span,
    })
}

pub fn parse_tag_path(cursor: &mut TokenCursor) -> Option<ModPath> {
    let start_span = cursor.peek_span()?;

    let root = match cursor.peek()? {
        Token::Tag(name) => {
            let id = cursor.intern(name);
            cursor.advance();
            id
        }
        _ => return None,
    };

    let mut segments = Vec::new();
    let mut _end_span = start_span;

    if !cursor.is_at(&Token::Dot) || !matches!(cursor.peek_at(1), Some(Token::Id(_))) {
        return None;
    }
    cursor.advance(); // consume Dot

    if let Some((Token::Id(name), span)) = cursor.advance() {
        segments.push(cursor.intern(name));
        _end_span = span;
    }

    while cursor.is_at(&Token::Dot) && matches!(cursor.peek_at(1), Some(Token::Id(_))) {
        cursor.advance(); // consume Dot
        if let Some((Token::Id(name), span)) = cursor.advance() {
            segments.push(cursor.intern(name));
            _end_span = span;
        }
    }

    Some(ModPath {
        root,
        segments,
        span: start_span,
    })
}

pub fn parse_tag_variant_path(cursor: &mut TokenCursor) -> Option<ModPath> {
    let start_span = cursor.peek_span()?;

    let root = match cursor.peek()? {
        Token::Tag(name) => {
            let id = cursor.intern(name);
            cursor.advance();
            id
        }
        _ => return None,
    };

    let mut segments = Vec::new();
    let mut _end_span = start_span;

    if !cursor.is_at(&Token::Dot) || !matches!(cursor.peek_at(1), Some(Token::Tag(_))) {
        return None;
    }
    cursor.advance(); // consume Dot

    if let Some((Token::Tag(name), span)) = cursor.advance() {
        segments.push(cursor.intern(name));
        _end_span = span;
    }

    while cursor.is_at(&Token::Dot) && matches!(cursor.peek_at(1), Some(Token::Tag(_))) {
        cursor.advance(); // consume Dot
        if let Some((Token::Tag(name), span)) = cursor.advance() {
            segments.push(cursor.intern(name));
            _end_span = span;
        }
    }

    Some(ModPath {
        root,
        segments,
        span: start_span,
    })
}
