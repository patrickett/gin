use crate::SpanId;
use crate::{Category, Symptom, SymptomLike};

pub enum ParseSymptom {
    UnexpectedToken,
    Custom(String),
    EmptyParens { suggested: String },
    UnusedValue { value: String },
    DirectFileImport { path: String },
}

impl SymptomLike for ParseSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let category: Category;
        let code: &str;
        let help: Option<String>;
        let message: String;

        match self {
            Self::UnexpectedToken => {
                category = Category::Flaw;
                code = "parse-unexpected-token";
                message = "invalid syntax".into();
                help = None;
            }
            Self::Custom(msg) => {
                category = Category::Flaw;
                code = "parse-custom";
                message = msg;
                help = None;
            }
            Self::EmptyParens { suggested } => {
                category = Category::Help;
                code = "parse-empty-parens";
                message = "empty parentheses are not needed".into();
                help = Some(format!("remove the parentheses: `{suggested}`"));
            }
            Self::UnusedValue { value } => {
                category = Category::Info;
                code = "parse-unused-value";
                message = format!("unused value: `{value}`");
                help =
                    Some("did you mean to indent this as part of the previous expression?".into());
            }
            Self::DirectFileImport { path } => {
                category = Category::Flaw;
                code = "parse-direct-file-import";
                message = format!("cannot import `.gin` files directly: `{}`", path);
                help = Some("remove the `.gin` extension and import the module folder instead".into());
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
