use lsp_types::SemanticTokenType;

use crate::frontend::Token;

// PERF: Consider caching semantic token type results for repeated tokens
pub trait HasSemanticTokenType {
    fn semantic_token_type(&self) -> SemanticTokenType;
    fn semantic_token_type_index(&self) -> Option<usize>;
}

impl<'src> HasSemanticTokenType for Token<'src> {
    fn semantic_token_type(&self) -> SemanticTokenType {
        use Token::*;

        match self {
            Id(_) => SemanticTokenType::FUNCTION,
            Tag(_) => SemanticTokenType::TYPE,
            Comment(_) | DocComment(_) => SemanticTokenType::COMMENT,
            Continue | Derives | Private | Public | Define | Return | Break | Alias | Macro
            | Needs | Then | When | Does | From | For | Use | Has | And | Def | Where | As | Do
            | If | In | Is | Of | Or => SemanticTokenType::KEYWORD,
            _ => SemanticTokenType::OPERATOR,
            // TODO: Implement semantic token type indexing for all remaining token types
            // PERF: Complete this to enable full LSP semantic highlighting support
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
        use Token::*;

        match self {
            Id(_) => Some(0),
            Tag(_) => Some(1),
            Comment(_) | DocComment(_) => Some(2),
            Continue | Derives | Private | Public | Define | Return | Break | Alias | Macro
            | Needs | Then | When | Else | Does | From | For | Use | Has | And | Def | Where
            | As | Do | If | In | Is | Of | Or => Some(3),
            Int(_) | Float(_) => Some(11),
            _ => None,
        }
    }
}
