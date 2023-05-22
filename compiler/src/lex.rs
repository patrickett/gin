#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    If,
    Else,
    For,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    ParenLeft,
    ParenRight,
    SlashBack,
    SlashForward,
    Colon,
    Comma,
    Tab,
    Space,
    Newline,
    Id(String),
    Number(usize),
    String(String),
    EOF,

    Pound,
    Plus,
    Dash,
    Equals,
    Ampersand,
    Star,
    Percent,
    Keyword(Keyword),
}

fn parse_keyword(keyword: &str) -> Option<Keyword> {
    match keyword {
        "for" => Some(Keyword::For),
        "if" => Some(Keyword::If),
        "else" => Some(Keyword::Else),
        _ => None,
    }
}

fn add_token(token: Token, ct: &mut String, tokens: &mut Vec<Token>) {
    if !ct.is_empty() {
        if ct.chars().all(|c| c.is_digit(10)) {
            let res: Result<usize, _> = ct.parse();
            if let Ok(n) = res {
                tokens.push(Token::Number(n));
            }
        } else if let Some(keyword) = parse_keyword(&ct) {
            tokens.push(Token::Keyword(keyword))
        } else if ct.starts_with('"') && ct.ends_with('"') {
            tokens.push(Token::String(ct.clone()));
        } else {
            tokens.push(Token::Id(ct.clone()))
        }
        ct.clear();
    }

    if token != Token::Space {
        tokens.push(token);
    }
}

// TODO: add { start: usize, end: usize } for LSP in the future
pub fn tokenize(source_code: String) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut token = String::new();

    for (_, c) in source_code.chars().enumerate() {
        match c {
            // TODO: replace handling comments in parsing with a better lex for comments
            '#' => add_token(Token::Pound, &mut token, &mut tokens),
            '&' => add_token(Token::Ampersand, &mut token, &mut tokens),
            '*' => add_token(Token::Star, &mut token, &mut tokens),
            '%' => add_token(Token::Percent, &mut token, &mut tokens),
            '=' => add_token(Token::Equals, &mut token, &mut tokens),
            '+' => add_token(Token::Plus, &mut token, &mut tokens),
            '-' => add_token(Token::Dash, &mut token, &mut tokens),
            ' ' => add_token(Token::Space, &mut token, &mut tokens),
            '\\' => add_token(Token::SlashForward, &mut token, &mut tokens),
            '/' => add_token(Token::SlashBack, &mut token, &mut tokens),
            '\t' => add_token(Token::Tab, &mut token, &mut tokens),
            ':' => add_token(Token::Colon, &mut token, &mut tokens),
            ',' => add_token(Token::Comma, &mut token, &mut tokens),
            '\n' => add_token(Token::Newline, &mut token, &mut tokens),
            '(' => add_token(Token::ParenLeft, &mut token, &mut tokens),
            ')' => add_token(Token::ParenRight, &mut token, &mut tokens),
            _ => token.push(c),
        }
    }

    if !token.is_empty() {
        tokens.push(Token::Id(token.clone()))
    }

    tokens.push(Token::EOF);
    tokens
}
