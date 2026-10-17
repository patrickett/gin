//! Shared parse-output helpers for parser integration tests.

#![allow(dead_code)]

use ast::Variant;
use diagnostic::Category;
use internment::Intern;
use parser::query::{ParseOutput, SourceParseExt};

pub fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

pub fn parse_ok(src: &str) -> ParseOutput {
    let out = src.parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "parse errors: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    out
}

pub fn parse_ok_no_flaws(src: &str) -> ParseOutput {
    let out = src.parse_source_full();
    let flaws: Vec<_> = out
        .symptoms
        .iter()
        .filter(|d| matches!(d.category, Category::Flaw))
        .collect();
    assert!(
        flaws.is_empty(),
        "parse flaws: {:?}",
        flaws.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    out
}

pub fn has_def(out: &ParseOutput, name: &str) -> bool {
    out.ast.defs.contains_key(&intern(name))
}

pub fn has_tag(out: &ParseOutput, name: &str) -> bool {
    out.ast.tags.contains_key(&intern(name))
}

pub fn count_errors(out: &ParseOutput) -> usize {
    out.symptoms
        .iter()
        .filter(|d| d.message.contains("error") || d.message.contains("expected"))
        .count()
}

pub fn count_unused(out: &ParseOutput) -> usize {
    out.symptoms
        .iter()
        .filter(|d| d.message.contains("unused"))
        .count()
}

pub fn count_exprs(out: &ParseOutput) -> usize {
    out.ast.exprs.len()
}

pub fn variant_name(v: &Variant) -> &str {
    match &v.shape().value {
        ast::Pattern::Nominal(name, _) => name.as_str(),
        other => panic!("expected nominal variant, got {other:?}"),
    }
}

pub fn variant_doc(v: &Variant) -> Option<&str> {
    match v {
        Variant::Local {
            doc_comment: Some(d),
            ..
        } => Some(d.value.as_str()),
        Variant::Local {
            doc_comment: None, ..
        }
        | Variant::External { .. } => None,
    }
}
