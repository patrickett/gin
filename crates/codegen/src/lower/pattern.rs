use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use typecheck::layout::LayoutKind;

struct SequencePatternParts {
    head: Ty,
    tail: Ty,
    length: usize,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    fn sequence_pattern_parts(&self, ty: &Ty) -> Option<SequencePatternParts> {
        match self.program.type_registry.resolved_definition_for_type(ty) {
            Ty::Array { elem, size } => {
                let length = usize::try_from(size.as_const_int()?).ok()?;
                Some(SequencePatternParts {
                    head: (*elem).clone(),
                    tail: Ty::Array {
                        elem,
                        size: ast::NormalExpr::from(length.saturating_sub(1) as i128),
                    },
                    length,
                })
            }
            Ty::Sequence(fields) => Some(SequencePatternParts {
                head: fields
                    .first()
                    .map(|field| (*field.ty).clone())
                    .unwrap_or(Ty::Unit),
                tail: Ty::Sequence(fields.iter().skip(1).cloned().collect()),
                length: fields.len(),
            }),
            _ => None,
        }
    }

    fn split_pattern_sequence(
        &self,
        block: &BlockRef<'c, 'c>,
        value: Value<'c, 'c>,
        ty: &Ty,
    ) -> Option<(Value<'c, 'c>, Ty, Value<'c, 'c>, Ty)> {
        let reference =
            typecheck::ty::reference_semantics_for_type(ty, Some(self.program.type_registry));
        let subject_ty = reference
            .as_ref()
            .map(|reference| &reference.pointee)
            .unwrap_or(ty);
        let parts = self.sequence_pattern_parts(subject_ty)?;
        if let Some(reference) = &reference {
            let layout = self
                .target_layout
                .layout_ty(subject_ty, Some(self.program.type_registry))
                .ok()?;
            let offset = match layout.kind {
                LayoutKind::Array { stride, .. } => stride,
                LayoutKind::Product { field_offsets } => {
                    field_offsets.get(1).copied().unwrap_or(layout.size)
                }
                _ => return None,
            };
            let offset = block.const_i64(self.mlir, i64::try_from(offset).ok()?, self.location());
            let tail = block.gep_i8(self, value, offset, self.location())?;
            let mutable =
                reference.permission == typecheck::type_registry::ReferencePermission::Mutate;
            return Some((
                value,
                Ty::Ref {
                    inner: Box::new(parts.head),
                    mutable,
                },
                tail,
                Ty::Ref {
                    inner: Box::new(parts.tail),
                    mutable,
                },
            ));
        }
        let head = if parts.length == 0 {
            block.append_op(
                self.mlir
                    .llvm_undef(self.ty_to_mlir(&parts.head), self.location()),
            )
        } else {
            block.append_op(self.mlir.llvm_extractvalue(
                value,
                0,
                self.ty_to_mlir(&parts.head),
                self.location(),
            ))
        };
        let mut tail = block.append_op(
            self.mlir
                .llvm_undef(self.ty_to_mlir(&parts.tail), self.location()),
        );
        for index in 1..parts.length {
            let field_ty = match &parts.tail {
                Ty::Array { elem, .. } => elem.as_ref(),
                Ty::Sequence(fields) => &fields[index - 1].ty,
                _ => return None,
            };
            let field = block.append_op(self.mlir.llvm_extractvalue(
                value,
                index as i64,
                self.ty_to_mlir(field_ty),
                self.location(),
            ));
            tail = block.append_op(self.mlir.llvm_insertvalue(
                tail,
                field,
                (index - 1) as i64,
                self.location(),
            ));
        }
        Some((head, parts.head, tail, parts.tail))
    }

    pub(super) fn lower_list_pattern_condition(
        &self,
        block: &BlockRef<'c, 'c>,
        value: Value<'c, 'c>,
        ty: &Ty,
        pattern: &Pattern,
    ) -> Option<Value<'c, 'c>> {
        let parts = self.sequence_pattern_parts(ty)?;
        match pattern {
            Pattern::ListEmpty => Some(block.const_int(
                self.mlir,
                self.mlir.i1(),
                i128::from(parts.length == 0),
                self.location(),
            )),
            Pattern::ListCons { .. } if parts.length == 0 => {
                Some(block.const_int(self.mlir, self.mlir.i1(), 0, self.location()))
            }
            Pattern::ListCons { head, tail } => {
                let (head_value, head_ty, tail_value, tail_ty) =
                    self.split_pattern_sequence(block, value, ty)?;
                let head =
                    self.lower_pattern_condition(block, head_value, &head_ty, &head.value)?;
                let tail =
                    self.lower_pattern_condition(block, tail_value, &tail_ty, &tail.value)?;
                Some(block.append_op(self.mlir.build_binop(
                    "arith.andi",
                    head,
                    tail,
                    self.mlir.i1(),
                    self.location(),
                )))
            }
            _ => None,
        }
    }

