//! A flaw (or warning) as structured data, and as the text `ginc --check-new` would print.

use std::sync::Arc;

use diagnostic::{Category, Diagnostic, Span, source_position};
use serde_json::{Value, json};

use typecheck::SourceFact;

use crate::source::{Decl, ParamInfo};

/// Where in which file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub file: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub start: (usize, usize),
    pub end: (usize, usize),
}

impl Located {
    pub fn new(file: &str, source: &str, span: Span) -> Self {
        Self {
            file: file.to_owned(),
            byte_start: span.start(),
            byte_end: span.end(),
            start: source_position(source, span.start()),
            end: source_position(source, span.end()),
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "file": self.file,
            "byte_start": self.byte_start, "byte_end": self.byte_end,
            "start": {"line": self.start.0, "column": self.start.1},
            "end": {"line": self.end.0, "column": self.end.1},
        })
    }
}

/// The declaration a flaw is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct {
    pub kind: &'static str,
    pub name: String,
    pub span: Located,
    pub params: Vec<ParamInfo>,
    pub result: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Flaw {
    pub code: String,
    /// `flaw` (the program is wrong) or `warning`.
    pub severity: &'static str,
    /// `parse` or `check`.
    pub stage: &'static str,
    pub message: String,
    pub span: Option<Located>,
    pub construct: Option<Construct>,
    /// What the checker says it could not establish, when it says.
    pub claim: Option<String>,
    /// Declarations removed before this flaw surfaced: it may be a knock-on of theirs.
    pub knock_on_of: Vec<String>,
    /// The exact expression being checked when the flaw arose (inside `span`).
    pub culprit: Option<Located>,
    /// What the checker had proved about the names in scope.
    pub proved: Option<Vec<SourceFact>>,
    /// For printing as `ginc --check-new` does.
    pub diagnostic: Diagnostic,
    pub source: Arc<str>,
    pub file: String,
}

impl Flaw {
    pub fn is_flaw(&self) -> bool {
        self.severity == "flaw"
    }

    pub fn json(&self) -> Value {
        let declared = self.construct.as_ref().map(|c| c.params.iter().map(param_json).collect::<Vec<_>>());
        let facts = match (&self.proved, declared) {
            (None, None) => Value::Null,
            (proved, declared) => json!({
                // The checker's own facts at this point (intervals, known values, multiples,
                // offsets, relations), absent for a flaw that arose before any were held.
                "proved": proved.as_ref().map(|p| p.iter().map(fact_json).collect::<Vec<_>>()),
                // What the enclosing function declared, as written.
                "declared_parameters": declared,
            }),
        };
        json!({
            "code": self.code,
            "severity": self.severity,
            "stage": self.stage,
            "message": self.message,
            "span": self.span.as_ref().map(Located::json),
            "culprit": self.culprit.as_ref().map(Located::json),
            "construct": self.construct.as_ref().map(|c| json!({
                "kind": c.kind, "name": c.name, "span": c.span.json(),
                "result": c.result,
            })),
            "claim": self.claim,
            "facts": facts,
            "knock_on_of": self.knock_on_of,
        })
    }

    /// The same text `ginc --check-new` prints (ariadne), on standard error.
    pub fn print(&self) {
        if self.span.is_some() {
            self.diagnostic.print(&self.source, &self.file);
        } else {
            eprintln!("{}: {}[{}]: {}", self.file, self.severity, self.code, self.message);
        }
    }

    pub fn sort_key(&self) -> (usize, usize, String) {
        (
            self.span.as_ref().map_or(usize::MAX, |s| s.byte_start),
            self.span.as_ref().map_or(0, |s| s.byte_end),
            self.code.clone(),
        )
    }
}

fn big(n: &i256::I256) -> Value {
    match i64::try_from(n.as_i128()) {
        Ok(small) if n.as_i128() == i128::from(small) => json!(small),
        _ => json!(n.to_string()),
    }
}

/// A fact as structured data and as the claim it makes.
pub fn fact_json(fact: &SourceFact) -> Value {
    let text = fact_text(fact);
    match fact {
        SourceFact::Interval { name, lower, upper } => json!({"kind":"interval","name":name,"lower":big(lower),"upper":big(upper),"text":text}),
        SourceFact::Known { name, value } => json!({"kind":"known","name":name,"value":big(value),"text":text}),
        SourceFact::Multiple { name, modulus } => json!({"kind":"multiple","name":name,"modulus":big(modulus),"text":text}),
        SourceFact::Offset { name, base, offset } => json!({"kind":"offset","name":name,"base":base,"offset":big(offset),"text":text}),
        SourceFact::Relation { left, operator, right } => json!({"kind":"relation","left":left,"operator":operator,"right":right,"text":text}),
    }
}

pub fn fact_text(fact: &SourceFact) -> String {
    ginc::fact_claim(fact)
}

pub fn param_json(p: &ParamInfo) -> Value {
    json!({"name": p.name, "convention": p.convention, "type": p.ty})
}

pub fn construct_at(file: &str, source: &str, decls: &[Decl], span: Span) -> Option<Construct> {
    let (start, end) = (span.start(), span.end());
    let decl = decls.iter().find(|d| d.start <= start && end <= d.end)?;
    let located = |s: usize, e: usize| {
        let range = Span::new(s, e);
        Located::new(file, source, range)
    };
    if let Some(member) = decl.members.iter().find(|m| m.start <= start && end <= m.end) {
        return Some(Construct {
            kind: "method",
            name: member.name.clone(),
            span: located(member.start, member.end),
            params: member.params.clone(),
            result: member.result.clone(),
        });
    }
    Some(Construct {
        kind: decl.kind,
        name: decl.name.clone(),
        span: located(decl.start, decl.end),
        params: decl.params.clone(),
        result: decl.result.clone(),
    })
}

pub fn from_diagnostic(
    stage: &'static str,
    d: &Diagnostic,
    file: &str,
    source: &Arc<str>,
    decls: &[Decl],
) -> Flaw {
    let located = (!d.span.is_empty()).then(|| Located::new(file, source, d.span));
    Flaw {
        code: d.code.slug().to_string(),
        severity: if d.category == Category::Flaw { "flaw" } else { "warning" },
        stage,
        message: d.message.clone(),
        construct: located.as_ref().and_then(|_| construct_at(file, source, decls, d.span)),
        span: located,
        claim: None,
        culprit: None,
        proved: None,
        knock_on_of: Vec::new(),
        diagnostic: d.clone(),
        source: Arc::clone(source),
        file: file.to_owned(),
    }
}
