use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Symptom, SymptomCode, SymptomLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum ParseSymptom {
    #[strum(to_string = "parse-unexpected-token")]
    UnexpectedToken,
    #[strum(to_string = "parse-custom")]
    Custom(String),
    #[strum(to_string = "parse-empty-parens")]
    EmptyParens { suggested: String },
    #[strum(to_string = "parse-unused-value")]
    UnusedValue { value: String },
    #[strum(to_string = "parse-direct-file-import")]
    DirectFileImport { path: String },
}

impl SymptomLike for ParseSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let (category, message, help) = match &self {
            Self::UnexpectedToken => (
                Category::Flaw,
                "invalid syntax".into(),
                None,
            ),
            Self::Custom(msg) => (
                Category::Flaw,
                msg.clone(),
                None,
            ),
            Self::EmptyParens { suggested } => (
                Category::Help,
                "empty parentheses are not needed".into(),
                Some(format!("remove the parentheses: `{suggested}`")),
            ),
            Self::UnusedValue { value } => (
                Category::Info,
                format!("unused value: `{value}`"),
                Some("did you mean to indent this as part of the previous expression?".into()),
            ),
            Self::DirectFileImport { path } => (
                Category::Flaw,
                format!("cannot import `.gin` files directly: `{}`", path),
                Some("remove the `.gin` extension and import the module folder instead".into()),
            ),
        };

        Symptom {
            code: SymptomCode::Parse(self),
            message,
            help,
            span_id,
            category,
        }
    }
}
