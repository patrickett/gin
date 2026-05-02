use crate::{Category, Diagnostic, DiagnosticLike, SpanTable};

#[derive(Default, Debug, Clone, PartialEq, Eq, strum::AsRefStr)]
pub enum LexSymptom {
    #[strum(serialize = "lex-unclosed-string")]
    UnclosedString,
    #[strum(serialize = "lex-invalid-integer")]
    InvalidInteger,
    #[strum(serialize = "lex-invalid-float")]
    InvalidFloat,
    #[strum(serialize = "lex-overflow-indent")]
    OverflowIndent,
    #[default]
    #[strum(serialize = "lex-unexpected-character")]
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

impl DiagnosticLike for LexSymptom {
    fn message(&self) -> String {
        match self {
            Self::UnclosedString => "unclosed string literal".into(),
            Self::InvalidInteger => "integer literal out of range".into(),
            Self::InvalidFloat => "float literal out of range".into(),
            Self::OverflowIndent => "indentation overflow".into(),
            Self::UnexpectedCharacter => "unexpected character".into(),
        }
    }
}

impl LexSymptom {
    /// Custom rendering for unclosed strings: inserts a suggestion quote into the source.
    pub fn render_custom(
        &self,
        diag: &Diagnostic,
        span_table: &SpanTable,
        source: &str,
        filename: &str,
    ) -> bool {
        if !matches!(self, Self::UnclosedString) {
            return false;
        }

        use ariadne::{Label, Report, ReportKind, Source};
        use std::ops::Range;

        let span = span_table.get(diag.span_id);
        let len = source.len();
        let start = span.start.min(len);
        let end = span.end.max(start).min(len);

        if start >= end {
            return false;
        }

        let mut display_source = source.to_string();
        display_source.insert(end, '\'');

        let quote_span = end..end + 1;
        let kind = ReportKind::Custom(Category::Flaw.as_str(), Category::Flaw.color());
        let span_range: Range<usize> = start..end;
        let msg = format!("[{}] {}", diag.error_code(), diag.message);
        let builder = Report::build(kind, (filename, span_range))
            .with_message(&msg)
            .with_label(
                Label::new((filename, quote_span))
                    .with_color(Category::Flaw.color())
                    .with_message("add single quote here"),
            );

        let report = builder.finish();
        report
            .eprint((filename, Source::from(display_source)))
            .unwrap_or_else(|e| eprintln!("Failed to print diagnostic: {e}"));
        true
    }
}
