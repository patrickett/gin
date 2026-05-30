use crate::{Category, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, Hash, strum::AsRefStr)]
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

    /// A `return` statement was found inside an indented body block where it should be at the
    /// outer (unindented) level — e.g. as the closer of a bind body or `if` guard.
    #[strum(serialize = "parse-indented-return")]
    IndentedReturn,
}

impl DiagnosticLike for ParseSymptom {
    fn message(&self) -> String {
        match self {
            Self::UnexpectedToken => "invalid syntax".into(),
            Self::Custom(msg) => msg.clone(),
            Self::EmptyParens { suggested: _ } => "empty parentheses are not needed".into(),
            Self::UnusedValue { value } => format!("unused value: `{value}`"),
            Self::IndentedReturn => {
                "return is inside the indented body block — it should be unindented to the outer level".into()
            }
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
            Self::IndentedReturn => {
                Some("unindent the `return` to close the block — it should be at the same level as the opening keyword".into())
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
            Self::IndentedReturn => Category::Help,
            _ => Category::Flaw,
        }
    }
}
