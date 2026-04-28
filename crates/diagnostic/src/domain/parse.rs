use strum::AsRefStr;

use crate::{Category, DiagnosticLike};

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

impl DiagnosticLike for ParseSymptom {
    fn message(&self) -> String {
        match self {
            Self::UnexpectedToken => "invalid syntax".into(),
            Self::Custom(msg) => msg.clone(),
            Self::EmptyParens { suggested: _ } => "empty parentheses are not needed".into(),
            Self::UnusedValue { value } => format!("unused value: `{value}`"),
            Self::DirectFileImport { path } => format!("cannot import `.gin` files directly: `{}`", path),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::UnexpectedToken | Self::Custom(_) => None,
            Self::EmptyParens { suggested } => Some(format!("remove the parentheses: `{suggested}`")),
            Self::UnusedValue { .. } => Some("did you mean to indent this as part of the previous expression?".into()),
            Self::DirectFileImport { .. } => Some("remove the `.gin` extension and import the module folder instead".into()),
        }
    }

    fn category(&self) -> Category {
        match self {
            Self::EmptyParens { .. } => Category::Help,
            Self::UnusedValue { .. } => Category::Info,
            _ => Category::Flaw,
        }
    }
}
