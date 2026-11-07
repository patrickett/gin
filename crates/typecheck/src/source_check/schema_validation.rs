//! Local source admissibility and annotation resolution, before layout installation.
//!
//! Collection and nominal shells precede imports. This phase runs after imports;
//! semantic cycles/contracts and use-site privacy remain separate later checks.
use super::*;

#[cfg(test)]
mod tests;

/// Only this module can turn raw declarations into complete, source-free packets.
/// Successful source validation does not imply semantic layout consistency.
pub(super) struct ValidatedSourceSchemas {
    entries: Vec<ValidatedSourceSchema>,
}

struct ValidatedSourceSchema {
    schema: CheckedSourceAggregate,
    constraints: Vec<contracts::Constraint>,
}

impl ValidatedSourceSchemas {
    pub(super) fn install(self, layouts: &mut [contracts::Layout]) {
        assert_eq!(self.entries.len(), layouts.len());
        for (entry, layout) in self.entries.into_iter().zip(layouts) {
            layout.schema = entry.schema;
            layout.constraints = entry.constraints;
        }
    }
}

impl<'a> Checker<'a> {
    pub(super) fn validate_source_schemas(
        &mut self,
    ) -> diagnostics::DiagnosticResult<ValidatedSourceSchemas> {
        // Keep the two passes: every annotation resolves before any constraint.
        // Repeated field validation is intentional: its errors and work are part
        // of the existing checker contract, not an optimization in this refactor.
        let schemas = self.resolve_source_schemas()?;
        self.charge(schemas.len())?;
        let mut validated = Vec::new();
        for (index, schema) in schemas.into_iter().enumerate() {
            let entry = self
                .validate_source_schema(index, schema)
                .map_err(|error| self.failure(error, self.module.schemas[index].body.span_id))?;
            validated.push(entry);
        }
        Ok(ValidatedSourceSchemas { entries: validated })
    }

    fn validate_source_schema(
        &mut self,
        index: usize,
        schema: CheckedSourceAggregate,
    ) -> Result<ValidatedSourceSchema> {
        let fields = self.source_fields(index)?;
        self.charge(fields.len())?;
        let names: Vec<_> = fields
            .iter()
            .map(|field| field.name.value.as_str())
            .collect();
        let mut constraints = Vec::new();
        // The type's parameters are in scope in its claims (`count * #size(element)`).
        let scope = self.schema_scope(index)?;
        self.proof.generics = scope.iter().map(|(name, _)| *name).collect();
        self.claim_fields = fields.clone();
        self.claim_schema = Some(index);
        let compiled = (|| -> Result<()> {
            for (slot, field) in fields.iter().enumerate() {
                self.compile_constraints(
                    field.annotation.as_ref().ok_or(SourceCheckError::Type)?,
                    slot,
                    &names,
                    &mut constraints,
                )?;
            }
            Ok(())
        })();
        self.proof.generics.clear();
        self.claim_fields.clear();
        self.claim_schema = None;
        compiled?;
        // Constraint terms use field slots or literal constants, not local
        // layouts. No fields or constraints are installed until all succeed.
        Ok(ValidatedSourceSchema {
            schema,
            constraints,
        })
    }

    fn resolve_source_schemas(
        &mut self,
    ) -> diagnostics::DiagnosticResult<Vec<CheckedSourceAggregate>> {
        self.charge(self.module.schemas.len())?;
        let mut output = Vec::new();
        for schema in 0..self.module.schemas.len() {
            let entry = self
                .resolve_source_schema(schema)
                .map_err(|error| self.failure(error, self.module.schemas[schema].body.span_id))?;
            output.push(entry);
        }
        Ok(output)
    }

