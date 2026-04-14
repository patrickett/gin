use crate::SpanId;
use crate::{Category, Symptom, SymptomLike};

#[derive(Default, Debug, Clone, PartialEq)]
pub enum LexSymptom {
    UnclosedString,
    InvalidInteger,
    InvalidFloat,
    OverflowIndent,
    #[default]
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
        let category = Category::Flaw;
        let code: &str;
        let help: Option<String> = None;
        let message: String;

        match self {
            Self::UnclosedString => {
                code = "lex-unclosed-string";
                message = "unclosed string literal".into();
            }
            Self::InvalidInteger => {
                code = "lex-invalid-integer";
                message = "integer literal out of range".into();
            }
            Self::InvalidFloat => {
                code = "lex-invalid-float";
                message = "float literal out of range".into();
            }
            Self::OverflowIndent => {
                code = "lex-overflow-indent";
                message = "indentation overflow".into();
            }
            Self::UnexpectedCharacter => {
                code = "lex-unexpected-character";
                message = "unexpected character".into();
            }
        }

        Symptom {
            code,
            message,
            help,
            span_id,
            category,
        }
    }
}
