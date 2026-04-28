use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Symptom, SymptomCode, SymptomLike};

#[derive(Default, Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum LexSymptom {
    #[strum(to_string = "lex-unclosed-string")]
    UnclosedString,
    #[strum(to_string = "lex-invalid-integer")]
    InvalidInteger,
    #[strum(to_string = "lex-invalid-float")]
    InvalidFloat,
    #[strum(to_string = "lex-overflow-indent")]
    OverflowIndent,
    #[default]
    #[strum(to_string = "lex-unexpected-character")]
    UnexpectedCharacter,
}

impl From<std::num::ParseIntError> for LexSymptom {
    fn from(_: std::num::ParseIntError) -> Self {
        LexSymptom::InvalidInteger
    }
}

impl From<std::num::ParseFloatError> for LexSymptom {
    fn from(_: std::num::ParseFloatError) -> Self {
        LexSymptom::InvalidFloat
    }
}

impl SymptomLike for LexSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let message: &str = match self {
            Self::UnclosedString => "unclosed string literal",
            Self::InvalidInteger => "integer literal out of range",
            Self::InvalidFloat => "float literal out of range",
            Self::OverflowIndent => "indentation overflow",
            Self::UnexpectedCharacter => "unexpected character",
        };

        Symptom {
            code: SymptomCode::Lex(self),
            message: message.into(),
            help: None,
            span_id,
            category: Category::Flaw,
        }
    }
}
