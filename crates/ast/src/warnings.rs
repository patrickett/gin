//! Parse-time warning checks that run directly on the AST.
//!
//! These functions are purely structural — they inspect `ast` types and
//! produce `diagnostic` values without needing any type information.

use diagnostic::{Category, Diagnostic, DiagnosticCode};

use crate::span::SpanTable;
use crate::{Bind, BindValue};

impl Bind {
    /// Check whether this bind's name follows lower_snake_case convention.
    pub fn name_case_warning(&self, span_table: &SpanTable) -> Option<Diagnostic> {
        let name = self.name.as_str();
        if name.is_lower_snake_case() {
            return None;
        }
        let suggested = name.to_lower_snake_case();
        Some(Diagnostic {
            code: DiagnosticCode::from("parse-bind-name-not-snake-case"),
            message: format!("bind name `{name}` should be lower snake_case"),
            help_on_span: None,
            help: Some(format!("rename the bind to `{suggested}`")),
            category: Category::Help,
            span: span_table.get(self.name_span),
            related: Vec::new(),
            args: Vec::new(),
        })
    }
}

trait NameCaseExt {
    fn is_lower_snake_case(&self) -> bool;
    fn to_lower_snake_case(&self) -> String;
}

impl NameCaseExt for str {
    fn is_lower_snake_case(&self) -> bool {
        !self.is_empty()
            && self
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
    }

    fn to_lower_snake_case(&self) -> String {
        let mut out = String::new();
        let mut prev_was_underscore = false;
        for (idx, ch) in self.chars().enumerate() {
            if ch == '_' {
                if !out.is_empty() && !prev_was_underscore {
                    out.push('_');
                    prev_was_underscore = true;
                }
                continue;
            }
            if ch.is_ascii_uppercase() {
                if idx > 0 && !prev_was_underscore {
                    out.push('_');
                }
                out.push(ch.to_ascii_lowercase());
                prev_was_underscore = false;
            } else {
                out.push(ch);
                prev_was_underscore = false;
            }
        }
        out.trim_matches('_').to_string()
    }
}

/// Extension trait for bind-warning diagnostics on `[Bind]`.
pub trait BindWarningsExt {
    /// Warn when a `name Ty` (declaration-only) is followed by `name := expr` (constant bind) in the
    /// same scope, suggesting the user intended a rebindable bind (`name: expr`) instead.
    ///
    /// Only the first such warning per declaration is emitted; subsequent constant binds for the same
    /// name are assumed to be the same pattern repeated.
    fn const_bind_after_declare_warnings(&self, span_table: &SpanTable) -> Vec<Diagnostic>;
}

impl BindWarningsExt for [Bind] {
    fn const_bind_after_declare_warnings(&self, span_table: &SpanTable) -> Vec<Diagnostic> {
        if self.len() < 2 {
            return Vec::new();
        }
        let mut diags = Vec::new();
        if matches!(&self[0].value, BindValue::Unassigned) {
            for bind in self.iter().skip(1) {
                if bind.is_constant() {
                    let name = bind.name.as_str().to_string();
                    diags.push(Diagnostic {
                        code: DiagnosticCode::from("type-const-bind-after-declare"),
                        message: format!("`{name}` was previously declared but is now defined with `:=`; use `:` for a rebindable bind"),
                        help_on_span: None,
                        help: Some(format!("change to `{name}: ...` to make it rebindable")),
                        category: Category::Help,
                        span: span_table.get(bind.name_span),
                        related: Vec::new(),
                        args: Vec::new(),
                    });
                    break;
                }
            }
        }
        diags
    }
}