    fn resolve_source_schema(&mut self, schema: usize) -> Result<CheckedSourceAggregate> {
        let scope = self.schema_scope(schema)?;
        self.fill_numbers(schema)?;
        let declaration = self.module.schemas[schema];
        self.charge(declaration.name.value.len() + scope.len())?;
        let mut parameters = Vec::new();
        for (name, _) in &scope {
            self.charge(name.len())?;
            parameters.push((*name).to_owned());
        }
        let mut fields = Vec::new();
        if let TypeBody::Domain(written) = &declaration.body.value {
            // The hidden integer a linear domain wraps; never written in source.
            let (lower, upper) = self.domain_bounds(written)?;
            fields.push(CheckedSourceField {
                name: "value".to_owned(),
                ty: SourceType::Integer { lower, upper },
                private: true,
                register: None,
                borrow: None,
                length: None,
            });
        }
        let written = self.source_fields(schema)?;
        let siblings: Vec<&str> = written.iter().map(|field| field.name.value.as_str()).collect();
        for field in written {
            self.validate_field(field)?;
            let (ty, borrow, length) = self.resolve_field_annotation(
                field.annotation.as_ref().ok_or(SourceCheckError::Type)?,
                &scope,
                &siblings,
            )?;
            self.charge(field.name.value.len() + 1)?;
            let register = self.field_register(field)?;
            if let Some(register) = &register
                && fields
                    .iter()
                    .any(|other| other.register.as_ref() == Some(register))
            {
                return Err(SourceCheckError::Duplicate);
            }
            fields.push(CheckedSourceField {
                name: field.name.value.clone(),
                ty,
                private: field
                    .attributes
                    .iter()
                    .any(|attribute| matches!(attribute.value, FieldAttribute::Private { .. })),
                register,
                borrow,
                length,
            });
        }
        self.check_length_fields(&fields)?;
        let borrowing = Self::borrowing_of(&fields);
        Ok(CheckedSourceAggregate {
            name: declaration.name.value.clone(),
            parameters,
            fields,
            variants: self.written_variants(schema, &scope)?,
            capability: self.module.schema_capability[schema],
            kind: AggregateKind::of(declaration),
            wrapping: self.module.schema_wrap[schema],
            defaults: self.parameter_defaults(schema)?,
            bounds: self.parameter_bounds(schema),
            provides: self.provided_traits(schema),
            borrowing,
            numbers: self.number_ranges(schema)?,
            domain_claims: self.domain_claims(declaration)?,
            discriminant: None,
        })
    }

