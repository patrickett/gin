use crate::ast::{FileAst, ParameterKind, Parameters};

#[derive(Debug, Clone)]
pub enum CompletionKind {
    Function,
    Variable,
    Tag,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

pub fn completions_for_ast(ast: &FileAst) -> Vec<CompletionCandidate> {
    let mut items = Vec::new();

    for (name, decl) in ast.tags() {
        let detail = decl
            .params()
            .as_ref()
            .map(|p| format!("tag {}{}", name, format_params(p)));
        let documentation = decl.doc_comment().map(|dc| dc.0.clone());
        items.push(CompletionCandidate {
            label: name.to_string(),
            kind: CompletionKind::Tag,
            detail,
            documentation,
        });
    }

    for (name, bind) in ast.defs() {
        let is_fn = bind.params().is_some();
        let detail = bind
            .params()
            .as_ref()
            .map(|p| format!("{}{}", name.as_str(), format_params(p)));
        let documentation = bind.doc_comment().map(|dc| dc.0.clone());
        items.push(CompletionCandidate {
            label: name.as_str().to_string(),
            kind: if is_fn {
                CompletionKind::Function
            } else {
                CompletionKind::Variable
            },
            detail,
            documentation,
        });
    }

    // TODO: fix keywords here, make a single source of truth
    for kw in ["if", "else", "for", "in", "while", "return", "use", "tag"] {
        items.push(CompletionCandidate {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
            detail: None,
            documentation: None,
        });
    }

    items
}

pub fn format_params(params: &Parameters) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|(name, kind)| match kind {
            ParameterKind::Generic => name.to_string(),
            ParameterKind::Tagged(tag) => format!("{name} {tag}"),
            ParameterKind::Default(expr) => format!("{name}: {expr:?}"),
        })
        .collect();
    format!("({})", parts.join(", "))
}
