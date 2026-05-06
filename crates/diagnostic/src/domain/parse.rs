use crate::{Category, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, strum::AsRefStr)]
#[non_exhaustive]
pub enum ParseSymptom {
    #[strum(serialize = "parse-unexpected-token")]
    UnexpectedToken,
    #[strum(serialize = "parse-custom")]
    Custom(String),
    #[strum(serialize = "parse-empty-parens")]
    EmptyParens { suggested: String },
    #[strum(serialize = "parse-unused-value")]
    UnusedValue { value: String },
    #[strum(serialize = "parse-direct-file-import")]
    DirectFileImport { path: String },
}

impl DiagnosticLike for ParseSymptom {
    fn message(&self) -> String {
        match self {
            Self::UnexpectedToken => "invalid syntax".into(),
            Self::Custom(msg) => msg.clone(),
            Self::EmptyParens { suggested: _ } => "empty parentheses are not needed".into(),
            Self::UnusedValue { value } => format!("unused value: `{value}`"),
            Self::DirectFileImport { path } => {
                format!("cannot import `.gin` files directly: `{}`", path)
            }
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::UnexpectedToken | Self::Custom(_) => None,
            Self::EmptyParens { suggested } => {
                Some(format!("remove the parentheses: `{suggested}`"))
            }
            Self::UnusedValue { .. } => {
                Some("did you mean to indent this as part of the previous expression?".into())
            }
            Self::DirectFileImport { .. } => {
                Some("remove the `.gin` extension and import the module folder instead".into())
            }
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
