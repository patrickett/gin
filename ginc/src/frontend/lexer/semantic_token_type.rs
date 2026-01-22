use lsp_types::SemanticTokenType;

use crate::frontend::Token;

// PERF: Consider caching semantic token type results for repeated tokens
pub trait HasSemanticTokenType {
    fn semantic_token_type(&self) -> SemanticTokenType;
    fn semantic_token_type_index(&self) -> Option<usize>;
}

impl<'src> HasSemanticTokenType for Token<'src> {
    fn semantic_token_type(&self) -> SemanticTokenType {
        match self {
            Token::Id(_) => SemanticTokenType::FUNCTION,
            Token::Tag(_) => SemanticTokenType::TYPE,
            Token::Comment(_) | Token::DocComment(_) => SemanticTokenType::COMMENT,
            Token::Continue
            | Token::Derives
            | Token::Private
            | Token::Public
            | Token::Define
            | Token::Return
            | Token::Break
            | Token::Alias
            | Token::Macro
            | Token::Needs
            | Token::Then
            | Token::When
            | Token::Does
            | Token::From
            | Token::For
            | Token::Use
            | Token::Has
            | Token::And
            | Token::Def
            | Token::Where
            | Token::As
            | Token::Do
            | Token::If
            | Token::In
            | Token::Is
            | Token::Of
            | Token::Or => SemanticTokenType::KEYWORD,
            _ => SemanticTokenType::OPERATOR,
            // TODO: Implement semantic token type indexing for all remaining token types
            // PERF: Complete this to enable full LSP semantic highlighting support
            // Token::Float(_) => todo!(),
            // Token::Int(_) => todo!(),
            // Token::String(_) => todo!(),
            // Token::Ellipsis => todo!(),
            // Token::IsReplacedBy => todo!(),
            // Token::Assignment => todo!(),
            // Token::Turnstile => todo!(),
            // Token::EqEq => todo!(),
            // Token::NotEq => todo!(),
            // Token::LessEq => todo!(),
            // Token::GreaterEq => todo!(),
            // Token::Equals => todo!(),
            // Token::Less => todo!(),
            // Token::Greater => todo!(),
            // Token::Plus => todo!(),
            // Token::Minus => todo!(),
            // Token::Star => todo!(),
            // Token::SlashOr => todo!(),
            // Token::Slash => todo!(),
            // Token::Bar => todo!(),
            // Token::Caret => todo!(),
            // Token::Tilde => todo!(),
            // Token::Dot => todo!(),
            // Token::Pound => todo!(),
            // Token::Colon => todo!(),
            // Token::ColonSemi => todo!(),
            // Token::ParenOpen => todo!(),
            // Token::ParenClose => todo!(),
            // Token::BracketOpen => todo!(),
            // Token::BracketClose => todo!(),
            // Token::CurlyOpen => todo!(),
            // Token::CurlyClose => todo!(),
            // Token::Ampersand => todo!(),
            // Token::Comma => todo!(),
            // Token::Newline => todo!(),
            // Token::Indent => todo!(),
            // Token::Dedent => todo!(),
            // Token::Whitespace => todo!(),
            // Token::Error => todo!(),
        }
    }

    // pub const LEGEND_TYPE: &[SemanticTokenType] = &[
    //     SemanticTokenType::FUNCTION,
    //     SemanticTokenType::STRUCT,
    //     SemanticTokenType::COMMENT,
    //     SemanticTokenType::KEYWORD,
    //     SemanticTokenType::OPERATOR,
    //     SemanticTokenType::VARIABLE,
    //     SemanticTokenType::STRING,
    //     SemanticTokenType::NUMBER,
    //     SemanticTokenType::PARAMETER,
    // ];

    fn semantic_token_type_index(&self) -> Option<usize> {
        match self {
            Token::Id(_) => Some(0),
            Token::Tag(_) => Some(1),
            Token::Comment(_) | Token::DocComment(_) => Some(2),
            Token::Continue
            | Token::Derives
            | Token::Private
            | Token::Public
            | Token::Define
            | Token::Return
            | Token::Break
            | Token::Alias
            | Token::Macro
            | Token::Needs
            | Token::Then
            | Token::When
            | Token::Else
            | Token::Does
            | Token::From
            | Token::For
            | Token::Use
            | Token::Has
            | Token::And
            | Token::Def
            | Token::Where
            | Token::As
            | Token::Do
            | Token::If
            | Token::In
            | Token::Is
            | Token::Of
            | Token::Or => Some(3),
            Token::Int(_) | Token::Float(_) => Some(11),
            _ => None,
        }
    }
}
