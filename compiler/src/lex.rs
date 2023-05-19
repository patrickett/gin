#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    SlashBack,
    SlashForward,
    Colon,
    Comma,
    Tab,
    Space,
    Newline,
    Id(String),
    EOF,
}

// TODO: add { start: usize, end: usize } for LSP in the future
pub fn tokenize(source_code: String) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut token = String::new();

    let mut resolve_token = |nt: Token, ct: &mut String| {
        if !ct.is_empty() {
            tokens.push(Token::Id(ct.clone()));
            ct.clear();
        }
        tokens.push(nt)
    };

    for (_, c) in source_code.chars().enumerate() {
        match c {
            ' ' => resolve_token(Token::Space, &mut token),
            '\\' => resolve_token(Token::SlashForward, &mut token),
            '/' => resolve_token(Token::SlashBack, &mut token),
            '\t' => resolve_token(Token::Tab, &mut token),
            ':' => resolve_token(Token::Colon, &mut token),
            ',' => resolve_token(Token::Comma, &mut token),
            '\n' => resolve_token(Token::Newline, &mut token),
            _ => token.push(c),
        }
    }

    if !token.is_empty() {
        tokens.push(Token::Id(token.clone()))
    }

    tokens.push(Token::EOF);

    tokens
}
