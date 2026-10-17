use ast::{BindValue, DeclareValue, Expr, FileAst, HasMember, ParameterKind};
use internment::Intern;
use std::path::Path;

use super::{DefLocation, ParsedFile, body_import_def_location, lexical};

pub(super) fn definition(
    file: &Path,
    ast: &FileAst,
    source: &str,
    byte: usize,
    read: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<DefLocation> {
    let resolver = Members {
        file,
        ast,
        source,
        read,
    };
    let (expr, _) = ast.expr_at_byte(byte)?;
    if let Expr::RecordGet { base, field } = expr {
        let owner = resolver.expression_type(&base.value, byte)?;
        return Some(resolver.member(&owner, *field)?.0);
    }
    let Expr::FnCall(call) = expr else {
        return None;
    };
    if call.syntax == ast::CallSyntax::Receiver {
        let receiver = call.args.as_ref()?.first()?;
        let owner = resolver.expression_type(&receiver.value, byte)?;
        return Some(resolver.member(&owner, call.path.root)?.0);
    }
    let index = call
        .path
        .segment_spans
        .iter()
        .position(|span| ast.span_table.contains(*span, byte))?;
    let mut owner = resolver.receiver(call.path.root, byte)?;
    for (index, name) in call.path.segments[..=index].iter().enumerate() {
        let (location, ty) = resolver.member(&owner, *name)?;
        if ast
            .span_table
            .contains(call.path.segment_spans[index], byte)
        {
            return Some(location);
        }
        owner = ty?;
    }
    None
}

struct Members<'a> {
    file: &'a Path,
    ast: &'a FileAst,
    source: &'a str,
    read: &'a dyn Fn(&Path) -> Option<ParsedFile>,
}

impl Members<'_> {
    fn expression_type(&self, expr: &Expr, byte: usize) -> Option<DefLocation> {
        match expr {
            Expr::SelfRef => self.tag(Intern::from_ref("self"), byte),
            Expr::RecordGet { base, field } => {
                let owner = self.expression_type(&base.value, byte)?;
                self.member(&owner, *field)?.1
            }
            Expr::FnCall(call) if call.path.is_bare() && call.args.is_none() => {
                self.receiver(call.path.root, byte)
            }
            _ => self.initializer(expr, byte),
        }
    }

    fn tag(&self, name: Intern<String>, byte: usize) -> Option<DefLocation> {
        if name.as_str() == "Self" || name.as_str() == "self" {
            let decl = self
                .ast
                .tags
                .values()
                .find(|decl| self.ast.span_table.contains(decl.span, byte))?;
            let span = self.ast.span_table.get(decl.name_span);
            return Some(DefLocation {
                file: self.file.to_path_buf(),
                byte_range: span.start()..span.end(),
            });
        }
        if let Some(decl) = self.ast.tags.get(&name) {
            let span = self.ast.span_table.get(decl.name_span);
            return Some(DefLocation {
                file: self.file.to_path_buf(),
                byte_range: span.start()..span.end(),
            });
        }
        for import in &self.ast.uses {
            for import in &import.0 {
                if let ast::ImportSource::LocalBundle(bundle) = &import.source {
                    for member in &bundle.members {
                        if member.alias.unwrap_or(member.flat_name()) == name {
                            let byte = self.ast.span_table.get(member.span).start();
                            return body_import_def_location(
                                self.file,
                                self.ast,
                                self.source,
                                byte,
                                self.read,
                            );
                        }
                    }
                }
            }
        }
        None
    }

    fn receiver(&self, name: Intern<String>, byte: usize) -> Option<DefLocation> {
        if let Some(owner) = self.tag(name, byte) {
            return Some(owner);
        }
        let span = lexical::definition_span(self.ast, name.as_str(), byte)?;
        if let Some((Expr::Bind(bind), _)) = self.ast.expr_at_byte(span.start) {
            if let Some((name, _)) = &bind.type_annotation {
                return self.tag(*name, span.start);
            }
            if let Some(ty) = &bind.return_tag {
                return self.annotation(&ty.value, span.start);
            }
            if let BindValue::Expr(expr) = &bind.value {
                return self.initializer(&expr.value, span.start);
            }
        }
        let mut params = self
            .ast
            .defs
            .values()
            .filter_map(|bind| bind.params.as_ref())
            .collect::<Vec<_>>();
        for decl in self.ast.tags.values() {
            if let DeclareValue::Has(members) = &decl.value {
                for member in members {
                    if let HasMember::Function(function) = member {
                        params.push(&function.params);
                    }
                }
            }
        }
        for param in params.into_iter().flat_map(|params| params.values()) {
            if self.ast.span_table.get(param.name_span).start() != span.start {
                continue;
            }
            match &param.kind {
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => return self.annotation(&ty.value, byte),
                _ => {}
            }
        }
        None
    }

    fn annotation(&self, expr: &Expr, byte: usize) -> Option<DefLocation> {
        match expr {
            Expr::AnonymousTag(name) => self.tag(*name, byte),
            Expr::TagCall(call) => self.tag(call.name, byte),
            Expr::FnCall(call) if call.path.is_bare() => self.tag(call.path.root, byte),
            Expr::Ref { inner, .. } => self.annotation(&inner.value, byte),
            _ => None,
        }
    }

    fn initializer(&self, expr: &Expr, byte: usize) -> Option<DefLocation> {
        if let Some(owner) = self.annotation(expr, byte) {
            return Some(owner);
        }
        let Expr::FnCall(call) = expr else {
            return None;
        };
        let mut owner = self.tag(call.path.root, byte)?;
        for name in &call.path.segments {
            owner = self.member(&owner, *name)?.1?;
        }
        Some(owner)
    }

    fn member(
        &self,
        owner: &DefLocation,
        name: Intern<String>,
    ) -> Option<(DefLocation, Option<DefLocation>)> {
        if owner.file != self.file {
            let parsed = (self.read)(&owner.file)?;
            return Members {
                file: &parsed.path,
                ast: &parsed.output.ast,
                source: &parsed.source,
                read: self.read,
            }
            .member(owner, name);
        }
        let decl = self.ast.tags.values().find(|decl| {
            self.ast.span_table.get(decl.name_span).start() == owner.byte_range.start
        })?;
        let DeclareValue::Has(members) = &decl.value else {
            return None;
        };
        let mut matches = members.iter().filter_map(|member| match member {
            HasMember::Property(property) if property.name == name => {
                Some((property.name_span, property.ty.as_deref()))
            }
            HasMember::Function(function) if function.name == name => {
                Some((function.name_span, function.return_ty.as_deref()))
            }
            _ => None,
        });
        let (span, ty) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let span = self.ast.span_table.get(span);
        let location = DefLocation {
            file: self.file.to_path_buf(),
            byte_range: span.start()..span.end(),
        };
        let ty = ty.and_then(|ty| self.annotation(&ty.value, span.start()));
        Some((location, ty))
    }
}
