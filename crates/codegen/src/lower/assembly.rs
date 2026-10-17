use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use melior::ir::operation::OperationLike;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_checked_assembly(
        &self,
        assembly: &typecheck::assembly::CheckedAssembly,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let output_count = usize::from(assembly.result.is_some()) + assembly.outputs.len();
        if assembly.inputs.len() != assembly.operands.len()
            || assembly.inputs.len() != assembly.tied_output_indices.len()
            || (assembly.result.is_some() && !assembly.outputs.is_empty())
        {
            return self.invalid_checked_assembly("inconsistent operand or result counts");
        }
        let Some(record_expression) = self.program.expr(assembly.record) else {
            return self.invalid_checked_assembly("missing input record");
        };
        let input_record = self
            .program
            .type_registry
            .resolved_definition_for_type(record_expression.ty);
        let mut projections = Vec::with_capacity(assembly.operands.len());
        for ((input, (name, operand)), tie) in assembly
            .inputs
            .iter()
            .zip(&assembly.operands)
            .zip(&assembly.tied_output_indices)
        {
            let tied_register = tie.and_then(|index| {
                if let Some(result) = &assembly.result {
                    (index == 0).then_some(result)
                } else {
                    assembly.outputs.get(index).map(|output| &output.register)
                }
            });
            if input.parameter != name.as_str()
                || input.tie != tie.is_some()
                || tie.is_some_and(|index| index >= output_count)
                || (tie.is_some() && tied_register != Some(&input.register))
            {
                return self.invalid_checked_assembly("inconsistent operand names or output ties");
            }
            let Some(expression) = self.program.expr(*operand) else {
                return self.invalid_checked_assembly("missing operand projection");
            };
            let typecheck::TypedExprKind::AggregateGet { base, index } = expression.kind else {
                return self.invalid_checked_assembly("operand is not a record projection");
            };
            if *base != assembly.record {
                return self
                    .invalid_checked_assembly("operand does not project from the input record");
            }
            let field = match &input_record {
                Ty::Record { fields, .. } => fields
                    .get(*index)
                    .map(|(name, ty)| (Some(name), ty.as_ref())),
                Ty::Sequence(fields) => fields
                    .get(*index)
                    .map(|field| (field.name.as_ref(), field.ty.as_ref())),
                _ => None,
            };
            if !field
                .is_some_and(|(field_name, ty)| field_name == Some(name) && ty == expression.ty)
            {
                return self
                    .invalid_checked_assembly("operand projection does not match its input field");
            }
            projections.push((*index, self.ty_to_mlir(expression.ty)));
        }
        let result_definition = self
            .program
            .type_registry
            .resolved_definition_for_type(result_ty);
        if !assembly.outputs.is_empty() {
            let names: Vec<_> = match &result_definition {
                Ty::Record { fields, .. } => fields.iter().map(|(name, _)| name.as_str()).collect(),
                Ty::Sequence(fields) => fields
                    .iter()
                    .filter_map(|field| field.name.as_ref().map(|name| name.as_str()))
                    .collect(),
                _ => Vec::new(),
            };
            if names.len() != assembly.outputs.len()
                || !names
                    .iter()
                    .zip(&assembly.outputs)
                    .all(|(name, output)| *name == output.name)
            {
                return self.invalid_checked_assembly(
                    "output names or order do not match the result fields",
                );
            }
        }
        let record_fields = if !assembly.outputs.is_empty() {
            match result_definition {
                Ty::Record { fields, .. } => Some(fields),
                _ => None,
            }
        } else {
            None
        };
        let result_types = if let Some(fields) = &record_fields {
            fields.iter().map(|(_, ty)| self.ty_to_mlir(ty)).collect()
        } else if assembly.outputs.is_empty() {
            assembly
                .result
                .as_ref()
                .map(|_| self.result_types_to_mlir(result_ty))
                .unwrap_or_default()
        } else {
            self.result_types_to_mlir(result_ty)
        };
        if result_types.len() != output_count
            || (output_count == 0 && !self.is_empty_product(result_ty))
        {
            return self
                .invalid_checked_assembly("output count does not match the result representation");
        }
        let record = self.lower_typed_expr(assembly.record, block, symtab)?;
        let operands: Vec<_> = projections
            .into_iter()
            .map(|(index, ty)| {
                block.append_op(self.mlir.llvm_extractvalue(
                    record,
                    index as i64,
                    ty,
                    self.location(),
                ))
            })
            .collect();
        let constraints = assembly.llvm_constraints();
        let mut attributes = vec![
            (
                Identifier::new(self.mlir, "asm_string"),
                StringAttribute::new(self.mlir, &assembly.template).into(),
            ),
            (
                Identifier::new(self.mlir, "constraints"),
                StringAttribute::new(self.mlir, &constraints.join(",")).into(),
            ),
            (
                Identifier::new(self.mlir, "asm_dialect"),
                IntegerAttribute::new(self.mlir.i64(), i64::from(assembly.dialect == "intel"))
                    .into(),
            ),
        ];
        if assembly.side_effects {
            attributes.push((
                Identifier::new(self.mlir, "has_side_effects"),
                melior::ir::Attribute::unit(self.mlir),
            ));
        }
        let operation_result_types = if result_types.len() > 1 {
            vec![melior::dialect::llvm::r#type::r#struct(
                self.mlir,
                &result_types,
                false,
            )]
        } else {
            result_types.clone()
        };
        let operation = OperationBuilder::new("llvm.inline_asm", self.location())
            .add_attributes(&attributes)
            .add_operands(&operands)
            .add_results(&operation_result_types)
            .build()
            .map_err(|error| {
                // codegen-assembly-invalid
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::AssemblyInvalid),
                    format!("failed to build inline assembly: {error}"),
                );
            })
            .ok()?;
        let operation = block.append_operation(operation);
        if record_fields.is_some() {
            let output: Value<'c, 'c> = operation
                .result(0)
                .expect("inline assembly with named outputs declares one physical result")
                .into();
            let record_ty = self.ty_to_mlir(result_ty);
            if output.r#type() == record_ty {
                return Some(LoweredValue::scalar(output));
            }
            let mut record = block.append_op(self.mlir.llvm_undef(record_ty, self.location()));
            for (index, ty) in result_types.iter().enumerate() {
                let value = if result_types.len() == 1 {
                    output
                } else {
                    block.append_op(self.mlir.llvm_extractvalue(
                        output,
                        index as i64,
                        *ty,
                        self.location(),
                    ))
                };
                record = block.append_op(self.mlir.llvm_insertvalue(
                    record,
                    value,
                    index as i64,
                    self.location(),
                ));
            }
            return Some(LoweredValue::new(vec![record]));
        }
        if result_types.len() <= 1 {
            return Some(LoweredValue::new(
                operation.results().map(Value::from).collect(),
            ));
        }
        let aggregate = operation.result(0).ok()?.into();
        Some(LoweredValue::new(
            result_types
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    block.append_op(self.mlir.llvm_extractvalue(
                        aggregate,
                        index as i64,
                        *ty,
                        self.location(),
                    ))
                })
                .collect(),
        ))
    }

    fn invalid_checked_assembly(&self, message: &str) -> Option<LoweredValue<'c>> {
        // codegen-assembly-invalid
        self.emit_symptom(
            DiagnosticCode::Codegen(CodegenCode::AssemblyInvalid),
            message.to_owned(),
        );
        None
    }
}
