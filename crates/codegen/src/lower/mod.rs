mod array;
mod assembly;
mod condition;
#[cfg(test)]
mod constant_tests;
mod constructor;
mod context;
mod dependent_array;
mod frame;
mod if_;
mod integer_arithmetic;
mod join;
mod loop_;
mod mlir;
mod pattern;
mod promotion;
mod return_;
mod slice;
mod storage;
mod ty_mapping;
mod when;

pub use context::{CodegenContext, CodegenSourceMap};

use crate::prelude::*;
use diagnostic::code::{
    AddressCode, CodegenCode, LayoutCode, RepresentationCode, SequenceCode, TextCode, TypeCode,
};

use ast::HashFloat;
use diagnostic::{Diagnostic, DiagnosticCode};
use flask::CompileTarget;
use melior::ir::Location;
use melior::ir::operation::{OperationLike, OperationMutLike};

fn integer_constant_bits(value: i256::I256) -> Option<i128> {
    ast::integer::to_i128(value).or_else(|| ast::integer::to_u128(value).map(|bits| bits as i128))
}

fn intrinsic_arith_name(op: &typecheck::typed::IntrinsicOp) -> Option<&'static str> {
    match op {
        typecheck::typed::IntrinsicOp::BitsAdd => Some("arith.addi"),
        typecheck::typed::IntrinsicOp::BitsSubtract => Some("arith.subi"),
        typecheck::typed::IntrinsicOp::BitsMultiply => Some("arith.muli"),
        typecheck::typed::IntrinsicOp::BitsSignedDivide => Some("arith.divsi"),
        typecheck::typed::IntrinsicOp::BitsUnsignedDivide => Some("arith.divui"),
        typecheck::typed::IntrinsicOp::BitsSignedRemainder => Some("arith.remsi"),
        typecheck::typed::IntrinsicOp::BitsUnsignedRemainder => Some("arith.remui"),
        typecheck::typed::IntrinsicOp::BitsAnd => Some("arith.andi"),
        typecheck::typed::IntrinsicOp::BitsOr => Some("arith.ori"),
        typecheck::typed::IntrinsicOp::BitsXor => Some("arith.xori"),
        typecheck::typed::IntrinsicOp::BitsShiftLeft => Some("arith.shli"),
        typecheck::typed::IntrinsicOp::BitsShiftRightLogical => Some("arith.shrui"),
        typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic => Some("arith.shrsi"),
        _ => None,
    }
}

pub struct ResolvedFileInput<'a> {
    pub program: typecheck::ResolvedProgram<'a>,
    pub source_map: CodegenSourceMap<'a>,
}

impl<'a> ResolvedFileInput<'a> {
    pub fn new(program: typecheck::ResolvedProgram<'a>, source_map: CodegenSourceMap<'a>) -> Self {
        Self {
            program,
            source_map,
        }
    }
}

pub struct FileCodegenDiagnostics {
    pub filename: String,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct ModuleCodegenOutput<'c> {
    pub module: Option<melior::ir::Module<'c>>,
    pub diagnostics: Vec<FileCodegenDiagnostics>,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub fn build_module_from_resolved_program_for_target(
        context: &'c Context,
        program: typecheck::ResolvedProgram<'a>,
        source_map: CodegenSourceMap<'a>,
        target: &CompileTarget,
    ) -> (Option<melior::ir::Module<'c>>, Vec<Diagnostic>) {
        let output = Self::build_module_from_resolved_files_for_target(
            context,
            &[ResolvedFileInput::new(program, source_map)],
            target,
        );
        let diagnostics = output
            .diagnostics
            .into_iter()
            .next()
            .map(|file| file.diagnostics)
            .unwrap_or_default();
        (output.module, diagnostics)
    }

    pub fn build_module_from_resolved_files_for_target(
        context: &'c Context,
        files: &[ResolvedFileInput<'a>],
        target: &CompileTarget,
    ) -> ModuleCodegenOutput<'c> {
        let target_layout = match typecheck::layout::TargetLayout::from_compile_target(target) {
            Ok(layout) => layout,
            Err(error) => {
                return ModuleCodegenOutput {
                    module: None,
                    diagnostics: files
                        .iter()
                        .map(|file| FileCodegenDiagnostics {
                            filename: file.source_map.filename.clone(),
                            // unsupported-target-layout
                            diagnostics: vec![Diagnostic::new(
                                LayoutCode::TargetUnsupported,
                                error.to_string(),
                            )],
                        })
                        .collect(),
                };
            }
        };
        let module_filename = files
            .first()
            .map_or("<module>", |file| file.source_map.filename.as_str());
        let mut module = melior::ir::Module::new(Location::new(context, module_filename, 0, 0));
        if let CompileTarget::Concrete(target) = target {
            module.as_operation_mut().set_attribute(
                "llvm.target_triple",
                StringAttribute::new(context, &target.raw).into(),
            );
        }
        let mut diagnostics = Vec::with_capacity(files.len());
        let mut succeeded = true;
        for (index, file) in files.iter().enumerate() {
            let string_prefix = (files.len() > 1).then(|| format!("{index}_"));
            let ctx = CodegenContext::with_string_prefix(
                context,
                file.program,
                target_layout,
                file.source_map.clone(),
                string_prefix.as_deref().unwrap_or_default(),
            );
            succeeded &=
                ctx.symptoms.count() == 0 && ctx.append_resolved_program_to_module(&module);
            ctx.emit_globals(&module);
            let file_diagnostics = ctx.drain_symptoms();
            succeeded &= file_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw);
            diagnostics.push(FileCodegenDiagnostics {
                filename: file.source_map.filename.clone(),
                diagnostics: file_diagnostics,
            });
        }

        if let Err(diagnostic) = crate::stack::verify_stack_storage(&module) {
            succeeded = false;
            if let Some(file) = diagnostics.first_mut() {
                file.diagnostics.push(diagnostic);
            }
        }

        if succeeded
            && !module.as_operation().verify()
            && std::env::var("CODEGEN_SKIP_VERIFY").is_err()
        {
            succeeded = false;
            if let Some(file) = diagnostics.first_mut() {
                file.diagnostics.push(
                    // codegen-invalid-module
                    Diagnostic::new(
                        CodegenCode::InvalidModule,
                        "generated MLIR module failed verification",
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
            }
        }

        ModuleCodegenOutput {
            module: succeeded.then_some(module),
            diagnostics,
        }
    }

    fn append_resolved_program_to_module(&self, module: &melior::ir::Module<'c>) -> bool {
        use melior::dialect::func;
        use melior::ir::{Block, Region};
        let program = self.program;
        let typed = program;

        for (def_id, bind) in program.defs {
            if bind.definition_stage == typecheck::typed::DefinitionStage::CompileTimeOnly
                || bind.requires_specialization()
            {
                continue;
            }
            let mut symtab = crate::ScopedSymbolTable::new();
            let func_name = self.callable_symbol(def_id);
            let declared_return_ty = program
                .fn_return_types
                .get(def_id)
                .unwrap_or(&bind.return_type);
            let function_return_ty = declared_return_ty;
            self.current_span.set(bind.name_span);
            let loc = self.location();

            let param_types_mlir: Vec<melior::ir::Type<'c>> = bind
                .params
                .iter()
                .enumerate()
                .filter_map(|(i, (_, ty))| {
                    if self.is_empty_product(ty) {
                        None
                    } else {
                        Some(match bind.param_conventions.get(i) {
                            Some(ParamConvention::Observe | ParamConvention::Mutate) => {
                                self.mlir.llvm_ptr()
                            }
                            _ => self.ty_to_mlir(ty),
                        })
                    }
                })
                .collect();
            let param_loc_pairs: Vec<(melior::ir::Type<'c>, Location<'c>)> =
                param_types_mlir.iter().map(|t| (*t, loc)).collect();

            if matches!(bind.body, typecheck::BindBody::Unlowered) {
                self.emit_internal(format!("callable `{func_name}` has no lowered body"));
                return false;
            }

            // Create function body region and entry block.
            let region = Region::new();
            let block = Block::new(&param_loc_pairs);
            region.append_block(block);
            let blk = region.first_block().unwrap();
            self.function_storage
                .replace(Some(frame::FunctionStorage::new(blk)));

            let mut runtime_param_index = 0;
            for (param_index, (name, param_ty)) in bind.params.iter().enumerate() {
                if self.is_empty_product(param_ty) {
                    let val = blk.append_op(
                        self.mlir
                            .llvm_undef(self.ty_to_mlir(param_ty), self.location()),
                    );
                    symtab.insert(*name, val, param_ty.clone(), false);
                    continue;
                }
                let val: melior::ir::Value<'c, 'c> =
                    blk.argument(runtime_param_index).unwrap().into();
                runtime_param_index += 1;
                let borrowed = matches!(
                    bind.param_conventions.get(param_index),
                    Some(ParamConvention::Observe | ParamConvention::Mutate)
                );
                symtab.insert(*name, val, param_ty.clone(), borrowed);
                let binder = ast::BinderId::new(
                    typed.file_id.0,
                    ast::BinderOwner::Parameter {
                        definition: *def_id,
                        slot: param_index as u32,
                    },
                );
                if self.addressed_binders.contains(&binder) {
                    self.materialize_named_place(*name, &blk, &mut symtab);
                }
                if name.as_str() == "self" {
                    let binding = symtab.get(name).unwrap().clone();
                    symtab.insert(def_id.0, binding.value, binding.ty, binding.is_slot);
                }
            }

            // Lower the body expression.
            let result_val = match &bind.body {
                typecheck::BindBody::Expr(eid) => self.lower_function_returns(
                    &[],
                    Some(*eid),
                    function_return_ty,
                    &blk,
                    &mut symtab,
                ),
                typecheck::BindBody::Body { exprs, ret } => {
                    self.lower_function_returns(exprs, *ret, function_return_ty, &blk, &mut symtab)
                }
                typecheck::BindBody::Unlowered => unreachable!(),
            };

            let ret_types = self.result_types_to_mlir(function_return_ty);
            self.function_storage.take();
            let Some(values) = result_val else {
                self.current_span.set(bind.name_span);
                // codegen-return-lowering-failed
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::ReturnLoweringFailed),
                    format!("could not lower the return value of callable `{func_name}`"),
                );
                return false;
            };
            if values.values().len() != ret_types.len()
                || values
                    .values()
                    .iter()
                    .zip(&ret_types)
                    .any(|(value, ty)| value.r#type() != *ty)
            {
                self.current_span.set(bind.name_span);
                // codegen-return-type-mismatch
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::ReturnTypeMismatch),
                    format!("lowered return values do not match the result types of callable `{func_name}`"),
                );
                return false;
            }
            blk.append_operation(func::r#return(values.values(), loc));

            if !self.append_function(
                module,
                &func_name,
                &param_types_mlir,
                &ret_types,
                region,
                None,
                loc,
            ) {
                return false;
            }
        }

