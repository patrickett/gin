//! `Name is <type expression>`: a transparent alias.
//!
//! `BytePointer is @Byte` makes the name another way to write the type; nothing new is
//! declared, so values, casts and methods are those of the aliased type. An alias is exported
//! and imported like a type: the capsule carries the type it stands for.
use super::*;

/// One alias, local (with its expression, resolved on first use) or imported (resolved).
pub(super) struct Alias<'a> {
    pub name: String,
    pub expression: Option<&'a Expression>,
    pub ty: Option<SourceType>,
    pub active: bool,
    pub span: SpanId,
}

impl<'a> Checker<'a> {
    /// Whether `declaration` names a type rather than listing the members of a union: a pointer,
    /// array or sequence type, or the application of a declared generic type
    /// (`ByteCount is Count(Byte)`), which a union's lone payload variant is not.
    pub(super) fn is_alias(&self, declaration: &TypeDeclaration) -> bool {
        let TypeBody::Is(root) = &declaration.body.value else {
            return false;
        };
        match &root.value {
            Expr::PrefixedType(_) | Expr::Array(_) | Expr::Sequence(_) => true,
            Expr::Call { callee, .. } => {
                let Expr::Name(applied) = &callee.value else {
                    return false;
                };
                self.files.iter().flat_map(|file| &file.items).any(|item| {
                    matches!(&item.value, Item::Type(other)
                        if other.name.value == *applied
                            && other.parameters.is_some()
                            && !std::ptr::eq(&**other, declaration)
                            && !matches!(other.body.value, TypeBody::Is(_)))
                })
            }
            _ => false,
        }
    }

    pub(super) fn declare_alias(
        &mut self,
        declaration: &'a TypeDeclaration,
        span: SpanId,
    ) -> Result<()> {
        let TypeBody::Is(root) = &declaration.body.value else {
            return Err(SourceCheckError::Malformed);
        };
        if declaration.parameters.is_some() {
            return Err(SourceCheckError::Unsupported);
        }
        let name = declaration.name.value.as_str();
        self.charge(self.module.aliases.len() + name.len() + 1)?;
        if self.module.aliases.iter().any(|alias| alias.name == name) {
            return Err(SourceCheckError::Duplicate);
        }
        self.module.aliases.push(Alias {
            name: name.to_owned(),
            expression: Some(root),
            ty: None,
            active: false,
            span,
        });
        Ok(())
    }

    /// The type `name` stands for, when it is an alias.
    pub(super) fn alias_type(&mut self, name: &str) -> Result<Option<SourceType>> {
        self.charge(self.module.aliases.len() + 1)?;
        let Some(position) = self.module.aliases.iter().position(|alias| alias.name == name)
        else {
            return Ok(None);
        };
        if let Some(ty) = self.module.aliases[position].ty {
            return Ok(Some(ty));
        }
        if self.module.aliases[position].active {
            let detail = format!("the alias `{name}` is defined in terms of itself");
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        let expression = self.module.aliases[position]
            .expression
            .ok_or(SourceCheckError::Malformed)?;
        self.module.aliases[position].active = true;
        let resolved = self.resolve_type(expression, &[]);
        self.module.aliases[position].active = false;
        let ty = resolved?;
        self.module.aliases[position].ty = Some(ty);
        Ok(Some(ty))
    }

    /// Resolve every local alias, so one nobody uses is still checked and can be exported.
    pub(super) fn resolve_aliases(&mut self) -> diagnostics::DiagnosticResult<()> {
        for position in 0..self.module.aliases.len() {
            let (name, span) = {
                let alias = &self.module.aliases[position];
                (alias.name.clone(), alias.span)
            };
            self.alias_type(&name)
                .map_err(|error| self.failure(error, span))?;
        }
        Ok(())
    }

    /// The local aliases and the types they stand for, for the capsule.
    pub(super) fn exported_aliases(&self) -> Vec<(String, SourceType)> {
        self.module
            .aliases
            .iter()
            .filter(|alias| alias.expression.is_some())
            .filter_map(|alias| alias.ty.map(|ty| (alias.name.clone(), ty)))
            .collect()
    }
}