    fn source_fields(&mut self, schema: usize) -> Result<Vec<&'a Binding>> {
        self.fields_of(self.module.schemas[schema])
    }

    /// The written fields of a `has` declaration, with every member's syntax checked.
    /// Methods are declared as functions, so only their gaps are validated here.
    pub(super) fn fields_of(
        &mut self,
        declaration: &'a TypeDeclaration,
    ) -> Result<Vec<&'a Binding>> {
        let TypeBody::Has(members) = &declaration.body.value else {
            return Ok(Vec::new());
        };
        self.charge(members.len())?;
        let body = self.range_of(declaration.body.span_id);
        let mut cursor = body.start + "has".len();
        let mut fields: Vec<&Binding> = Vec::new();
        let mut seen = false;
        let mut previous_provision = false;
        for member in members {
            self.child(declaration.body.span_id, member.span_id)?;
            let range = self.range_of(member.span_id);
            let provision = matches!(
                member.value,
                TypeMember::Provision(_) | TypeMember::ProvisionApplied(_)
            );
            // `Type has A and B`: the provisions of a header are joined by `and`.
            let joined = provision
                && previous_provision
                && self.gap_text(range.file, cursor, range.start)?.trim() == "and";
            if !joined {
                self.field_gap(range.file, cursor, range.start, seen)?;
            }
            previous_provision = provision;
            cursor = range.end;
            seen = true;
            let field = match &member.value {
                TypeMember::Binding(field) => field,
                // A provision names a trait; its members are declared as methods.
                TypeMember::Provision(name) => {
                    self.child(member.span_id, name.span_id)?;
                    continue;
                }
                TypeMember::ProvisionApplied(applied) => {
                    self.child(member.span_id, applied.span_id)?;
                    continue;
                }
                TypeMember::Error => return Err(SourceCheckError::Malformed),
            };
            // Methods are declared as functions; their gaps are still checked above.
            if diagnostics::is_method(field) {
                continue;
            }
            self.field_syntax(member.span_id, field)?;
            self.child(member.span_id, field.name.span_id)?;
            let name = self.name(&field.name)?;
            self.charge_product(fields.len(), name.len() + 1)?;
            if fields.iter().any(|other| other.name.value == name) {
                return Err(SourceCheckError::Duplicate);
            }
            if let Some(annotation) = &field.annotation {
                self.child(member.span_id, annotation.span_id)?;
            }
            for attribute in &field.attributes {
                self.child(member.span_id, attribute.span_id)?;
            }
            fields.push(field);
        }
        self.syntax_trivia(body.file, cursor, body.end)?;
        Ok(fields)
    }

    pub(super) fn validate_field(&mut self, field: &'a Binding) -> Result<()> {
        if field.parameters.is_some()
            || field.value.is_some()
            || field.convention.is_some()
            || field.result_binder.is_some()
            || field.contract.is_some()
        {
            return Err(SourceCheckError::Unsupported);
        }
        if field.attributes.len() > 1 {
            return Err(SourceCheckError::Duplicate);
        }
        for attribute in &field.attributes {
            // Fields take `#private` and `#register`; `#infix` marks methods.
            match attribute.value {
                FieldAttribute::Private {
                    hash,
                    private_keyword,
                } => {
                    self.child(attribute.span_id, hash)?;
                    self.child(attribute.span_id, private_keyword)?;
                    self.token(hash, "#")?;
                    self.keyword(private_keyword, "private")?;
                }
                FieldAttribute::Register { hash, keyword, .. } => {
                    self.child(attribute.span_id, hash)?;
                    self.child(attribute.span_id, keyword)?;
                    self.token(hash, "#")?;
                    self.keyword(keyword, "register")?;
                }
                FieldAttribute::Infix { .. } | FieldAttribute::Inline { .. } => {
                    return Err(SourceCheckError::Unsupported);
                }
            }
        }
        Ok(())
    }

    fn field_register(&mut self, field: &'a Binding) -> Result<Option<String>> {
        for attribute in &field.attributes {
            if let FieldAttribute::Register { name, .. } = attribute.value {
                return self.attribute_text(name).map(Some);
            }
        }
        Ok(None)
    }

    /// The contents of a `'text'` attribute argument. These reach the machine verbatim, so
    /// only a plain name is accepted: nothing can smuggle a constraint or a second operand.
    pub(super) fn attribute_text(&mut self, literal: SpanId) -> Result<String> {
        let written = self.span(literal)?;
        let text = quoted(written)
            .filter(|text| plain_name(text))
            .ok_or(SourceCheckError::InstructionTarget)?;
        Ok(text.to_owned())
    }

    /// The text of an `#instruction`: the target library's own assembly, so anything but a
    /// quote or a backslash. `;` separates instructions, and `$name` places a symbol.
    pub(super) fn instruction_text(&mut self, literal: SpanId) -> Result<String> {
        let written = self.span(literal)?;
        let text = quoted(written)
            .filter(|text| !text.is_empty())
            .ok_or(SourceCheckError::InstructionTarget)?;
        self.charge(text.len())?;
        placeholders(text).ok_or(SourceCheckError::InstructionTarget)?;
        // `;` becomes a line break, so one inside quoted data would split it.
        let mut inside = false;
        for byte in text.bytes() {
            match byte {
                b'"' => inside = !inside,
                b';' if inside => return Err(SourceCheckError::InstructionTarget),
                _ => {}
            }
        }
        Ok(text.to_owned())
    }

    /// The mnemonic `#instruction` defines `binding` as, when it does.
    pub(super) fn instruction_of(&self, binding: &'a Binding) -> Option<String> {
        let marked = self
            .module
            .instructions
            .iter()
            .find(|(marked, _)| std::ptr::eq(*marked, binding));
        marked.map(|(_, text)| text.clone())
    }

    /// The memory operation `#load` or `#store` defines `binding` as, when it does.
    pub(super) fn memory_of(&self, binding: &'a Binding) -> Option<MemoryOp> {
        self.module
            .memory
            .iter()
            .find(|(marked, _)| std::ptr::eq(*marked, binding))
            .map(|(_, op)| *op)
    }
}
