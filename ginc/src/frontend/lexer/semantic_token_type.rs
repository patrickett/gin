use crate::frontend::lexer::Token;
use lsp_types::SemanticTokenType;

// PERF: Consider caching semantic token type results for repeated tokens
pub trait HasSemanticTokenType {
    fn semantic_token_type(&self) -> SemanticTokenType;
    fn semantic_token_type_index(&self) -> Option<usize>;
}

impl<'src> HasSemanticTokenType for Token<'src> {
    fn semantic_token_type(&self) -> SemanticTokenType {
        use Token::*;

        match self {
            Tag(_) => SemanticTokenType::TYPE,
            Comment(_) | DocComment(_) => SemanticTokenType::COMMENT,
            Continue | Derives | Private | Public | Define | Return | Break | Alias | Macro
            | Needs | Then | When | Does | From | For | Loop | Use | Has | And | Def | Where
            | As | Do | If | In | Is | Of | Or | Else => SemanticTokenType::KEYWORD,
            Int(_) | Float(_) => SemanticTokenType::NUMBER,
            String(_) | FormatString(_) => SemanticTokenType::STRING,
            _ => SemanticTokenType::OPERATOR,
        }
    }

    fn semantic_token_type_index(&self) -> Option<usize> {
        use Token::*;

        match self {
            Tag(_) => Some(1),
            Comment(_) | DocComment(_) => Some(2),
            Continue | Derives | Private | Public | Define | Return | Break | Alias | Macro
            | Needs | Then | When | Else | Does | From | For | Loop | Use | Has | And | Def
            | Where | As | Do | If | In | Is | Of | Or => Some(3),
            Int(_) | Float(_) | DotDot => Some(7),
            String(_) | FormatString(_) => Some(6),
            _ => None,
        }
    }
}
