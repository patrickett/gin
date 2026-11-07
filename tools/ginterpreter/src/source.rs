//! Reading a source text as the declarations it is made of, so a session can replace one by
//! name, and finding which declaration (and which parameters) a byte offset belongs to.

use ast::{Binding, Expr, Item, SourceFile, Statement, TypeBody, TypeMember};

/// One parameter as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamInfo {
    pub name: String,
    pub convention: Option<String>,
    /// The annotation text as written (`in 0 to 9`, `Point`), or `None` for an unannotated one.
    pub ty: Option<String>,
}

/// A local bound before some point of a function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name: String,
    pub ty: Option<String>,
}

/// One top-level declaration: its leading `#entry`/`#unrestricted`/... marker lines included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    /// `function`, `binding`, `type`, `import`.
    pub kind: &'static str,
    pub name: String,
    /// What a later declaration must equal to replace this one.
    pub key: String,
    pub start: usize,
    pub end: usize,
    pub params: Vec<ParamInfo>,
    /// The result annotation as written.
    pub result: Option<String>,
    /// The `where` clause as written, and further result alternatives.
    pub contract: Option<String>,
    pub alternatives: Vec<String>,
    /// Methods of a `has` block, by byte range.
    pub members: Vec<Member>,
    /// The statements of the body, for locals.
    pub locals: Vec<(usize, LocalInfo)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub params: Vec<ParamInfo>,
    pub result: Option<String>,
    pub contract: Option<String>,
    pub alternatives: Vec<String>,
    pub locals: Vec<(usize, LocalInfo)>,
}

/// A parsed submission.
pub enum Split {
    Empty,
    Declarations(Vec<(Decl, String)>),
    /// A single expression, as written.
    Expression(String),
}

fn text<'a>(file: &'a SourceFile, id: ast::SpanId) -> &'a str {
    let span = file.span_table.get(id);
    file.text.get(span.start()..span.end()).unwrap_or("")
}

fn params(file: &SourceFile, binding: &Binding) -> Vec<ParamInfo> {
    let Some(list) = &binding.parameters else { return Vec::new() };
    list.value
        .iter()
        .map(|p| ParamInfo {
            name: p.value.name.value.clone(),
            convention: p.value.convention.as_ref().map(|c| text(file, c.span_id).trim().to_owned()),
            ty: p.value.annotation.as_ref().map(|a| text(file, a.span_id).trim().to_owned()),
        })
        .collect()
}

fn locals(file: &SourceFile, binding: &Binding) -> Vec<(usize, LocalInfo)> {
    let Some(value) = &binding.value else { return Vec::new() };
    let Expr::Block(block) = &value.expression.value else { return Vec::new() };
    block
        .statements
        .iter()
        .filter_map(|statement| match &statement.value {
            Statement::Binding(local) if local.parameters.is_none() => {
                let end = file.span_table.get(statement.span_id).end();
                Some((
                    end,
                    LocalInfo {
                        name: local.name.value.clone(),
                        ty: local.annotation.as_ref().map(|a| text(file, a.span_id).trim().to_owned()),
                    },
                ))
            }
            _ => None,
        })
        .collect()
}

/// The declarations of `file`, in order, each with the markers that precede it.
pub fn declarations(file: &SourceFile) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut marker: Option<usize> = None;
    for item in &file.items {
        let span = file.span_table.get(item.span_id);
        let (start, end) = (span.start(), span.end());
        let decl = |kind, name: String, key: String, binding: Option<&Binding>| Decl {
            kind,
            name,
            key,
            start: marker.unwrap_or(start),
            end,
            params: binding.map(|b| params(file, b)).unwrap_or_default(),
            result: binding.and_then(|b| b.annotation.as_ref().map(|a| text(file, a.span_id).trim().to_owned())),
            contract: binding.and_then(|b| b.contract.as_ref().map(|c| text(file, c.span_id).trim().to_owned())),
            alternatives: binding.map(|b| b.alternatives.iter().map(|a| text(file, a.span_id).trim().to_owned()).collect()).unwrap_or_default(),
            members: Vec::new(),
            locals: binding.map(|b| locals(file, b)).unwrap_or_default(),
        };
        match &item.value {
            Item::Private | Item::Entry | Item::Load | Item::Store | Item::Wrap | Item::Unrestricted | Item::Instruction { .. } => {
                marker.get_or_insert(start);
                continue;
            }
            Item::Binding(binding) => {
                let name = binding.name.value.clone();
                let (kind, key) = match &binding.parameters {
                    Some(list) => ("function", format!("fn {name}/{}", list.value.len())),
                    None => ("binding", format!("let {name}")),
                };
                out.push(decl(kind, name, key, Some(binding)));
            }
            Item::Type(declaration) => {
                let name = declaration.name.value.clone();
                let key = match &declaration.body.value {
                    TypeBody::Has(_) => {
                        let header = file.text[start..end].lines().next().unwrap_or("").trim();
                        format!("has {header}")
                    }
                    _ => format!("type {name}"),
                };
                let mut d = decl("type", name.clone(), key, None);
                if let TypeBody::Has(members) = &declaration.body.value {
                    for member in members {
                        if let TypeMember::Binding(b) = &member.value {
                            let span = file.span_table.get(member.span_id);
                            d.members.push(Member {
                                name: format!("{name}.{}", b.name.value),
                                start: span.start(),
                                end: span.end(),
                                params: params(file, b),
                                result: b.annotation.as_ref().map(|a| text(file, a.span_id).trim().to_owned()),
                                contract: b.contract.as_ref().map(|c| text(file, c.span_id).trim().to_owned()),
                                alternatives: b.alternatives.iter().map(|a| text(file, a.span_id).trim().to_owned()).collect(),
                                locals: locals(file, b),
                            });
                        }
                    }
                }
                out.push(d);
            }
            Item::Import(_) => {
                let key = format!("import {}", file.text[start..end].trim());
                out.push(decl("import", key.clone(), key, None));
            }
            Item::Expression(_) | Item::Error => {}
        }
        marker = None;
    }
    out
}

/// Split a submission into declarations, or recognise one expression.
pub fn split(source: &str) -> Result<Split, Vec<diagnostic::Diagnostic>> {
    if source.trim().is_empty() {
        return Ok(Split::Empty);
    }
    let file = parser::parse_strict(source)?;
    let expressions = file.items.iter().filter(|i| matches!(i.value, Item::Expression(_))).count();
    if expressions > 0 {
        if file.items.len() == 1 {
            return Ok(Split::Expression(source.trim().to_owned()));
        }
        return Err(vec![diagnostic::Diagnostic::new(
            diagnostic::code::SourceCheckCode::Unsupported,
            "a submission is either declarations or one expression, not both",
        )]);
    }
    let decls = declarations(&file);
    Ok(Split::Declarations(
        decls.into_iter().map(|d| { let t = source[d.start..d.end].to_owned(); (d, t) }).collect(),
    ))
}