    pub(super) fn lower_constructor_payload_condition(
        &self,
        block: &BlockRef<'c, 'c>,
        value: Value<'c, 'c>,
        ty: &Ty,
        pattern: &Pattern,
        tag_matches: Value<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let Pattern::Generic {
            name,
            params,
            param_spans,
            ..
        } = pattern
        else {
            return Some(tag_matches);
        };
        let resolved = self.program.type_registry.resolved_definition_for_type(ty);
        let Ty::Union { variants, .. } = &resolved else {
            return None;
        };
        let (variant_index, variant) = variants
            .iter()
            .enumerate()
            .find(|(_, variant)| variant.name == *name)?;
        if param_spans.len() != variant.fields.len() {
            // codegen-pattern-arity
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::PatternArity),
                format!(
                    "constructor pattern `{name}` has {} arguments but its variant has {} fields",
                    param_spans.len(),
                    variant.fields.len()
                ),
            );
            return None;
        }
        let mut constraints = Vec::new();
        for (slot, (parameter, _)) in param_spans.iter().enumerate() {
            let (_, kind) = params.iter().find(|(name, _)| name == parameter)?;
            let constraint = match kind {
                ParameterKind::Generic | ParameterKind::Proposition(_) => {
                    Pattern::Nominal(*parameter, SpanId::INVALID)
                }
                ParameterKind::Tagged(expr)
                | ParameterKind::ValueParam { ty: expr }
                | ParameterKind::Inferred { ty: expr } => Pattern::from_expr(expr.value.clone()),
                ParameterKind::Default(expr) => match &expr.value {
                    Expr::Lit(_) | Expr::TagCall(_) => Pattern::from_expr(expr.value.clone()),
                    Expr::AnonymousTag(name) => {
                        Pattern::Qualified(Spanned::new(ModPath::new(*name, vec![]), expr.span_id))
                    }
                    Expr::FnCall(call)
                        if call.args.as_ref().is_none_or(Vec::is_empty)
                            && call.path.value.segments.is_empty() =>
                    {
                        Pattern::Qualified(call.path.clone())
                    }
                    _ => {
                        // codegen-pattern-parameter-unsupported
                        self.emit_symptom(
                            DiagnosticCode::Codegen(CodegenCode::PatternParameterUnsupported),
                            "constructor payload pattern is not supported".into(),
                        );
                        return None;
                    }
                },
            };
            if !constraint.is_catch_all_pattern() {
                constraints.push((slot, constraint));
            }
        }
        if constraints.is_empty() {
            return Some(tag_matches);
        }
        let then_region = Region::new();
        let else_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block().unwrap();
        let mut matched = then_block.const_int(self.mlir, self.mlir.i1(), 1, self.location());
        for (slot, constraint) in constraints {
            let field_ty = &variant.fields[slot].1;
            let field =
                self.lower_sum_payload(&then_block, value, ty, variant_index, slot, field_ty)?;
            let condition =
                self.lower_pattern_condition(&then_block, field, field_ty, &constraint)?;
            matched = then_block.append_op(self.mlir.build_binop(
                "arith.andi",
                matched,
                condition,
                self.mlir.i1(),
                self.location(),
            ));
        }
        then_block.append_operation(scf_dialect::r#yield(&[matched], self.location()));
        let else_block = Block::new(&[]);
        else_region.append_block(else_block);
        let else_block = else_region.first_block().unwrap();
        let unmatched = else_block.const_int(self.mlir, self.mlir.i1(), 0, self.location());
        else_block.append_operation(scf_dialect::r#yield(&[unmatched], self.location()));
        let operation = block.append_scf_if(
            tag_matches,
            &[self.mlir.i1()],
            then_region,
            else_region,
            self.location(),
        );
        Some(operation.result(0).ok()?.into())
    }

    pub(crate) fn bind_pattern_payload_fields(
        &self,
        block: &BlockRef<'c, 'c>,
        pattern: &Pattern,
        value: Value<'c, 'c>,
        ty: &Ty,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        match pattern {
            Pattern::Nominal(name, _)
                if name.as_str() != "_"
                    && name
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_lowercase()) =>
            {
                symtab.insert(*name, value, ty.clone(), false);
            }
            Pattern::ListCons { head, tail } => {
                let (head_value, head_ty, tail_value, tail_ty) =
                    self.split_pattern_sequence(block, value, ty)?;
                self.bind_pattern_payload_fields(block, &head.value, head_value, &head_ty, symtab)?;
                self.bind_pattern_payload_fields(block, &tail.value, tail_value, &tail_ty, symtab)?;
            }
            Pattern::Generic {
                name, param_spans, ..
            } => {
                let reference = typecheck::ty::reference_semantics_for_type(
                    ty,
                    Some(self.program.type_registry),
                );
                let subject_ty = reference
                    .as_ref()
                    .map(|reference| &reference.pointee)
                    .unwrap_or(ty);
                let resolved = self
                    .program
                    .type_registry
                    .resolved_definition_for_type(subject_ty);
                let Ty::Union { variants, .. } = &resolved else {
                    return None;
                };
                let (index, variant) = variants
                    .iter()
                    .enumerate()
                    .find(|(_, variant)| variant.name == *name)?;
                for (slot, (param, _)) in param_spans.iter().enumerate() {
                    if param.as_str() == "_" {
                        continue;
                    }
                    let (_, field_ty) = variant.fields.get(slot)?;
                    let (field, field_ty) = if let Some(reference) = &reference {
                        let offset = self.sum_payload_offset(subject_ty, index, slot)?;
                        let offset = block.const_i64(
                            self.mlir,
                            i64::try_from(offset).ok()?,
                            self.location(),
                        );
                        let pointer = block.gep_i8(self, value, offset, self.location())?;
                        (
                            pointer,
                            Ty::Ref {
                                inner: field_ty.clone(),
                                mutable: reference.permission
                                    == typecheck::type_registry::ReferencePermission::Mutate,
                            },
                        )
                    } else {
                        (
                            self.lower_sum_payload(
                                block, value, subject_ty, index, slot, field_ty,
                            )?,
                            (**field_ty).clone(),
                        )
                    };
                    symtab.insert(*param, field, field_ty, false);
                }
            }
            _ => {}
        }
        Some(())
    }
}