        let entries = program
            .defs
            .iter()
            .filter(|(_, bind)| bind.attributes.entry)
            .collect::<Vec<_>>();
        if entries.len() > 1 {
            // codegen-entry
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::Entry),
                "only one callable may declare #entry".to_string(),
            );
            return false;
        }
        if let Some((id, bind)) = entries.first() {
            self.current_span.set(bind.name_span);
            if !bind.params.is_empty() {
                // codegen-entry
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::Entry),
                    "#entry requires a callable without parameters".to_string(),
                );
                return false;
            }
            if program
                .defs
                .iter()
                .any(|(other, candidate)| other.0.as_str() == "main" && !candidate.attributes.entry)
            {
                // codegen-entry
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::Entry),
                    "the #entry host symbol conflicts with an ordinary callable named main"
                        .to_string(),
                );
                return false;
            }
            let ty = program.fn_return_types.get(id).unwrap_or(&bind.return_type);
            if !self.is_empty_product(ty)
                && program
                    .type_registry
                    .integer_validity_for_type(ty)
                    .is_none()
            {
                // codegen-entry
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::Entry),
                    "#entry must return an integer or ()".to_string(),
                );
                return false;
            }
            let loc = self.location();
            let region = Region::new();
            region.append_block(Block::new(&[]));
            let block = region.first_block().unwrap();
            let call = OperationBuilder::new("func.call", loc)
                .add_attributes(&[(
                    Identifier::new(self.mlir, "callee"),
                    self.mlir.symbol_ref_attr(&self.callable_symbol(id)),
                )])
                .add_results(&self.result_types_to_mlir(ty))
                .build()
                .unwrap();
            let call = block.append_operation(call);
            let result = if self.is_empty_product(ty) {
                block.const_i64(self.mlir, 0, loc)
            } else {
                let value = Value::from(call.result(0).unwrap());
                let Some(value) = self.convert_integer_value(value, ty, self.mlir.i64(), &block)
                else {
                    return false;
                };
                value
            };
            block.append_operation(func::r#return(&[result], loc));
            return self.append_function(
                module,
                "main",
                &[],
                &[self.mlir.i64()],
                region,
                None,
                loc,
            );
        }

        true
    }

    fn callable_symbol<'b>(&self, id: &'b typecheck::DefId) -> std::borrow::Cow<'b, str> {
        if self
            .program
            .defs
            .get(id)
            .is_some_and(|bind| bind.attributes.entry)
        {
            std::borrow::Cow::Borrowed("__gin.entry")
        } else if id.1 == 0 {
            std::borrow::Cow::Borrowed(id.0.as_str())
        } else {
            std::borrow::Cow::Owned(format!("{}$def${}", id.0, id.1))
        }
    }

    fn pointee_ty_for_reference_like(&self, ty: &Ty) -> Option<Ty> {
        let definition = self.program.type_registry.resolved_definition_for_type(ty);
        if let Some(pointee) = definition.pointee_ty() {
            return Some(pointee.clone());
        }
        self.reference_pointee_ty(ty)
    }

    #[allow(clippy::too_many_arguments)]
    fn append_function(
        &self,
        module: &melior::ir::Module<'c>,
        name: &str,
        parameters: &[melior::ir::Type<'c>],
        results: &[melior::ir::Type<'c>],
        region: Region<'c>,
        visibility: Option<&str>,
        location: Location<'c>,
    ) -> bool {
        let function_type = melior::ir::r#type::FunctionType::new(self.mlir, parameters, results);
        let mut attributes = vec![
            (
                Identifier::new(self.mlir, "sym_name"),
                StringAttribute::new(self.mlir, name).into(),
            ),
            (
                Identifier::new(self.mlir, "function_type"),
                TypeAttribute::new(function_type.into()).into(),
            ),
        ];
        if let Some(visibility) = visibility {
            attributes.push((
                Identifier::new(self.mlir, "sym_visibility"),
                StringAttribute::new(self.mlir, visibility).into(),
            ));
        }
        let builder = OperationBuilder::new("func.func", location).add_attributes(&attributes);
        match builder.add_regions([region]).build() {
            Ok(function) => {
                let _ = module.body().append_operation(function);
                true
            }
            Err(error) => {
                self.emit_internal(format!("failed to build function `{name}`: {error}"));
                false
            }
        }
    }

    fn expect_arity<const N: usize, T: Copy>(&self, values: &[T], op_name: &str) -> Option<[T; N]> {
        let Ok(items) = <[T; N]>::try_from(values) else {
            self.emit_internal(format!("invalid typed {op_name} arity"));
            return None;
        };
        Some(items)
    }

    fn pack_sequence(
        &self,
        values: LoweredValue<'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let aggregate = self.ty_to_mlir(ty);
        if let [value] = values.values()
            && value.r#type() == aggregate
        {
            return Some(*value);
        }
        self.insert_aggregate(block, aggregate, values.into_values().into_iter().map(Some))
    }

    fn value_from_lowered(
        &self,
        values: LoweredValue<'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        if self.is_empty_product(ty) {
            Some(block.unit_value(self, self.location()))
        } else if matches!(ty, Ty::Sequence(_)) {
            self.pack_sequence(values, ty, block)
        } else {
            values.single()
        }
    }

    fn unpack_sequence(
        &self,
        value: Value<'c, 'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<LoweredValue<'c>> {
        let Ty::Sequence(fields) = ty else {
            return Some(LoweredValue::scalar(value));
        };
        let mut values = Vec::with_capacity(fields.len());
        for (index, field) in fields.iter().enumerate() {
            values.push(block.append_op(self.mlir.llvm_extractvalue(
                value,
                i64::try_from(index).ok()?,
                self.ty_to_mlir(&field.ty),
                self.location(),
            )));
        }
        Some(LoweredValue::new(values))
    }

    fn require_call_target(
        &self,
        target: &typecheck::DefId,
        args: Option<&[typecheck::ExprId]>,
        symtab: &ScopedSymbolTable<'c>,
    ) -> Option<()> {
        if self.program.defs.contains_key(target)
            || self.program.fn_return_types.contains_key(target)
            || (args.is_none_or(<[_]>::is_empty) && symtab.get(&target.0).is_some())
        {
            return Some(());
        }
        self.emit_internal(format!("unresolved call target `{}`", target.0));
        None
    }

    fn call_return_type<'s>(
        &'s self,
        target: &typecheck::DefId,
        substituted_ty: Option<&'s Ty>,
        fallback: &'s Ty,
    ) -> &'s Ty {
        let return_ty = self
            .program
            .fn_return_types
            .get(target)
            .or_else(|| self.program.defs.get(target).map(|bind| &bind.return_type))
            .unwrap_or(fallback);
        substituted_ty.unwrap_or(return_ty)
    }

    fn lower_call_arguments(
        &self,
        target: &typecheck::DefId,
        args: Option<&[typecheck::ExprId]>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Vec<Value<'c, 'c>>> {
        let signature = self.program.defs.get(target);
        let mut lowered = Vec::new();
        for (index, arg) in args.unwrap_or_default().iter().enumerate() {
            let source = self.program.expr(*arg)?.ty;
            let convention = signature.and_then(|bind| bind.param_conventions.get(index));
            let value = match convention {
                Some(ParamConvention::Observe | ParamConvention::Mutate) => {
                    self.lower_typed_place(*arg, block, symtab)?
                }
                _ => {
                    let value = self.lower_typed_expr(*arg, block, symtab)?;
                    if let Some((_, parameter)) = signature.and_then(|bind| bind.params.get(index))
                        && IntegerType::try_from(value.r#type()).is_ok()
                        && let target = self.ty_to_mlir(parameter)
                        && IntegerType::try_from(target).is_ok()
                    {
                        self.convert_integer_value(value, source, target, block)?
                    } else {
                        value
                    }
                }
            };
            if !self.is_empty_product(source) {
                lowered.push(value);
            }
        }
        Some(lowered)
    }

    pub fn lower_typed_values(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        if let Some(values) = self.operand_values.borrow().get(&expr_id).cloned() {
            return Some(values);
        }
        if self.has_returning_operands(expr_id) {
            return self.lower_returning_operands(expr_id, block, symtab);
        }
        if self.program.is_local_callable(expr_id) {
            return Some(LoweredValue::new(Vec::new()));
        }
        if let Some(definition) = self.program.local_call(expr_id) {
            return self.lower_local_callable(expr_id, definition, block, symtab);
        }
        if let Some(value) = self.when_subjects.borrow().get(&expr_id).copied() {
            return Some(LoweredValue::new(vec![value]));
        }
        if self.emission_construction.contains(&expr_id) {
            return Some(LoweredValue::new(Vec::new()));
        }
        let expr = self.program.expr(expr_id)?;
        self.current_span.set(*expr.span);
        if let typecheck::TypedExprKind::Emit {
            state: typecheck::assembly::AssemblyState::Checked(assembly),
            ..
        } = &expr.kind
        {
            return self.lower_checked_assembly(assembly, expr.ty, block, symtab);
        }
        if let typecheck::TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
        } = &expr.kind
        {
            self.require_call_target(target, args.as_deref(), symtab)?;
            let return_ty = self.call_return_type(target, substituted_ty.as_ref(), expr.ty);
            if matches!(return_ty, Ty::Sequence(_)) {
                if args.as_ref().is_none_or(Vec::is_empty)
                    && let Some(binding) = symtab.get(&target.0).cloned()
                {
                    let value =
                        self.slot_binding_value(&binding, &binding.ty, block, self.location())?;
                    return self.unpack_sequence(value, return_ty, block);
                }
                let lowered_args =
                    self.lower_call_arguments(target, args.as_deref(), block, symtab)?;
                let operation = OperationBuilder::new("func.call", self.location())
                    .add_attributes(&[(
                        Identifier::new(self.mlir, "callee"),
                        self.mlir.symbol_ref_attr(&self.callable_symbol(target)),
                    )])
                    .add_operands(&lowered_args)
                    .add_results(&self.result_types_to_mlir(return_ty))
                    .build()
                    .map_err(|error| self.emit_internal(format!("result call lowering: {error}")))
                    .ok()?;
                let operation = block.append_operation(operation);
                return Some(LoweredValue::new(
                    operation.results().map(Value::from).collect(),
                ));
            }
            if matches!(expr.ty, Ty::Sequence(_)) {
                self.emit_internal("sequence call has a scalar return signature");
                return None;
            }
        }
        match &expr.kind {
            typecheck::TypedExprKind::Block { statements, value } => {
                symtab.push_scope();
                let values = self.lower_block_values(statements, *value, expr.ty, block, symtab);
                symtab.pop_scope();
                return values;
            }
            typecheck::TypedExprKind::Return { value, .. } => {
                self.store_function_return(*value, block, symtab)?;
                return Some(LoweredValue::new(Vec::new()));
            }
            _ => {}
        }
        match &expr.kind {
            typecheck::TypedExprKind::ResultRow(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.lower_typed_expr(*item, block, symtab)?);
                }
                Some(LoweredValue::new(values))
            }
            typecheck::TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned: false,
            } if matches!(expr.ty, Ty::Sequence(_)) => {
                let value = self.lower_block_values(stmts, Some(*body), expr.ty, block, symtab)?;
                let aggregate = self.pack_sequence(value.clone(), expr.ty, block)?;
                symtab.insert(*name, aggregate, expr.ty.clone(), false);
                if self.return_storage.borrow().is_some() {
                    self.materialize_named_place(*name, block, symtab)?;
                }
                Some(value)
            }
            typecheck::TypedExprKind::Reassign { name, value, .. }
                if matches!(expr.ty, Ty::Sequence(_)) =>
            {
                let value = self.lower_typed_values(*value, block, symtab)?;
                let aggregate = self.pack_sequence(value.clone(), expr.ty, block)?;
                let binding = symtab.get(name)?.clone();
                if binding.is_slot {
                    block.store_typed(self, binding.value, aggregate, self.location())?;
                } else {
                    symtab.assign(name, crate::Slot::value(aggregate, expr.ty.clone()));
                }
                Some(value)
            }
            typecheck::TypedExprKind::SelfRef { target } if matches!(expr.ty, Ty::Sequence(_)) => {
                let binding = symtab.get(&target.0)?.clone();
                let value =
                    self.slot_binding_value(&binding, &binding.ty, block, self.location())?;
                self.unpack_sequence(value, expr.ty, block)
            }
            typecheck::TypedExprKind::If(if_expr)
                if matches!(expr.ty, Ty::Sequence(_)) || self.is_empty_product(expr.ty) =>
            {
                self.lower_typed_if_values(if_expr, expr.ty, block, symtab)
            }
            typecheck::TypedExprKind::When(when_expr)
                if matches!(expr.ty, Ty::Sequence(_)) || self.is_empty_product(expr.ty) =>
            {
                self.lower_typed_when_values(when_expr, expr.ty, block, symtab)
            }
            _ if self.is_empty_product(expr.ty) => {
                self.lower_typed_expr(expr_id, block, symtab)?;
                Some(LoweredValue::new(Vec::new()))
            }
            _ => {
                let value = self.lower_typed_expr(expr_id, block, symtab)?;
                self.unpack_sequence(value, expr.ty, block)
            }
        }
    }

    pub fn lower_typed_expr(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let cached = self.operand_values.borrow().get(&expr_id).cloned();
        if cached.is_some() || self.has_returning_operands(expr_id) {
            let values = match cached {
                Some(values) => values,
                None => self.lower_returning_operands(expr_id, block, symtab)?,
            };
            let ty = self.program.expr(expr_id)?.ty;
            return self.value_from_lowered(values, ty, block);
        }
        if self.program.is_local_callable(expr_id) {
            return Some(block.unit_value(self, self.location()));
        }
        if self.program.local_call(expr_id).is_some() {
            let value = self.lower_typed_values(expr_id, block, symtab)?;
            let ty = self.program.expr(expr_id)?.ty;
            return self.value_from_lowered(value, ty, block);
        }
        if let Some(value) = self.when_subjects.borrow().get(&expr_id).copied() {
            return Some(value);
        }
        if self.emission_construction.contains(&expr_id) {
            return Some(block.unit_value(self, self.location()));
        }
        let typed_ast = self.program;
        let expr_ref = typed_ast.expr(expr_id)?;
        if matches!(expr_ref.ty, Ty::Sequence(_))
            && matches!(
                expr_ref.kind,
                typecheck::TypedExprKind::FnCall { .. }
                    | typecheck::TypedExprKind::Bind {
                        unassigned: false,
                        ..
                    }
                    | typecheck::TypedExprKind::SelfRef { .. }
                    | typecheck::TypedExprKind::Reassign { .. }
                    | typecheck::TypedExprKind::If(_)
                    | typecheck::TypedExprKind::When(_)
                    | typecheck::TypedExprKind::Emit {
                        state: typecheck::assembly::AssemblyState::Checked(_),
                        ..
                    }
            )
        {
            let values = self.lower_typed_values(expr_id, block, symtab)?;
            return self.pack_sequence(values, expr_ref.ty, block);
        }
        if matches!(
            expr_ref.kind,
            typecheck::TypedExprKind::Block { .. } | typecheck::TypedExprKind::Return { .. }
        ) {
            let values = self.lower_typed_values(expr_id, block, symtab)?;
            return self.value_from_lowered(values, expr_ref.ty, block);
        }
        // Propagate source location from the original expression span.
        self.current_span.set(*expr_ref.span);

        if matches!(expr_ref.ty, ast::Ty::UnresolvedLiteral(_)) {
            match expr_ref.ty {
                // type-unresolved-text-literal-codegen
                ast::Ty::UnresolvedLiteral(ast::ty::LiteralKind::Text) => self.emit_symptom(
                    DiagnosticCode::Type(TypeCode::Text(TextCode::UnresolvedAtCodegen)),
                    "unresolved text literal reached code generation".to_string(),
                ),
                _ => self.emit_internal("unresolved integer literal reached code generation"),
            }
            return None;
        }
        if matches!(expr_ref.ty, ast::Ty::UnresolvedSequence { .. }) {
            // type-unresolved-sequence-codegen
            self.emit_symptom(
                DiagnosticCode::Type(TypeCode::Sequence(SequenceCode::UnresolvedAtCodegen)),
                "unresolved sequence literal reached code generation".to_string(),
            );
            return None;
        }
        if expr_ref.ty.is_unresolved_address() {
            // type-unresolved-address-codegen
            self.emit_symptom(
                DiagnosticCode::Type(TypeCode::Address(AddressCode::UnresolvedAtCodegen)),
                "unresolved raw address reached code generation".to_string(),
            );
            return None;
        }

        match expr_ref.kind {
            typecheck::TypedExprKind::Block { .. } | typecheck::TypedExprKind::Return { .. } => {
                unreachable!()
            }
            typecheck::TypedExprKind::Domain(_) => {
                // codegen-domain-value
                self.emit_symptom(DiagnosticCode::Codegen(CodegenCode::DomainValue), "domains are compile-time values; interpret them through a library range before runtime use".to_string());
                None
            }
            typecheck::TypedExprKind::Invalid => None,
            typecheck::TypedExprKind::Todo => {
                // unfilled-hole
                self.emit_symptom(
                    DiagnosticCode::UnfilledHole,
                    "cannot compile an unfilled #todo hole".into(),
                );
                None
            }
            typecheck::TypedExprKind::Target => {
                // codegen-target-query
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::TargetQuery),
                    "#target is compile-time-only".to_string(),
                );
                None
            }
            typecheck::TypedExprKind::TargetField(_) => None,
            typecheck::TypedExprKind::Emit {
                specification,
                operands: _,
                state,
                ..
            } => {
                if let typecheck::assembly::AssemblyState::Checked(assembly) = state {
                    let values =
                        self.lower_checked_assembly(assembly, expr_ref.ty, block, symtab)?;
                    if self.is_empty_product(expr_ref.ty) {
                        Some(block.unit_value(self, self.location()))
                    } else {
                        values.single()
                    }
                } else {
                    let _ = specification;
                    // codegen-assembly-not-checked
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::AssemblyNotChecked),
                        "pending or invalid assembly reached code generation".to_string(),
                    );
                    None
                }
            }
            typecheck::TypedExprKind::Lit(lit) => {
                if self
                    .program
                    .type_registry
                    .resolved_definition_for_type(expr_ref.ty)
                    .union_literal_values()
                    .is_some()
                {
                    let Some(value) = expr_ref.facts.const_facts.const_value.as_ref() else {
                        self.emit_internal("materialized literal union has no constant value");
                        return None;
                    };
                    return self.lower_literal_union_value(value, expr_ref.ty, block);
                }
                match lit {
                    ast::Literal::Int(_) => {
                        let Some(value @ ast::ConstValue::Int(_)) =
                            expr_ref.facts.const_facts.const_value.as_ref()
                        else {
                            self.emit_internal(
                                "materialized integer literal has no integer constant value",
                            );
                            return None;
                        };
                        self.lower_const_value(block, value, expr_ref.ty)
                    }
                    ast::Literal::Float(HashFloat(value)) => {
                        self.lower_float_constant(block, *value, expr_ref.ty)
                    }
                    ast::Literal::String(text) => {
                        self.lower_text_constant(block, text, expr_ref.ty)
                    }
                }
            }
            typecheck::TypedExprKind::Rematerialize { constant, .. } => {
                let value = match constant {
                    ast::ConstValue::Proposition(holds) => i128::from(*holds),
                    ast::ConstValue::Int(value) => integer_constant_bits(*value)?,
                    ast::ConstValue::ResultAlternative { label, .. } => {
                        let ast::Ty::ResultFamily { alternatives, .. } = expr_ref.ty else {
                            self.emit_internal("result rematerialization has a non-result type");
                            return None;
                        };
                        i128::try_from(
                            alternatives
                                .iter()
                                .position(|alternative| alternative.label == *label)?,
                        )
                        .ok()?
                    }
                    _ => {
                        self.emit_internal("rematerialization proof is not an anonymous atom");
                        return None;
                    }
                };
                Some(block.const_int(
                    self.mlir,
                    self.ty_to_mlir(expr_ref.ty),
                    value,
                    self.location(),
                ))
            }
            typecheck::TypedExprKind::TargetQuery { kind, operand } => {
                let value =
                    match self
                        .target_layout
                        .query(*kind, operand, Some(self.program.type_registry))
                    {
                        Ok(value) => value,
                        Err(error) => {
                            self.emit_symptom(error.code(), error.to_string());
                            return None;
                        }
                    };
                let exact = i256::I256::from(value);
                let width = ast::integer::FiniteHull::new(exact, exact)
                    .map(|hull| hull.inferred_representation().width().get())
                    .unwrap_or(1);
                Some(block.const_int(
                    self.mlir,
                    IntegerType::new(self.mlir, width).into(),
                    i128::from(value),
                    self.location(),
                ))
            }
            typecheck::TypedExprKind::FnCall {
                target,
                args,
                substituted_ty,
                ..
            } => {
                self.require_call_target(target, args.as_deref(), symtab)?;
                let lowered_args =
                    self.lower_call_arguments(target, args.as_deref(), block, symtab)?;

                let loc = self.location();

                let fn_name = self.callable_symbol(target);

                if (args.is_none() || args.as_ref().is_none_or(|a| a.is_empty()))
                    && let Some(slot) = symtab.get(&target.0)
                {
                    return self.lower_slot_value(slot, expr_ref.ty, block);
                }

                let return_ty = self.call_return_type(target, substituted_ty.as_ref(), expr_ref.ty);

                if self.is_empty_product(return_ty) {
                    block.call_unit(self.mlir, &fn_name, &lowered_args, loc);
                    return Some(block.unit_value(self, loc));
                }

                let return_ty = self.ty_to_mlir(return_ty);
                Some(block.call(self.mlir, &fn_name, &lowered_args, return_ty, loc))
            }
            typecheck::TypedExprKind::IntrinsicCall { op, args } => {
                let mut lowered: Vec<_> = args
                    .iter()
                    .map(|arg| self.lower_typed_expr(*arg, block, symtab))
                    .collect::<Option<_>>()?;
                match op {
                    typecheck::typed::IntrinsicOp::AddressLoad => {
                        let [address] = self.expect_arity(lowered.as_slice(), op.name())?;
                        let address_ty = typed_ast.expr(*args.first()?)?.ty;
                        return block.load_typed(
                            self,
                            address,
                            self.ty_to_mlir(&self.pointee_ty_for_reference_like(address_ty)?),
                            self.location(),
                        );
                    }
                    typecheck::typed::IntrinsicOp::AddressStore => {
                        let [address, value] = self.expect_arity(lowered.as_slice(), op.name())?;
                        block.store_typed(self, address, value, self.location())?;
                        return Some(block.unit_value(self, self.location()));
                    }
                    typecheck::typed::IntrinsicOp::AddressOffset => {
                        let [address, index] = self.expect_arity(lowered.as_slice(), op.name())?;
                        let address_ty = typed_ast.expr(*args.first()?)?.ty;
                        let pointee = self.pointee_ty_for_reference_like(address_ty)?;
                        if !self.validate_pointee_stride(&pointee) {
                            return None;
                        }
                        let index = self.convert_integer_value(
                            index,
                            typed_ast.expr(args[1])?.ty,
                            IntegerType::new(self.mlir, self.target_layout.index_bits()).into(),
                            block,
                        )?;
                        return block.gep_typed_result(
                            self,
                            address,
                            self.ty_to_mlir(&pointee),
                            &[index],
                            &[i32::MIN],
                            self.ty_to_mlir(expr_ref.ty),
                            self.location(),
                        );
                    }
                    typecheck::typed::IntrinsicOp::AddressToBits => {
                        let [address] = self.expect_arity(lowered.as_slice(), op.name())?;
                        let operation = OperationBuilder::new("llvm.ptrtoint", self.location())
                            .add_operands(&[address])
                            .add_results(&[self.ty_to_mlir(expr_ref.ty)])
                            .build()
                            .map_err(|error| self.emit_internal(format!("ptrtoint: {error}")))
                            .ok()?;
                        return Some(block.append_op(operation));
                    }
                    typecheck::typed::IntrinsicOp::BitsToAddress => {
                        let [bits] = self.expect_arity(lowered.as_slice(), op.name())?;
                        let operation = OperationBuilder::new("llvm.inttoptr", self.location())
                            .add_operands(&[bits])
                            .add_results(&[self.ty_to_mlir(expr_ref.ty)])
                            .build()
                            .map_err(|error| self.emit_internal(format!("inttoptr: {error}")))
                            .ok()?;
                        return Some(block.append_op(operation));
                    }
                    _ => {}
                }
                if let typecheck::typed::IntrinsicOp::BitsCompare { predicate, .. } = op {
                    let [lhs, rhs] = self.expect_arity(lowered.as_slice(), op.name())?;
                    let relation = match predicate {
                        typecheck::intrinsic::ComparisonPredicate::Equal => {
                            ast::ProofRelation::Equal
                        }
                        typecheck::intrinsic::ComparisonPredicate::Less => ast::ProofRelation::Less,
                        typecheck::intrinsic::ComparisonPredicate::LessOrEqual => {
                            ast::ProofRelation::LessOrEqual
                        }
                        typecheck::intrinsic::ComparisonPredicate::Greater => {
                            ast::ProofRelation::Greater
                        }
                        typecheck::intrinsic::ComparisonPredicate::GreaterOrEqual => {
                            ast::ProofRelation::GreaterOrEqual
                        }
                    };
                    let compared = self.compare_integer_values(
                        [args[0], args[1]],
                        [lhs, rhs],
                        relation,
                        block,
                    )?;
                    let result_ty = self.ty_to_mlir(expr_ref.ty);
                    if result_ty == self.mlir.i1() {
                        return Some(compared);
                    }
                    let extended = OperationBuilder::new("arith.extui", self.location())
                        .add_operands(&[compared])
                        .add_results(&[result_ty])
                        .build()
                        .map_err(|error| {
                            self.emit_internal(format!("comparison result lowering: {error}"))
                        })
                        .ok()?;
                    return Some(block.append_op(extended));
                }
                if matches!(
                    op,
                    typecheck::typed::IntrinsicOp::BitsShiftLeft
                        | typecheck::typed::IntrinsicOp::BitsShiftRightLogical
                        | typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic
                ) && args.iter().all(|arg| {
                    typed_ast
                        .expr(*arg)
                        .is_some_and(|expr| expr.ty.anonymous_integer_validity().is_some())
                }) {
                    return self.lower_integer_shift(
                        *op,
                        self.expect_arity(args.as_slice(), op.name())?,
                        self.expect_arity(lowered.as_slice(), op.name())?,
                        expr_ref.ty,
                        block,
                    );
                }
                if matches!(
                    op,
                    typecheck::typed::IntrinsicOp::BitsSignedDivide
                        | typecheck::typed::IntrinsicOp::BitsUnsignedDivide
                        | typecheck::typed::IntrinsicOp::BitsSignedRemainder
                        | typecheck::typed::IntrinsicOp::BitsUnsignedRemainder
                ) {
                    return self.lower_integer_division(
                        *op,
                        self.expect_arity(args.as_slice(), op.name())?,
                        self.expect_arity(lowered.as_slice(), op.name())?,
                        expr_ref.ty,
                        block,
                    );
                }
                let operation = match intrinsic_arith_name(op) {
                    Some(operation) => operation,
                    None => match op {
                        typecheck::typed::IntrinsicOp::BitsExtendSigned => "arith.extsi",
                        typecheck::typed::IntrinsicOp::BitsExtendUnsigned => "arith.extui",
                        typecheck::typed::IntrinsicOp::BitsTruncate => "arith.trunci",
                        _ => unreachable!(),
                    },
                };
                let result_ty = self.ty_to_mlir(expr_ref.ty);
                if matches!(
                    op,
                    typecheck::typed::IntrinsicOp::BitsExtendSigned
                        | typecheck::typed::IntrinsicOp::BitsExtendUnsigned
                        | typecheck::typed::IntrinsicOp::BitsTruncate
                ) && lowered.len() == 1
                    && lowered[0].r#type() == result_ty
                {
                    return Some(lowered[0]);
                }
                match op.contract() {
                    typecheck::intrinsic::IntrinsicContract::SameWidthBinary
                    | typecheck::intrinsic::IntrinsicContract::Comparison => {
                        self.expect_arity::<2, _>(lowered.as_slice(), op.name())?;
                    }
                    typecheck::intrinsic::IntrinsicContract::Extension
                    | typecheck::intrinsic::IntrinsicContract::Truncation => {
                        self.expect_arity::<1, _>(lowered.as_slice(), op.name())?;
                    }
                    typecheck::intrinsic::IntrinsicContract::AddressLoad
                    | typecheck::intrinsic::IntrinsicContract::AddressStore
                    | typecheck::intrinsic::IntrinsicContract::AddressOffset
                    | typecheck::intrinsic::IntrinsicContract::AddressConversion => unreachable!(),
                }
                if matches!(
                    op.contract(),
                    typecheck::intrinsic::IntrinsicContract::SameWidthBinary
                ) && lowered.len() == 2
                {
                    for (index, arg) in args.iter().enumerate().take(2) {
                        lowered[index] = self.convert_integer_value(
                            lowered[index],
                            typed_ast.expr(*arg)?.ty,
                            result_ty,
                            block,
                        )?;
                    }
                }
                if matches!(
                    op,
                    typecheck::typed::IntrinsicOp::BitsShiftLeft
                        | typecheck::typed::IntrinsicOp::BitsShiftRightLogical
                        | typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic
                ) {
                    let [_, count] = self.expect_arity(lowered.as_slice(), op.name())?;
                    let width = expr_ref.ty.anonymous_integer_width().unwrap_or_default();
                    let width_value =
                        block.const_int(self.mlir, result_ty, i128::from(width), self.location());
                    let reduced = OperationBuilder::new("arith.remui", self.location())
                        .add_operands(&[count, width_value])
                        .add_results(&[result_ty])
                        .build()
                        .map_err(|error| {
                            self.emit_internal(format!("shift count lowering: {error}"))
                        })
                        .ok()?;
                    lowered[1] = block.append_op(reduced);
                }
                let operation = OperationBuilder::new(operation, self.location())
                    .add_operands(&lowered)
                    .add_results(&[result_ty])
                    .build()
                    .map_err(|error| self.emit_internal(format!("intrinsic lowering: {error}")))
                    .ok()?;
                Some(block.append_op(operation))
            }
            typecheck::TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                if *unassigned {
                    for stmt_id in stmts {
                        self.lower_typed_expr(*stmt_id, block, symtab)?;
                    }
                    // Declaration without value: allocate stack space but don't initialize.
                    // Use the declared type to determine the alloca size.
                    let mlir_ty = self.ty_to_mlir(expr_ref.ty);
                    let slot = self.allocate_stack_slot(mlir_ty)?;
                    symtab.insert(*name, slot, expr_ref.ty.clone(), true);
                    Some(slot)
                } else {
                    let values =
                        self.lower_block_values(stmts, Some(*body), expr_ref.ty, block, symtab)?;
                    let val = if self.is_empty_product(expr_ref.ty) {
                        block.unit_value(self, self.location())
                    } else {
                        values.single()?
                    };
                    symtab.insert(*name, val, expr_ref.ty.clone(), false);
                    if self.return_storage.borrow().is_some()
                        || typed_ast
                            .exprs
                            .place_of(expr_id)
                            .flatten()
                            .is_some_and(|place| {
                                self.addressed_binders
                                    .contains(&typed_ast.places[place.0 as usize].binder)
                            })
                    {
                        self.materialize_named_place(*name, block, symtab)?;
                    }
                    Some(val)
                }
            }
            typecheck::TypedExprKind::Reassign { name, value, .. } => {
                let val = self.lower_typed_expr(*value, block, symtab)?;
                if let Some(slot) = symtab.get(name).cloned() {
                    let stored_ty = if slot.is_slot {
                        typecheck::ty::reference_pointee_for_type(
                            &slot.ty,
                            Some(self.program.type_registry),
                        )
                        .unwrap_or_else(|| slot.ty.clone())
                    } else {
                        slot.ty.clone()
                    };
                    let val = self.convert_integer_value(
                        val,
                        typed_ast.expr(*value)?.ty,
                        self.ty_to_mlir(&stored_ty),
                        block,
                    )?;
                    if slot.is_slot {
                        block.store_typed(self, slot.value, val, self.location())?;
                    } else {
                        symtab.assign(name, crate::Slot::value(val, slot.ty));
                    }
                    Some(val)
                } else {
                    self.emit_internal(format!(
                        "cannot reassign '{}': not found in symtab",
                        name.as_str()
                    ));
                    None
                }
            }
            typecheck::TypedExprKind::Binary { op, .. } => {
                let symbol: &'static str = op.into();
                self.emit_internal(format!(
                    "unresolved binary operator `{}` reached code generation",
                    symbol
                ));
                None
            }
            typecheck::TypedExprKind::InvalidOperator { op, .. } => {
                self.emit_internal(format!(
                    "invalid operator registration for `{}` reached code generation",
                    op.symbol()
                ));
                None
            }
            typecheck::TypedExprKind::TagCall {
                variant_id,
                discriminant,
                args,
                field_names,
            } => {
                let constructed_ty = expr_ref.ty;
                if let Ok(typecheck::representation::Repr::Array { length, .. }) =
                    typecheck::representation::Repr::derive(
                        constructed_ty,
                        Some(typed_ast.type_registry),
                    )
                {
                    let [init] = args.as_deref().unwrap_or_default() else {
                        // codegen-array-constructor-arity
                        self.emit_symptom(
                            DiagnosticCode::Codegen(CodegenCode::ArrayConstructorArity),
                            "fixed-array constructor requires exactly one value argument".into(),
                        );
                        return None;
                    };
                    return self.lower_repeated_array(*init, length, constructed_ty, block, symtab);
                }
                if let Ty::ResultFamily { alternatives, .. } = constructed_ty {
                    let Some(discriminant) = alternatives
                        .iter()
                        .position(|alternative| alternative.label == variant_id.name)
                    else {
                        self.emit_internal(
                            "constructed alternative is absent from its result family",
                        );
                        return None;
                    };
                    if let Some(args) = args {
                        for argument in args {
                            self.lower_typed_expr(*argument, block, symtab)?;
                        }
                    }
                    return Some(block.const_int(
                        self.mlir,
                        self.ty_to_mlir(constructed_ty),
                        discriminant as i128,
                        self.location(),
                    ));
                }
                let resolved_constructed = self
                    .program
                    .type_registry
                    .resolved_definition_for_type(constructed_ty);
                let is_payload_free_union = {
                    resolved_constructed.union_literal_values().is_some()
                        || matches!(
                            &resolved_constructed,
                            Ty::Union { variants, .. }
                                if variants.iter().all(|variant| variant.fields.is_empty())
                        )
                };
                if args.as_ref().is_some_and(|args| args.is_empty())
                    && self
                        .program
                        .type_registry
                        .integer_validity_for_type(constructed_ty)
                        .is_some()
                {
                    return Some(block.const_int(
                        self.mlir,
                        self.ty_to_mlir(constructed_ty),
                        0,
                        self.location(),
                    ));
                }

                let (fields, names) = match &resolved_constructed {
                    Ty::Record { fields, .. } => (fields.as_slice(), field_names.as_slice()),
                    Ty::Union { variants, .. } => {
                        let Some(variant) = variants
                            .get(*discriminant)
                            .filter(|variant| variant.name == variant_id.name)
                        else {
                            // codegen-constructor-invalid
                            self.emit_symptom(
                                DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                                format!("constructor variant `{}` does not match discriminant {} in `{}`", variant_id.name, discriminant, constructed_ty.format_for_hover()),
                            );
                            return None;
                        };
                        (variant.fields.as_slice(), &[][..])
                    }
                    _ => {
                        // sum-representation-missing
                        self.emit_symptom(DiagnosticCode::Representation(RepresentationCode::SumMissing), "non-record tag construction reached codegen without a tagged-sum representation".into());
                        return None;
                    }
                };
                let lowered_args = self.lower_constructor_arguments(
                    args.as_deref().unwrap_or_default(),
                    names,
                    fields,
                    block,
                    symtab,
                )?;
                if is_payload_free_union {
                    let result_ty = self.ty_to_mlir(constructed_ty);
                    let value = match &resolved_constructed {
                        Ty::Union { variants, .. } => variants
                            .get(*discriminant)?
                            .discriminant_value(*discriminant),
                        _ => return None,
                    };
                    return Some(block.const_int(
                        self.mlir,
                        result_ty,
                        integer_constant_bits(value)?,
                        self.location(),
                    ));
                }
                if matches!(&resolved_constructed, Ty::Union { .. }) {
                    return self.lower_sum_constructor(
                        block,
                        constructed_ty,
                        *discriminant,
                        &lowered_args,
                    );
                }
                let struct_ty = self.ty_to_mlir(constructed_ty);
                self.insert_aggregate(block, struct_ty, lowered_args.into_iter().map(Some))
            }
            typecheck::TypedExprKind::SelfRef { target } => {
                let slot = symtab.get(&target.0)?;
                self.lower_slot_value(slot, expr_ref.ty, block)
            }
            typecheck::TypedExprKind::Range { start, end } => {
                let _s = self.lower_typed_expr(*start, block, symtab)?;
                let _e = self.lower_typed_expr(*end, block, symtab)?;
                None // Range lowering not yet implemented
            }
            typecheck::TypedExprKind::RecordValue(items)
            | typecheck::TypedExprKind::ResultRow(items) => {
                let vals = items
                    .iter()
                    .map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect::<Option<Vec<_>>>()?;

                let aggregate_ty = self.ty_to_mlir(expr_ref.ty);
                self.insert_aggregate(block, aggregate_ty, vals.into_iter().map(Some))
            }
            typecheck::TypedExprKind::ArrayLit(items) => {
                let repr = typecheck::representation::Repr::derive(
                    expr_ref.ty,
                    Some(typed_ast.type_registry),
                )
                .ok();
                let Some(typecheck::representation::Repr::Array { length, .. }) = repr else {
                    // unresolved-fixed-array-construction
                    self.emit_symptom(
                        DiagnosticCode::Representation(
                            RepresentationCode::FixedArrayConstructionUnresolved,
                        ),
                        "fixed-array literal reached codegen without array representation"
                            .to_string(),
                    );
                    return None;
                };
                self.lower_fixed_array_elements(items, length, expr_ref.ty, block, symtab)
            }
            typecheck::TypedExprKind::UnionPayload { expr, variant } => {
                let value = self.lower_typed_expr(*expr, block, symtab)?;
                self.lower_sum_payload(
                    block,
                    value,
                    typed_ast.expr(*expr)?.ty,
                    *variant,
                    0,
                    expr_ref.ty,
                )
            }
            typecheck::TypedExprKind::Cast { expr, .. } => {
                if self
                    .program
                    .type_registry
                    .resolved_definition_for_type(expr_ref.ty)
                    .union_literal_values()
                    .is_some()
                    && let source = self.program.expr(*expr)?
                    && let Some(constant) = source.facts.const_facts.const_value.as_ref()
                {
                    if !matches!(source.kind, typecheck::TypedExprKind::Lit(_)) {
                        self.lower_typed_expr(*expr, block, symtab)?;
                    }
                    return self.lower_literal_union_value(constant, expr_ref.ty, block);
                }
                let value = self.lower_typed_expr(*expr, block, symtab)?;
                let result_ty = self.ty_to_mlir(expr_ref.ty);
                if value.r#type() == self.mlir.llvm_ptr() && result_ty != value.r#type() {
                    let op = OperationBuilder::new("llvm.ptrtoint", self.location())
                        .add_operands(&[value])
                        .add_results(&[result_ty])
                        .build()
                        .map_err(|error| self.emit_internal(format!("ptrtoint: {error}")))
                        .ok()?;
                    Some(block.append_op(op))
                } else if result_ty == self.mlir.llvm_ptr() && result_ty != value.r#type() {
                    let op = OperationBuilder::new("llvm.inttoptr", self.location())
                        .add_operands(&[value])
                        .add_results(&[result_ty])
                        .build()
                        .map_err(|error| self.emit_internal(format!("inttoptr: {error}")))
                        .ok()?;
                    Some(block.append_op(op))
                } else if result_ty != value.r#type() {
                    self.convert_integer_value(value, typed_ast.expr(*expr)?.ty, result_ty, block)
                } else {
                    Some(value)
                }
            }
            typecheck::TypedExprKind::Negate(init) => {
                let operand_ty = typed_ast.expr(*init)?.ty;
                let integer = typed_ast
                    .type_registry
                    .integer_validity_for_type(operand_ty)
                    .is_some();
                let floating = matches!(operand_ty, Ty::Float { .. });
                if !integer && !floating {
                    // codegen-negation-type-mismatch
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::NegationTypeMismatch),
                        "negation requires an integer or floating-point operand".into(),
                    );
                    return None;
                }
                let val = self.lower_typed_expr(*init, block, symtab)?;
                let result_ty = val.r#type();
                let loc = self.location();
                let builder = if floating {
                    OperationBuilder::new("arith.negf", loc).add_operands(&[val])
                } else {
                    let zero = block.const_int(self.mlir, result_ty, 0, loc);
                    OperationBuilder::new("arith.subi", loc).add_operands(&[zero, val])
                };
                let op = builder
                    .add_results(&[result_ty])
                    .build()
                    .map_err(|error| self.emit_internal(format!("negation lowering: {error}")))
                    .ok()?;
                Some(block.append_op(op))
            }
            typecheck::TypedExprKind::FixedArrayAlloc { init, size } => {
                let repr = typecheck::representation::Repr::derive(
                    expr_ref.ty,
                    Some(typed_ast.type_registry),
                )
                .ok();
                let Some(typecheck::representation::Repr::Array { length, .. }) = repr else {
                    // unresolved-fixed-array-construction
                    self.emit_symptom(
                        DiagnosticCode::Representation(
                            RepresentationCode::FixedArrayConstructionUnresolved,
                        ),
                        "fixed-array construction reached codegen without array representation"
                            .to_string(),
                    );
                    return None;
                };
                let matches_length =
                    typecheck::Normalize::normalize_to_value(size).is_some_and(|value| {
                        matches!(value, ast::ConstValue::Int(value)
                        if ast::integer::to_u64(value) == Some(length))
                    });
                if !matches_length {
                    // codegen-array-length-mismatch
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::ArrayLengthMismatch),
                        "fixed-array construction count does not match its selected storage extent"
                            .to_string(),
                    );
                    return None;
                }
                self.lower_repeated_array(*init, length, expr_ref.ty, block, symtab)
            }
            typecheck::TypedExprKind::Ref(init) if !expr_ref.ty.is_mutable_ref() => {
                if let Some(value) = typecheck::array_promotion::promoted_array_value(
                    self.program.exprs,
                    self.program.type_registry,
                    *init,
                ) {
                    self.lower_promoted_array(*init, value, block)
                } else {
                    self.lower_typed_place(*init, block, symtab)
                }
            }
            typecheck::TypedExprKind::TakePtr(init) | typecheck::TypedExprKind::Ref(init) => {
                self.lower_typed_place(*init, block, symtab)
            }
            typecheck::TypedExprKind::AggregateGet { base, index } => {
                let base_ty = typed_ast.expr(*base)?.ty;
                if let Ty::Sequence(fields) = base_ty {
                    let value = self
                        .lower_typed_values(*base, block, symtab)?
                        .values()
                        .get(*index)
                        .copied()?;
                    return self.convert_integer_value(
                        value,
                        &fields.get(*index)?.ty,
                        self.ty_to_mlir(expr_ref.ty),
                        block,
                    );
                }
                if let Some(pointee) = self.pointee_ty_for_reference_like(base_ty)
                    && let Some((element, _)) = typecheck::representation::Repr::array_parts(
                        &pointee,
                        Some(typed_ast.type_registry),
                    )
                {
                    let base = self.lower_typed_place(*base, block, symtab)?;
                    let address =
                        self.lower_constant_element_address(base, &pointee, *index, block)?;
                    let value = block.load_typed(
                        self,
                        address,
                        self.ty_to_mlir(&element),
                        self.location(),
                    )?;
                    return self.convert_integer_value(
                        value,
                        &element,
                        self.ty_to_mlir(expr_ref.ty),
                        block,
                    );
                }
                let base_val = self.lower_typed_expr(*base, block, symtab)?;
                let pointee = self.pointee_ty_for_reference_like(base_ty);
                let base_val = if let Some(pointee_ty) = &pointee {
                    block.load_typed(
                        self,
                        base_val,
                        self.ty_to_mlir(pointee_ty),
                        self.location(),
                    )?
                } else {
                    base_val
                };
                symtab.insert(
                    internment::Intern::new(format!("$call{}", base.0)),
                    base_val,
                    pointee.clone().unwrap_or_else(|| base_ty.clone()),
                    false,
                );
                let definition = self
                    .program
                    .type_registry
                    .resolved_definition_for_type(pointee.as_ref().unwrap_or(base_ty));
                let field_ty = match &definition {
                    Ty::Record { fields, .. } => fields.get(*index)?.1.as_ref(),
                    Ty::Sequence(fields) => fields.get(*index)?.ty.as_ref(),
                    Ty::Array { elem, .. } => elem.as_ref(),
                    _ => return None,
                };
                let value = block.append_op(self.mlir.llvm_extractvalue(
                    base_val,
                    *index as i64,
                    self.ty_to_mlir(field_ty),
                    self.location(),
                ));
                self.convert_integer_value(value, field_ty, self.ty_to_mlir(expr_ref.ty), block)
            }
            typecheck::TypedExprKind::Deref(base) => {
                let ptr = self.lower_typed_expr(*base, block, symtab)?;
                block.load_typed(self, ptr, self.ty_to_mlir(expr_ref.ty), self.location())
            }
            typecheck::TypedExprKind::AggregateSet {
                base,
                value,
                index,
                operator,
            } => {
                let base_ty = typed_ast.expr(*base)?.ty;
                let pointee = self.pointee_ty_for_reference_like(base_ty);
                let base_ty = pointee.as_ref().unwrap_or(base_ty);
                if let Some((element_ty, _)) = typecheck::representation::Repr::array_parts(
                    base_ty,
                    Some(typed_ast.type_registry),
                ) {
                    let base_ptr = self.lower_typed_place(*base, block, symtab)?;
                    let element_ptr =
                        self.lower_constant_element_address(base_ptr, base_ty, *index, block)?;
                    let rhs = self.lower_typed_expr(*value, block, symtab)?;
                    let value = if let Some(operator) = operator {
                        let lhs = block.load_typed(
                            self,
                            element_ptr,
                            self.ty_to_mlir(&element_ty),
                            self.location(),
                        )?;
                        self.lower_compound_operator(operator, lhs, rhs, &element_ty, block)?
                    } else {
                        rhs
                    };
                    block.store_typed(self, element_ptr, value, self.location())?;
                    Some(
                        block.append_op(
                            self.mlir
                                .llvm_undef(self.ty_to_mlir(expr_ref.ty), self.location()),
                        ),
                    )
                } else {
                    self.lower_typed_expr(*base, block, symtab)?;
                    self.lower_typed_expr(*value, block, symtab)
                }
            }
            typecheck::TypedExprKind::SliceGet { .. } => {
                // codegen-slice-requires-borrow
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::SliceRequiresBorrow),
                    "slice storage must be borrowed".to_string(),
                );
                None
            }
            typecheck::TypedExprKind::BufGet { buf, index } => {
                let (element_ptr, element_ty) =
                    self.lower_buffer_element(expr_id, *buf, *index, block, symtab)?;
                block.load_typed(
                    self,
                    element_ptr,
                    self.ty_to_mlir(&element_ty),
                    self.location(),
                )
            }
            typecheck::TypedExprKind::BufSet {
                buf,
                index,
                value,
                operator,
            } => {
                let (element_ptr, elem_ty) =
                    self.lower_buffer_element(expr_id, *buf, *index, block, symtab)?;
                let rhs = self.lower_typed_expr(*value, block, symtab)?;
                let value = if let Some(operator) = operator {
                    let lhs = block.load_typed(
                        self,
                        element_ptr,
                        self.ty_to_mlir(&elem_ty),
                        self.location(),
                    )?;
                    self.lower_compound_operator(operator, lhs, rhs, &elem_ty, block)?
                } else {
                    rhs
                };
                block.store_typed(self, element_ptr, value, self.location())?;
                Some(value)
            }
            typecheck::TypedExprKind::RecordSet {
                base,
                field,
                value,
                operator,
            } => {
                let base_ty = typed_ast.expr(*base)?.ty;
                let pointee = self.pointee_ty_for_reference_like(base_ty);
                let base_ty = pointee.as_ref().unwrap_or(base_ty);
                let resolved_base = self
                    .program
                    .type_registry
                    .resolved_definition_for_type(base_ty);
                let typecheck::ty::Ty::Record { fields, .. } = &resolved_base else {
                    self.emit_internal("record write base does not have record type".to_string());
                    return None;
                };
                let (field_index, field_ty) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == field)
                    .map(|(index, (_, ty))| (index, ty.as_ref()))?;
                let base_ptr = self.lower_typed_place(*base, block, symtab)?;
                let field_ptr = block.gep_typed(
                    self,
                    base_ptr,
                    self.ty_to_mlir(base_ty),
                    &[],
                    &[0, field_index as i32],
                    self.location(),
                )?;
                let rhs = self.lower_typed_expr(*value, block, symtab)?;
                let value = if let Some(operator) = operator {
                    let lhs = block.load_typed(
                        self,
                        field_ptr,
                        self.ty_to_mlir(field_ty),
                        self.location(),
                    )?;
                    self.lower_compound_operator(operator, lhs, rhs, field_ty, block)?
                } else {
                    rhs
                };
                block.store_typed(self, field_ptr, value, self.location())?;
                Some(value)
            }
            typecheck::TypedExprKind::List(items) => {
                if let Ok(typecheck::representation::Repr::Array { length, .. }) =
                    typecheck::representation::Repr::derive(
                        expr_ref.ty,
                        Some(typed_ast.type_registry),
                    )
                {
                    return self.lower_fixed_array_elements(
                        items,
                        length,
                        expr_ref.ty,
                        block,
                        symtab,
                    );
                }
                let vals = items
                    .iter()
                    .map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect::<Option<Vec<_>>>()?;
                vals.first().copied().or_else(|| {
                    self.is_empty_product(expr_ref.ty)
                        .then(|| block.unit_value(self, self.location()))
                })
            }
            typecheck::TypedExprKind::If(if_expr) => {
                self.lower_typed_if_values(if_expr, expr_ref.ty, block, symtab)?;
                Some(block.unit_value(self, self.location()))
            }
            typecheck::TypedExprKind::When(when_expr) => {
                let values = self.lower_typed_when_values(when_expr, expr_ref.ty, block, symtab)?;
                if self.is_empty_product(expr_ref.ty) {
                    Some(block.unit_value(self, self.location()))
                } else {
                    values.single()
                }
            }
            typecheck::TypedExprKind::Loop(loop_expr) => {
                self.lower_typed_loop(loop_expr, block, symtab)
            }
            typecheck::TypedExprKind::Destructure {
                value,
                field_bindings,
                ..
            } => {
                let source_ty = typed_ast.expr(*value)?.ty;
                let referenced = typecheck::ty::reference_pointee_for_type(
                    source_ty,
                    Some(typed_ast.type_registry),
                );
                let source_ty = referenced.as_ref().unwrap_or(source_ty);
                let Ty::Record { fields, .. } = typed_ast
                    .type_registry
                    .resolved_definition_for_type(source_ty)
                else {
                    self.emit_internal("record destructuring requires a record representation");
                    return None;
                };
                let aggregate = self.lower_typed_expr(*value, block, symtab)?;
                let aggregate = if referenced.is_some() {
                    block.load_typed(
                        self,
                        aggregate,
                        self.ty_to_mlir(source_ty),
                        self.location(),
                    )?
                } else {
                    aggregate
                };
                for (field, binding) in field_bindings {
                    let (index, (_, ty)) = fields
                        .iter()
                        .enumerate()
                        .find(|(_, (name, _))| name == field)?;
                    let value = block.append_op(self.mlir.llvm_extractvalue(
                        aggregate,
                        index as i64,
                        self.ty_to_mlir(ty),
                        self.location(),
                    ));
                    symtab.insert(*binding, value, (**ty).clone(), false);
                }
                Some(block.unit_value(self, self.location()))
            }
        }
    }

    fn lower_compound_operator(
        &self,
        operator: &typecheck::ResolvedCompoundOperator,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        result_ty: &typecheck::ty::Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        match operator {
            typecheck::ResolvedCompoundOperator::Callable { target, return_ty } => {
                Some(block.call(
                    self.mlir,
                    target.0.as_str(),
                    &[lhs, rhs],
                    self.ty_to_mlir(return_ty),
                    self.location(),
                ))
            }
            typecheck::ResolvedCompoundOperator::Intrinsic(op) => {
                let operation = match intrinsic_arith_name(op) {
                    Some(operation) => operation,
                    None => {
                        self.emit_internal(format!("invalid compound intrinsic {}", op.name()));
                        return None;
                    }
                };
                let operation = OperationBuilder::new(operation, self.location())
                    .add_operands(&[lhs, rhs])
                    .add_results(&[self.ty_to_mlir(result_ty)])
                    .build()
                    .map_err(|error| self.emit_internal(format!("compound lowering: {error}")))
                    .ok()?;
                Some(block.append_op(operation))
            }
            typecheck::ResolvedCompoundOperator::Invalid(error) => {
                self.emit_internal(format!("invalid compound operator: {error:?}"));
                None
            }
        }
    }

    fn lower_typed_place(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.program;
        let expr_ref = typed_ast.expr(expr_id)?;
        self.current_span.set(*expr_ref.span);

        match &expr_ref.kind {
            typecheck::TypedExprKind::Lit(ast::Literal::String(text)) => {
                self.lower_text_address(block, text, expr_ref.ty)
            }
            typecheck::TypedExprKind::FnCall { target, args, .. }
                if args.as_ref().is_none_or(Vec::is_empty) && symtab.get(&target.0).is_some() =>
            {
                self.materialize_named_place(target.0, block, symtab)
            }
            typecheck::TypedExprKind::SelfRef { target } => {
                self.materialize_named_place(target.0, block, symtab)
            }
            typecheck::TypedExprKind::Ref(inner) | typecheck::TypedExprKind::TakePtr(inner) => {
                self.lower_typed_place(*inner, block, symtab)
            }
            typecheck::TypedExprKind::Deref(inner) => self.lower_typed_expr(*inner, block, symtab),
            typecheck::TypedExprKind::UnionPayload { expr, variant } => {
                let base = self.lower_typed_place(*expr, block, symtab)?;
                let offset = self.sum_payload_offset(typed_ast.expr(*expr)?.ty, *variant, 0)?;
                let offset = block.const_i64(self.mlir, offset as i64, self.location());
                block.gep_i8(self, base, offset, self.location())
            }
            typecheck::TypedExprKind::AggregateGet { base, index } => {
                let base_ptr = self.lower_typed_place(*base, block, symtab)?;
                let base_ty = typed_ast.expr(*base)?.ty;
                let pointee = self.pointee_ty_for_reference_like(base_ty);
                let base_ty = pointee.as_ref().unwrap_or(base_ty);
                self.lower_constant_element_address(base_ptr, base_ty, *index, block)
            }
            typecheck::TypedExprKind::ArrayLit(_)
            | typecheck::TypedExprKind::FixedArrayAlloc { .. } => {
                let value = self.lower_typed_expr(expr_id, block, symtab)?;
                let pointer = self.allocate_stack_slot(value.r#type())?;
                block.store_typed(self, pointer, value, self.location())?;
                Some(pointer)
            }
            typecheck::TypedExprKind::SliceGet { buf, start, length } => {
                self.lower_slice_address(*buf, *start, *length, block, symtab)
            }
            typecheck::TypedExprKind::BufGet { buf, index } => self
                .lower_buffer_element(expr_id, *buf, *index, block, symtab)
                .map(|(address, _)| address),
            typecheck::TypedExprKind::FnCall { .. }
            | typecheck::TypedExprKind::IntrinsicCall { .. }
            | typecheck::TypedExprKind::Cast { .. }
            | typecheck::TypedExprKind::Rematerialize { .. }
            | typecheck::TypedExprKind::TagCall { .. }
            | typecheck::TypedExprKind::RecordValue(_)
            | typecheck::TypedExprKind::ResultRow(_)
            | typecheck::TypedExprKind::When(_)
            | typecheck::TypedExprKind::Block { .. } => {
                let value = self.lower_typed_expr(expr_id, block, symtab)?;
                if typecheck::ty::reference_pointee_for_type(
                    expr_ref.ty,
                    Some(typed_ast.type_registry),
                )
                .is_some()
                {
                    return Some(value);
                }
                let pointer = self.allocate_stack_slot(value.r#type())?;
                block.store_typed(self, pointer, value, self.location())?;
                Some(pointer)
            }
            _ => {
                self.emit_internal("expression is not an addressable place");
                None
            }
        }
    }

    fn materialize_named_place(
        &self,
        name: internment::Intern<String>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let binding = symtab.get(&name)?.clone();
        if binding.is_slot {
            return Some(binding.value);
        }
        let pointer = if typecheck::ty::reference_pointee_for_type(
            &binding.ty,
            Some(self.program.type_registry),
        )
        .is_some()
        {
            binding.value
        } else {
            let pointer = self.allocate_stack_slot(self.ty_to_mlir(&binding.ty))?;
            block.store_typed(self, pointer, binding.value, self.location())?;
            pointer
        };
        symtab.assign(
            &name,
            crate::Slot {
                value: pointer,
                ty: binding.ty,
                is_slot: true,
            },
        );
        Some(pointer)
    }
}
