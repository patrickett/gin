mod condition;
mod context;
mod equality;
mod if_;
mod loop_;
mod mlir;
mod storage;
mod ty_mapping;
mod when;

pub use context::{CodegenContext, CodegenSourceMap};

use crate::prelude::*;

use ast::HashFloat;
use ast::ty::TyArg;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use melior::ir::Location;
use melior::ir::operation::{OperationLike, OperationMutLike};
use typecheck::normal_expr::Normalize;

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
    pub fn build_module_from_resolved_program(
        context: &'c Context,
        program: typecheck::ResolvedProgram<'a>,
        source_map: CodegenSourceMap<'a>,
    ) -> (Option<melior::ir::Module<'c>>, Vec<Diagnostic>) {
        let output = Self::build_module_from_resolved_files(
            context,
            &[ResolvedFileInput::new(program, source_map)],
        );
        let diagnostics = output
            .diagnostics
            .into_iter()
            .next()
            .map(|file| file.diagnostics)
            .unwrap_or_default();
        (output.module, diagnostics)
    }

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

    pub fn build_module_from_resolved_files(
        context: &'c Context,
        files: &[ResolvedFileInput<'a>],
    ) -> ModuleCodegenOutput<'c> {
        Self::build_module_from_resolved_files_for_target(context, files, &CompileTarget::Library)
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
                            diagnostics: vec![Diagnostic::new(
                                "unsupported-target-layout",
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
            succeeded &= ctx.append_resolved_program_to_module(&module);
            ctx.emit_string_globals(&module);
            diagnostics.push(FileCodegenDiagnostics {
                filename: file.source_map.filename.clone(),
                diagnostics: ctx.drain_symptoms(),
            });
        }

        if succeeded
            && !module.as_operation().verify()
            && std::env::var("CODEGEN_SKIP_VERIFY").is_err()
        {
            succeeded = false;
            if let Some(file) = diagnostics.first_mut() {
                file.diagnostics.push(
                    Diagnostic::new(
                        "codegen-invalid-module",
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
            if bind.definition_stage == typecheck::typed::DefinitionStage::CompileTimeOnly {
                continue;
            }
            let mut symtab = crate::ScopedSymbolTable::new();
            let func_name = def_id.0.as_str();
            let declared_return_ty = program
                .fn_return_types
                .get(def_id)
                .unwrap_or(&bind.return_type);
            let function_return_ty = match &bind.body {
                typecheck::BindBody::Expr(expr_id) => {
                    program.exprs.ty_of(*expr_id).unwrap_or(declared_return_ty)
                }
                typecheck::BindBody::Body {
                    ret: Some(expr_id), ..
                } => program.exprs.ty_of(*expr_id).unwrap_or(declared_return_ty),
                typecheck::BindBody::Body { ret: None, .. } | typecheck::BindBody::Unlowered => {
                    declared_return_ty
                }
            };
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
                        definition: def_id.0,
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
            let (result_id, result_val) = match &bind.body {
                typecheck::BindBody::Expr(eid) => {
                    let lowered = self.lower_typed_values(*eid, &blk, &mut symtab);
                    (Some(*eid), lowered)
                }
                typecheck::BindBody::Body { exprs, ret } => {
                    let result_id = *ret;
                    let result_val = self.lower_typed_returning_sequence(
                        exprs,
                        *ret,
                        function_return_ty,
                        &blk,
                        &mut symtab,
                    );
                    (result_id, result_val)
                }
                typecheck::BindBody::Unlowered => unreachable!(),
            };

            let actual_return_ty = result_val
                .as_ref()
                .and_then(|_| result_id.and_then(|result_id| typed.exprs.ty_of(result_id)))
                .unwrap_or(function_return_ty);
            let function_return_ty = if self.is_empty_product(function_return_ty)
                && !self.is_empty_product(actual_return_ty)
            {
                actual_return_ty
            } else {
                function_return_ty
            };

            let entry_integer_return = func_name == "main"
                && self
                    .program
                    .type_registry
                    .integer_validity_for_type(function_return_ty)
                    .is_some();
            let ret_types = if self.is_empty_product(function_return_ty) {
                if func_name == "main" {
                    let success = blk.const_i64(self.mlir, 0, loc);
                    blk.append_operation(func::r#return(&[success], loc));
                    vec![self.mlir.i64()]
                } else {
                    blk.append_operation(func::r#return(&[], loc));
                    Vec::new()
                }
            } else if !entry_integer_return {
                let ret_types = self.result_types_to_mlir(function_return_ty);
                match result_val {
                    Some(values) => blk.append_operation(func::r#return(values.values(), loc)),
                    None => blk.append_operation(func::r#return(&[], loc)),
                };
                ret_types
            } else {
                let ret_ty = self.mlir.i64();
                match result_val.and_then(|value| value.single()) {
                    Some(val) if val.r#type() != ret_ty => {
                        let cast = self.convert_integer_value(val, actual_return_ty, ret_ty, &blk);
                        match cast {
                            Some(value) => blk.append_operation(func::r#return(&[value], loc)),
                            None => blk.append_operation(func::r#return(&[], loc)),
                        }
                    }
                    Some(val) => blk.append_operation(func::r#return(&[val], loc)),
                    None => blk.append_operation(func::r#return(&[], loc)),
                };
                vec![ret_ty]
            };

            if !self.append_function(
                module,
                func_name,
                &param_types_mlir,
                &ret_types,
                region,
                None,
                loc,
            ) {
                return false;
            }
        }

        true
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

    fn lower_checked_assembly(
        &self,
        assembly: &typecheck::assembly::CheckedAssembly,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let mut operands = Vec::with_capacity(assembly.operands.len());
        for (_, operand) in &assembly.operands {
            let Some(value) = self.lower_typed_expr(*operand, block, symtab) else {
                self.emit_symptom(
                    "codegen-assembly-operand",
                    "checked assembly operand could not be lowered".to_string(),
                );
                return None;
            };
            operands.push(value);
        }
        let constraints = assembly.llvm_constraints();
        let attributes = vec![
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
            (
                Identifier::new(self.mlir, "has_side_effects"),
                melior::ir::Attribute::unit(self.mlir),
            ),
        ];
        let result_types = if assembly.outputs.is_empty() {
            assembly
                .result
                .as_ref()
                .map(|_| self.result_types_to_mlir(result_ty))
                .unwrap_or_default()
        } else {
            self.result_types_to_mlir(result_ty)
        };
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
            .ok()?;
        let operation = block.append_operation(operation);
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

    pub fn lower_typed_values(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let expr = self.program.expr(expr_id)?;
        if let typecheck::TypedExprKind::Asm {
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
            let return_ty = self
                .program
                .fn_return_types
                .get(target)
                .or_else(|| self.program.defs.get(target).map(|bind| &bind.return_type))
                .unwrap_or(expr.ty);
            let return_ty = substituted_ty.as_ref().unwrap_or(return_ty);
            if matches!(return_ty, Ty::ResultRow(_)) {
                if args.as_ref().is_none_or(Vec::is_empty)
                    && let Some(value) = symtab.get_lowered(&target.0)
                {
                    return Some(value.clone());
                }
                let mut lowered_args = Vec::new();
                if let Some(args) = args {
                    let conventions = self
                        .program
                        .defs
                        .get(target)
                        .map(|bind| bind.param_conventions.as_slice())
                        .unwrap_or_default();
                    for (index, arg) in args.iter().enumerate() {
                        let value = match conventions.get(index) {
                            Some(ParamConvention::Observe | ParamConvention::Mutate) => {
                                self.lower_typed_place(*arg, block, symtab)
                            }
                            _ => self.lower_typed_expr(*arg, block, symtab),
                        }?;
                        lowered_args.push(value);
                    }
                }
                let operation = OperationBuilder::new("func.call", self.location())
                    .add_attributes(&[(
                        Identifier::new(self.mlir, "callee"),
                        self.mlir.symbol_ref_attr(target.0.as_str()),
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
        }
        match &expr.kind {
            typecheck::TypedExprKind::ResultRow(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.extend(self.lower_typed_values(*item, block, symtab)?.into_values());
                }
                Some(LoweredValue::new(values))
            }
            typecheck::TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned: false,
            } if matches!(expr.ty, Ty::ResultRow(_)) => {
                for stmt in stmts {
                    self.lower_typed_values(*stmt, block, symtab);
                }
                let value = self.lower_typed_values(*body, block, symtab)?;
                symtab.insert_lowered(*name, value.clone(), expr.ty.clone());
                Some(value)
            }
            typecheck::TypedExprKind::Reassign { name, value, .. }
                if matches!(expr.ty, Ty::ResultRow(_)) =>
            {
                let value = self.lower_typed_values(*value, block, symtab)?;
                symtab
                    .assign_lowered(name, value.clone(), expr.ty.clone())
                    .then_some(value)
            }
            typecheck::TypedExprKind::SelfRef { target } if matches!(expr.ty, Ty::ResultRow(_)) => {
                symtab.get_lowered(&target.0).cloned()
            }
            typecheck::TypedExprKind::AggregateGet { base, index }
                if matches!(self.program.expr(*base)?.ty, Ty::ResultRow(_)) =>
            {
                let values = self.lower_typed_values(*base, block, symtab)?;
                values
                    .values()
                    .get(*index)
                    .copied()
                    .map(LoweredValue::scalar)
            }
            typecheck::TypedExprKind::If(if_expr)
                if matches!(expr.ty, Ty::ResultRow(_)) || self.is_empty_product(expr.ty) =>
            {
                self.lower_typed_if_values(if_expr, expr.ty, block, symtab)
            }
            typecheck::TypedExprKind::When(when_expr)
                if matches!(expr.ty, Ty::ResultRow(_)) || self.is_empty_product(expr.ty) =>
            {
                self.lower_typed_when_values(when_expr, expr.ty, block, symtab)
            }
            _ if self.is_empty_product(expr.ty) => {
                self.lower_typed_expr(expr_id, block, symtab)?;
                Some(LoweredValue::new(Vec::new()))
            }
            _ => self
                .lower_typed_expr(expr_id, block, symtab)
                .map(LoweredValue::scalar),
        }
    }

    pub fn lower_typed_expr(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.program;
        let expr_ref = typed_ast.expr(expr_id)?;
        // Propagate source location from the original expression span.
        self.current_span.set(*expr_ref.span);

        if matches!(expr_ref.ty, ast::Ty::UnresolvedLiteral(_)) {
            match expr_ref.ty {
                ast::Ty::UnresolvedLiteral(ast::ty::LiteralKind::Text) => self.emit_symptom(
                    "type-unresolved-text-literal-codegen",
                    "unresolved text literal reached code generation".to_string(),
                ),
                _ => self.emit_internal("unresolved integer literal reached code generation"),
            }
            return None;
        }
        if matches!(expr_ref.ty, ast::Ty::UnresolvedSequence { .. }) {
            self.emit_symptom(
                "type-unresolved-sequence-codegen",
                "unresolved sequence literal reached code generation".to_string(),
            );
            return None;
        }
        if expr_ref.ty.is_unresolved_address() {
            self.emit_symptom(
                "type-unresolved-address-codegen",
                "unresolved raw address reached code generation".to_string(),
            );
            return None;
        }

        match expr_ref.kind {
            typecheck::TypedExprKind::Invalid => None,
            typecheck::TypedExprKind::Register(inner) => {
                self.lower_typed_values(*inner, block, symtab)?.single()
            }
            typecheck::TypedExprKind::Target => {
                self.emit_symptom(
                    "codegen-target-query",
                    "#target is compile-time-only".to_string(),
                );
                None
            }
            typecheck::TypedExprKind::TargetField(_) => None,
            typecheck::TypedExprKind::Asm {
                specification,
                operands: _,
                state,
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
                    self.emit_symptom(
                        "codegen-assembly-not-checked",
                        "pending or invalid assembly reached code generation".to_string(),
                    );
                    None
                }
            }
            typecheck::TypedExprKind::Lit(lit) => match lit {
                ast::Literal::Number(0) => {
                    if let Some(value) = self.lower_synthetic_record_field(expr_id, block, symtab) {
                        return Some(value);
                    }
                    let value = match expr_ref.facts.const_facts.const_value.as_ref() {
                        Some(ast::ConstValue::Int(value)) => {
                            ast::integer::to_i128(*value).unwrap_or(0)
                        }
                        _ => 0,
                    };
                    Some(block.const_int(
                        self.mlir,
                        self.ty_to_mlir(expr_ref.ty),
                        value,
                        self.location(),
                    ))
                }
                ast::Literal::Number(_) | ast::Literal::Int(_) => {
                    let Some(ast::ConstValue::Int(value)) =
                        expr_ref.facts.const_facts.const_value.as_ref()
                    else {
                        self.emit_internal("materialized integer literal has no constant value");
                        return None;
                    };
                    Some(block.const_int(
                        self.mlir,
                        self.ty_to_mlir(expr_ref.ty),
                        ast::integer::to_i128(*value)?,
                        self.location(),
                    ))
                }
                ast::Literal::Float(HashFloat(f)) => {
                    let f_attr = self.mlir.f64_attr(*f);
                    Some(block.append_op(self.mlir.const_op(
                        f_attr,
                        self.mlir.f64(),
                        self.location(),
                    )))
                }
                ast::Literal::String(s) => {
                    let name = self.register_string(s);
                    let ptr = self.addressof_string_global(block, &name, self.location())?;
                    block.load_typed(self, ptr, self.ty_to_mlir(expr_ref.ty), self.location())
                }
            },
            typecheck::TypedExprKind::Rematerialize { constant, .. } => {
                let value = match constant {
                    ast::ConstValue::Int(value) => ast::integer::to_i128(*value)?,
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
                let mut lowered_args = Vec::new();
                if let Some(args) = args {
                    let conventions = typed_ast
                        .defs
                        .get(target)
                        .map(|bind| bind.param_conventions.as_slice())
                        .unwrap_or_default();
                    for (i, arg_id) in args.iter().enumerate() {
                        let arg = match conventions.get(i) {
                            Some(ParamConvention::Observe | ParamConvention::Mutate) => {
                                self.lower_typed_place(*arg_id, block, symtab)
                            }
                            _ => self.lower_typed_expr(*arg_id, block, symtab),
                        }?;
                        lowered_args.push(arg);
                    }
                }

                let loc = self.location();

                let fn_name = target.0.as_str();

                if (args.is_none() || args.as_ref().is_none_or(|a| a.is_empty()))
                    && let Some(slot) = symtab.get(&target.0)
                {
                    return if slot.is_slot {
                        let reference_slot = typecheck::ty::reference_pointee_for_type(
                            &slot.ty,
                            Some(self.program.type_registry),
                        )
                        .is_some();
                        if reference_slot
                            && typecheck::ty::reference_pointee_for_type(
                                expr_ref.ty,
                                Some(self.program.type_registry),
                            )
                            .is_some()
                        {
                            Some(slot.value)
                        } else {
                            block.load_typed(
                                self,
                                slot.value,
                                self.ty_to_mlir(if reference_slot {
                                    expr_ref.ty
                                } else {
                                    &slot.ty
                                }),
                                self.location(),
                            )
                        }
                    } else {
                        Some(slot.value)
                    };
                }

                let return_ty = self
                    .program
                    .fn_return_types
                    .get(target)
                    .or_else(|| typed_ast.defs.get(target).map(|bind| &bind.return_type))
                    .unwrap_or(expr_ref.ty);
                let return_ty = substituted_ty.as_ref().unwrap_or(return_ty);

                if !typed_ast.defs.contains_key(target)
                    && !self.program.fn_return_types.contains_key(target)
                {
                    if self.is_empty_product(return_ty) {
                        return None;
                    }
                    return Some(
                        block.append_op(self.mlir.llvm_undef(self.ty_to_mlir(return_ty), loc)),
                    );
                }

                if self.is_empty_product(return_ty) {
                    block.call_unit(self.mlir, fn_name, &lowered_args, loc);
                    return Some(block.unit_value(self, loc));
                }

                let return_ty = self.ty_to_mlir(return_ty);
                Some(block.call(self.mlir, fn_name, &lowered_args, return_ty, loc))
            }
            typecheck::TypedExprKind::IntrinsicCall { op, args } => {
                let mut lowered: Vec<_> = args
                    .iter()
                    .map(|arg| self.lower_typed_expr(*arg, block, symtab))
                    .collect::<Option<_>>()?;
                match op {
                    typecheck::typed::IntrinsicOp::AddressLoad => {
                        let [address] = lowered.as_slice() else {
                            self.emit_internal(format!("invalid typed {} arity", op.name()));
                            return None;
                        };
                        let address_ty = typed_ast.expr(*args.first()?)?.ty;
                        return block.load_typed(
                            self,
                            *address,
                            self.ty_to_mlir(&self.pointee_ty_for_reference_like(address_ty)?),
                            self.location(),
                        );
                    }
                    typecheck::typed::IntrinsicOp::AddressStore => {
                        let [address, value] = lowered.as_slice() else {
                            self.emit_internal(format!("invalid typed {} arity", op.name()));
                            return None;
                        };
                        block.store_typed(self, *address, *value, self.location())?;
                        return Some(block.unit_value(self, self.location()));
                    }
                    typecheck::typed::IntrinsicOp::AddressOffset => {
                        let [address, index] = lowered.as_slice() else {
                            self.emit_internal(format!("invalid typed {} arity", op.name()));
                            return None;
                        };
                        let address_ty = typed_ast.expr(*args.first()?)?.ty;
                        let pointee = self.pointee_ty_for_reference_like(address_ty)?;
                        if !self.validate_pointee_stride(&pointee) {
                            return None;
                        }
                        let scaled_index = self.scale_pointer_index(block, *index, &pointee)?;
                        return block.gep_typed_result(
                            self,
                            *address,
                            IntegerType::new(self.mlir, 8).into(),
                            &[scaled_index],
                            &[i32::MIN],
                            self.ty_to_mlir(expr_ref.ty),
                            self.location(),
                        );
                    }
                    typecheck::typed::IntrinsicOp::AddressToBits => {
                        let [address] = lowered.as_slice() else {
                            self.emit_internal(format!("invalid typed {} arity", op.name()));
                            return None;
                        };
                        let operation = OperationBuilder::new("llvm.ptrtoint", self.location())
                            .add_operands(&[*address])
                            .add_results(&[self.ty_to_mlir(expr_ref.ty)])
                            .build()
                            .map_err(|error| self.emit_internal(format!("ptrtoint: {error}")))
                            .ok()?;
                        return Some(block.append_op(operation));
                    }
                    typecheck::typed::IntrinsicOp::BitsToAddress => {
                        let [bits] = lowered.as_slice() else {
                            self.emit_internal(format!("invalid typed {} arity", op.name()));
                            return None;
                        };
                        let operation = OperationBuilder::new("llvm.inttoptr", self.location())
                            .add_operands(&[*bits])
                            .add_results(&[self.ty_to_mlir(expr_ref.ty)])
                            .build()
                            .map_err(|error| self.emit_internal(format!("inttoptr: {error}")))
                            .ok()?;
                        return Some(block.append_op(operation));
                    }
                    _ => {}
                }
                if let typecheck::typed::IntrinsicOp::BitsCompare {
                    interpretation,
                    predicate,
                } = op
                {
                    let [lhs, rhs] = lowered.as_slice() else {
                        self.emit_internal(format!("invalid typed {} arity", op.name()));
                        return None;
                    };
                    let signed = *interpretation == typecheck::intrinsic::IntegerComparison::Signed;
                    let width = args
                        .iter()
                        .zip([*lhs, *rhs])
                        .map(|(arg, value)| {
                            let ty = typed_ast.expr(*arg)?.ty;
                            let width = IntegerType::try_from(value.r#type()).ok()?.width();
                            let extra = signed
                                && matches!(ty, Ty::AnonymousInteger { .. })
                                && ty.anonymous_operation_interpretation()
                                    == Some(ast::ty::IntegerInterpretation::Unsigned);
                            Some(width + u32::from(extra))
                        })
                        .collect::<Option<Vec<_>>>()?
                        .into_iter()
                        .max()?;
                    let operand_ty = IntegerType::new(self.mlir, width).into();
                    let lhs = self.convert_integer_value(
                        *lhs,
                        typed_ast.expr(args[0])?.ty,
                        operand_ty,
                        block,
                    )?;
                    let rhs = self.convert_integer_value(
                        *rhs,
                        typed_ast.expr(args[1])?.ty,
                        operand_ty,
                        block,
                    )?;
                    let predicate = match (interpretation, predicate) {
                        (
                            typecheck::intrinsic::IntegerComparison::Signed,
                            typecheck::intrinsic::ComparisonPredicate::Less,
                        ) => Predicates::SLT,
                        (
                            typecheck::intrinsic::IntegerComparison::Signed,
                            typecheck::intrinsic::ComparisonPredicate::LessOrEqual,
                        ) => Predicates::SLE,
                        (
                            typecheck::intrinsic::IntegerComparison::Signed,
                            typecheck::intrinsic::ComparisonPredicate::Greater,
                        ) => Predicates::SGT,
                        (
                            typecheck::intrinsic::IntegerComparison::Signed,
                            typecheck::intrinsic::ComparisonPredicate::GreaterOrEqual,
                        ) => Predicates::SGE,
                        (
                            typecheck::intrinsic::IntegerComparison::Signed,
                            typecheck::intrinsic::ComparisonPredicate::Equal,
                        ) => Predicates::EQ,
                        (
                            typecheck::intrinsic::IntegerComparison::Unsigned,
                            typecheck::intrinsic::ComparisonPredicate::Less,
                        ) => Predicates::ULT,
                        (
                            typecheck::intrinsic::IntegerComparison::Unsigned,
                            typecheck::intrinsic::ComparisonPredicate::LessOrEqual,
                        ) => Predicates::ULE,
                        (
                            typecheck::intrinsic::IntegerComparison::Unsigned,
                            typecheck::intrinsic::ComparisonPredicate::Greater,
                        ) => Predicates::UGT,
                        (
                            typecheck::intrinsic::IntegerComparison::Unsigned,
                            typecheck::intrinsic::ComparisonPredicate::GreaterOrEqual,
                        ) => Predicates::UGE,
                        (
                            typecheck::intrinsic::IntegerComparison::Unsigned,
                            typecheck::intrinsic::ComparisonPredicate::Equal,
                        ) => Predicates::EQ,
                    };
                    let compared =
                        block.append_op(self.mlir.build_cmpi(predicate, lhs, rhs, self.location()));
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
                let operation = match op {
                    typecheck::typed::IntrinsicOp::BitsAdd => "arith.addi",
                    typecheck::typed::IntrinsicOp::BitsSubtract => "arith.subi",
                    typecheck::typed::IntrinsicOp::BitsMultiply => "arith.muli",
                    typecheck::typed::IntrinsicOp::BitsSignedDivide => "arith.divsi",
                    typecheck::typed::IntrinsicOp::BitsUnsignedDivide => "arith.divui",
                    typecheck::typed::IntrinsicOp::BitsSignedRemainder => "arith.remsi",
                    typecheck::typed::IntrinsicOp::BitsUnsignedRemainder => "arith.remui",
                    typecheck::typed::IntrinsicOp::BitsAnd => "arith.andi",
                    typecheck::typed::IntrinsicOp::BitsOr => "arith.ori",
                    typecheck::typed::IntrinsicOp::BitsXor => "arith.xori",
                    typecheck::typed::IntrinsicOp::BitsShiftLeft => "arith.shli",
                    typecheck::typed::IntrinsicOp::BitsShiftRightLogical => "arith.shrui",
                    typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic => "arith.shrsi",
                    typecheck::typed::IntrinsicOp::BitsExtendSigned => "arith.extsi",
                    typecheck::typed::IntrinsicOp::BitsExtendUnsigned => "arith.extui",
                    typecheck::typed::IntrinsicOp::BitsTruncate => "arith.trunci",
                    typecheck::typed::IntrinsicOp::BitsCompare { .. } => unreachable!(),
                    typecheck::typed::IntrinsicOp::AddressLoad
                    | typecheck::typed::IntrinsicOp::AddressStore
                    | typecheck::typed::IntrinsicOp::AddressOffset
                    | typecheck::typed::IntrinsicOp::AddressToBits
                    | typecheck::typed::IntrinsicOp::BitsToAddress => unreachable!(),
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
                if matches!(
                    op,
                    typecheck::typed::IntrinsicOp::BitsShiftLeft
                        | typecheck::typed::IntrinsicOp::BitsShiftRightLogical
                        | typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic
                ) {
                    let [_, count] = lowered.as_slice() else {
                        self.emit_internal(format!("invalid typed {} arity", op.name()));
                        return None;
                    };
                    let width = expr_ref.ty.anonymous_integer_width().unwrap_or_default();
                    let width_value =
                        block.const_int(self.mlir, result_ty, i128::from(width), self.location());
                    let reduced = OperationBuilder::new("arith.remui", self.location())
                        .add_operands(&[*count, width_value])
                        .add_results(&[result_ty])
                        .build()
                        .map_err(|error| {
                            self.emit_internal(format!("shift count lowering: {error}"))
                        })
                        .ok()?;
                    lowered[1] = block.append_op(reduced);
                }
                let expected = match op.contract() {
                    typecheck::intrinsic::IntrinsicContract::SameWidthBinary => 2,
                    typecheck::intrinsic::IntrinsicContract::Comparison => 2,
                    typecheck::intrinsic::IntrinsicContract::Extension
                    | typecheck::intrinsic::IntrinsicContract::Truncation => 1,
                    typecheck::intrinsic::IntrinsicContract::AddressLoad
                    | typecheck::intrinsic::IntrinsicContract::AddressStore
                    | typecheck::intrinsic::IntrinsicContract::AddressOffset
                    | typecheck::intrinsic::IntrinsicContract::AddressConversion => unreachable!(),
                };
                if lowered.len() != expected {
                    self.emit_internal(format!("invalid typed {} arity", op.name()));
                    return None;
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
                for stmt_id in stmts {
                    self.lower_typed_expr(*stmt_id, block, symtab)?;
                }

                if *unassigned {
                    // Declaration without value: allocate stack space but don't initialize.
                    // Use the declared type to determine the alloca size.
                    let loc = self.location();
                    let mlir_ty = self.ty_to_mlir(expr_ref.ty);
                    let slot = block.alloca_typed(self.mlir, mlir_ty, loc);
                    symtab.insert(*name, slot, expr_ref.ty.clone(), true);
                    Some(slot)
                } else {
                    let val = self.lower_typed_expr(*body, block, symtab)?;
                    symtab.insert(*name, val, expr_ref.ty.clone(), false);
                    if typed_ast
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
                    if slot.is_slot {
                        block.store_typed(self, slot.value, val, self.location())?;
                    } else {
                        symtab.assign(
                            name,
                            crate::Slot {
                                value: val,
                                ty: expr_ref.ty.clone(),
                                is_slot: false,
                            },
                        );
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
            typecheck::TypedExprKind::Binary { op, lhs, rhs } => {
                if *op == BinOp::Equal {
                    let lhs_ty = typed_ast.expr(*lhs)?.ty;
                    let lhs = self.lower_typed_expr(*lhs, block, symtab)?;
                    let rhs = self.lower_typed_expr(*rhs, block, symtab)?;
                    return self.lower_equality(block, lhs, rhs, lhs_ty);
                }
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
                field_names: _,
            } => {
                let constructed_ty = expr_ref.ty;
                let constructor_ty = typed_ast
                    .tag_types
                    .get(&variant_id.union)
                    .cloned()
                    .unwrap_or_else(|| constructed_ty.clone());
                let fixed_array_status = |ty: &Ty| -> (bool, Option<(Ty, u64)>) {
                    let Some(instance) = ty.named_instance_stripping_reference_wrappers() else {
                        return (false, None);
                    };
                    let Some(declaration) = typed_ast
                        .type_registry
                        .declaration_for_instance(instance)
                        .or_else(|| {
                            typed_ast
                                .type_registry
                                .declaration_ids_by_display_name(instance.declaration.name.as_str())
                                .into_iter()
                                .find_map(|id| typed_ast.type_registry.declaration(id))
                        })
                    else {
                        return (false, None);
                    };
                    let Some(acceptance) = declaration.sequence_literal.as_ref() else {
                        return (false, None);
                    };
                    let typecheck::SequenceLiteralMaterialization::FixedArray { length_parameter } =
                        acceptance.materialization
                    else {
                        return (false, None);
                    };
                    let element = instance
                        .arguments
                        .iter()
                        .find_map(|(parameter, argument)| {
                            (*parameter == acceptance.element_parameter).then_some(argument)
                        })
                        .and_then(|argument| match argument {
                            TyArg::Type(ty) => Some((**ty).clone()),
                            TyArg::Const(_) => None,
                        });
                    let Some(element) = element else {
                        return (true, None);
                    };
                    let length = instance.arguments.iter().find_map(|(parameter, argument)| {
                        (*parameter == length_parameter).then_some(argument)
                    });
                    let Some(length) = length else {
                        return (true, None);
                    };
                    let Some(length) = (match length {
                        TyArg::Const(expr) => match expr.normalize_to_value() {
                            Some(ast::ConstValue::Int(value)) => ast::integer::to_u64(value),
                            _ => None,
                        },
                        TyArg::Type(ty) => {
                            match self.program.type_registry.resolved_definition_for_type(ty) {
                                Ty::Literal(ast::ConstValue::Int(value)) => {
                                    ast::integer::to_u64(value)
                                }
                                _ => None,
                            }
                        }
                    }) else {
                        return (true, None);
                    };
                    (true, Some((element, length)))
                };
                let (constructed_known, constructed_candidate) = fixed_array_status(constructed_ty);
                let (constructor_known, constructor_candidate) =
                    fixed_array_status(&constructor_ty);
                let is_any_known_fixed_array_former = constructed_known || constructor_known;
                let fixed_array_candidate = constructed_candidate.or(constructor_candidate);
                if let Some((_array_ty, length)) = fixed_array_candidate {
                    let Some(args) = args.as_deref() else {
                        self.emit_internal("fixed-array constructor expects a value argument");
                        return None;
                    };
                    let Some(init) = args.first() else {
                        self.emit_internal("fixed-array constructor expects a value argument");
                        return None;
                    };
                    let init = match self.lower_typed_expr(*init, block, symtab) {
                        Some(init) => Some(init),
                        None => {
                            let expr = typed_ast.expr(*init)?;
                            if let Some(const_value) = &expr.facts.const_facts.const_value {
                                self.lower_const_value(block, const_value, expr.ty)
                            } else if self
                                .program
                                .type_registry
                                .integer_validity_for_type(expr.ty)
                                .is_some()
                            {
                                Some(block.const_int(
                                    self.mlir,
                                    self.ty_to_mlir(expr.ty),
                                    0,
                                    self.location(),
                                ))
                            } else if self
                                .program
                                .type_registry
                                .resolved_definition_for_type(expr.ty)
                                .union_literal_values()
                                .is_some()
                            {
                                if let typecheck::TypedExprKind::TagCall { discriminant, .. } =
                                    &expr.kind
                                {
                                    let value = i64::try_from(*discriminant).ok()?;
                                    Some(block.const_int(
                                        self.mlir,
                                        self.ty_to_mlir(expr.ty),
                                        value.into(),
                                        self.location(),
                                    ))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    }?;
                    let array_ty = self.ty_to_mlir(constructed_ty);
                    let mut value =
                        block.append_op(self.mlir.llvm_undef(array_ty, self.location()));
                    for index in 0..length {
                        value = block.append_op(self.mlir.llvm_insertvalue(
                            value,
                            init,
                            i64::try_from(index).ok()?,
                            self.location(),
                        ));
                    }
                    return Some(value);
                }
                if is_any_known_fixed_array_former {
                    self.emit_symptom(
                        "unresolved-fixed-array-construction",
                        "fixed-array construction reached codegen without array representation"
                            .to_string(),
                    );
                    return Some(
                        block.append_op(
                            self.mlir
                                .llvm_undef(self.ty_to_mlir(constructed_ty), self.location()),
                        ),
                    );
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
                if is_payload_free_union {
                    let result_ty = self.ty_to_mlir(constructed_ty);
                    return Some(block.const_int(
                        self.mlir,
                        result_ty,
                        *discriminant as i128,
                        self.location(),
                    ));
                }
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

                let lowered_args: Vec<Value<'c, 'c>> = args
                    .as_ref()
                    .map(|a| {
                        a.iter()
                            .filter_map(|arg_id| self.lower_typed_expr(*arg_id, block, symtab))
                            .collect()
                    })
                    .unwrap_or_default();
                if matches!(&resolved_constructed, Ty::Union { .. }) {
                    return self.lower_sum_constructor(
                        block,
                        constructed_ty,
                        *discriminant,
                        &lowered_args,
                    );
                }
                if !matches!(&resolved_constructed, Ty::Record { .. }) {
                    self.emit_symptom(
                        "sum-representation-missing",
                        "non-record tag construction reached codegen without a tagged-sum representation"
                            .to_string(),
                    );
                    return None;
                }
                let field_types: Vec<Type<'c>> = lowered_args.iter().map(|v| v.r#type()).collect();
                use melior::dialect::llvm::r#type;
                let struct_ty = r#type::r#struct(self.mlir, &field_types, false);
                let undef = block.append_op(self.mlir.llvm_undef(struct_ty, self.location()));
                let seed = undef;
                let result = lowered_args
                    .into_iter()
                    .enumerate()
                    .fold(seed, |acc, (i, v)| {
                        block.append_op(self.mlir.llvm_insertvalue(
                            acc,
                            v,
                            i as i64,
                            self.location(),
                        ))
                    });
                Some(result)
            }
            typecheck::TypedExprKind::SelfRef { target } => {
                let slot = symtab.get(&target.0)?;
                if slot.is_slot {
                    block.load_typed(
                        self,
                        slot.value,
                        self.ty_to_mlir(expr_ref.ty),
                        self.location(),
                    )
                } else {
                    Some(slot.value)
                }
            }
            typecheck::TypedExprKind::Range { start, end } => {
                let _s = self.lower_typed_expr(*start, block, symtab)?;
                let _e = self.lower_typed_expr(*end, block, symtab)?;
                None // Range lowering not yet implemented
            }
            typecheck::TypedExprKind::RecordValue(items) => {
                let vals: Vec<Value<'c, 'c>> = items
                    .iter()
                    .filter_map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect();
                if vals.is_empty() {
                    return None;
                }

                let aggregate_ty = self.ty_to_mlir(expr_ref.ty);
                let mut value =
                    block.append_op(self.mlir.llvm_undef(aggregate_ty, self.location()));
                for (index, item) in vals.into_iter().enumerate() {
                    value = block.append_op(self.mlir.llvm_insertvalue(
                        value,
                        item,
                        i64::try_from(index).ok()?,
                        self.location(),
                    ));
                }
                Some(value)
            }
            typecheck::TypedExprKind::ResultRow(_) => {
                self.emit_internal("result row reached scalar lowering");
                None
            }
            typecheck::TypedExprKind::ArrayLit(items) => {
                let repr = typecheck::representation::Repr::derive(
                    expr_ref.ty,
                    Some(typed_ast.type_registry),
                )
                .ok();
                let Some(typecheck::representation::Repr::Array { length, .. }) = repr else {
                    self.emit_symptom(
                        "unresolved-fixed-array-construction",
                        "fixed-array literal reached codegen without array representation"
                            .to_string(),
                    );
                    return None;
                };
                let array_ty = self.ty_to_mlir(expr_ref.ty);
                let mut value = block.append_op(self.mlir.llvm_undef(array_ty, self.location()));
                for (index, item) in items.iter().take(usize::try_from(length).ok()?).enumerate() {
                    let element = self.lower_typed_expr(*item, block, symtab)?;
                    value = block.append_op(self.mlir.llvm_insertvalue(
                        value,
                        element,
                        i64::try_from(index).ok()?,
                        self.location(),
                    ));
                }
                Some(value)
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
                let val = self.lower_typed_expr(*init, block, symtab)?;
                let result_ty = val.r#type();
                let zero = block.const_int(self.mlir, result_ty, 0, self.location());
                let loc = self.location();
                let op = melior::ir::operation::OperationBuilder::new("arith.subi", loc)
                    .add_operands(&[zero, val])
                    .add_results(&[result_ty])
                    .build()
                    .ok()?;
                Some(block.append_op(op))
            }
            typecheck::TypedExprKind::FixedArrayAlloc { init, .. } => {
                let repr = typecheck::representation::Repr::derive(
                    expr_ref.ty,
                    Some(typed_ast.type_registry),
                )
                .ok();
                let Some(typecheck::representation::Repr::Array { length, .. }) = repr else {
                    self.emit_symptom(
                        "unresolved-fixed-array-construction",
                        "fixed-array construction reached codegen without array representation"
                            .to_string(),
                    );
                    return None;
                };
                let init = self.lower_typed_expr(*init, block, symtab)?;
                let array_ty = self.ty_to_mlir(expr_ref.ty);
                let mut value = block.append_op(self.mlir.llvm_undef(array_ty, self.location()));
                for index in 0..length {
                    value = block.append_op(self.mlir.llvm_insertvalue(
                        value,
                        init,
                        i64::try_from(index).ok()?,
                        self.location(),
                    ));
                }
                Some(value)
            }
            typecheck::TypedExprKind::TakePtr(init) | typecheck::TypedExprKind::Ref(init) => {
                self.lower_typed_place(*init, block, symtab)
            }
            typecheck::TypedExprKind::AggregateGet { base, index } => {
                let base_ty = typed_ast.expr(*base)?.ty;
                if matches!(base_ty, Ty::ResultRow(_)) {
                    return self
                        .lower_typed_values(*base, block, symtab)?
                        .values()
                        .get(*index)
                        .copied();
                }
                let base_val = self.lower_typed_expr(*base, block, symtab)?;
                let base_val = if let Some(pointee_ty) = self.pointee_ty_for_reference_like(base_ty)
                {
                    block.load_typed(
                        self,
                        base_val,
                        self.ty_to_mlir(&pointee_ty),
                        self.location(),
                    )?
                } else {
                    base_val
                };
                // Extract the field at the given index from the lowered aggregate.
                // Use the expression's own type as the result type.
                let result_ty = self.ty_to_mlir(expr_ref.ty);
                Some(block.append_op(self.mlir.llvm_extractvalue(
                    base_val,
                    *index as i64,
                    result_ty,
                    self.location(),
                )))
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
                if let Some((element_ty, _)) = typecheck::representation::Repr::array_parts(
                    base_ty,
                    Some(typed_ast.type_registry),
                ) {
                    let base_ptr = self.lower_typed_place(*base, block, symtab)?;
                    let element_ptr = block.gep_typed(
                        self,
                        base_ptr,
                        self.ty_to_mlir(base_ty),
                        &[],
                        &[0, *index as i32],
                        self.location(),
                    )?;
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
            typecheck::TypedExprKind::BufGet { buf, index } => {
                let (element_ptr, element_ty) =
                    self.lower_buffer_element(*buf, *index, block, symtab)?;
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
                    self.lower_buffer_element(*buf, *index, block, symtab)?;
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
                    let array_ty = self.ty_to_mlir(expr_ref.ty);
                    let mut value =
                        block.append_op(self.mlir.llvm_undef(array_ty, self.location()));
                    for (index, item) in
                        items.iter().take(usize::try_from(length).ok()?).enumerate()
                    {
                        let element = self.lower_typed_expr(*item, block, symtab)?;
                        value = block.append_op(self.mlir.llvm_insertvalue(
                            value,
                            element,
                            i64::try_from(index).ok()?,
                            self.location(),
                        ));
                    }
                    return Some(value);
                }
                let vals: Vec<Value<'c, 'c>> = items
                    .iter()
                    .filter_map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect();
                vals.first().copied().or_else(|| {
                    self.is_empty_product(expr_ref.ty)
                        .then(|| block.unit_value(self, self.location()))
                })
            }
            typecheck::TypedExprKind::If(if_expr) => {
                self.lower_typed_if(if_expr, expr_ref.ty, block, symtab)
            }
            typecheck::TypedExprKind::When(when_expr) => {
                if let Some(cv) = &expr_ref.facts.const_facts.const_value {
                    self.lower_const_value(block, cv, expr_ref.ty)
                } else {
                    self.lower_typed_when(when_expr, expr_ref.ty, block, symtab)
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

    fn lower_synthetic_record_field(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.program;
        let index = expr_id.as_usize().checked_sub(1)?;

        let typecheck::TypedExprKind::FnCall { target, args, .. } =
            typed_ast.exprs.kind.get(index)?
        else {
            return None;
        };
        if !args.as_ref().is_none_or(Vec::is_empty) {
            return None;
        }

        let slot = symtab.get(&target.0)?;
        let field_ty = typed_ast.expr(expr_id)?.ty;
        let resolved_slot = self
            .program
            .type_registry
            .resolved_definition_for_type(&slot.ty);
        let field_index = if let Ty::Record { fields, .. } = &resolved_slot {
            let mut matches = fields
                .iter()
                .enumerate()
                .filter(|(_, (_, ty))| ty.as_ref() == field_ty);
            let (index, _) = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            index
        } else {
            return None;
        };
        let aggregate = if slot.is_slot {
            block.load_typed(self, slot.value, self.ty_to_mlir(&slot.ty), self.location())?
        } else {
            slot.value
        };
        let result_ty = self.ty_to_mlir(field_ty);
        Some(block.append_op(self.mlir.llvm_extractvalue(
            aggregate,
            field_index as i64,
            result_ty,
            self.location(),
        )))
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
                let operation = match op {
                    typecheck::typed::IntrinsicOp::BitsAdd => "arith.addi",
                    typecheck::typed::IntrinsicOp::BitsSubtract => "arith.subi",
                    typecheck::typed::IntrinsicOp::BitsMultiply => "arith.muli",
                    typecheck::typed::IntrinsicOp::BitsSignedDivide => "arith.divsi",
                    typecheck::typed::IntrinsicOp::BitsUnsignedDivide => "arith.divui",
                    typecheck::typed::IntrinsicOp::BitsSignedRemainder => "arith.remsi",
                    typecheck::typed::IntrinsicOp::BitsUnsignedRemainder => "arith.remui",
                    typecheck::typed::IntrinsicOp::BitsAnd => "arith.andi",
                    typecheck::typed::IntrinsicOp::BitsOr => "arith.ori",
                    typecheck::typed::IntrinsicOp::BitsXor => "arith.xori",
                    typecheck::typed::IntrinsicOp::BitsShiftLeft => "arith.shli",
                    typecheck::typed::IntrinsicOp::BitsShiftRightLogical => "arith.shrui",
                    typecheck::typed::IntrinsicOp::BitsShiftRightArithmetic => "arith.shrsi",
                    _ => {
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
                let name = self.register_string(text);
                self.addressof_string_global(block, &name, self.location())
            }
            typecheck::TypedExprKind::FnCall { target, args, .. }
                if args.as_ref().is_none_or(Vec::is_empty) =>
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
                block.gep_typed(
                    self,
                    base_ptr,
                    self.ty_to_mlir(base_ty),
                    &[],
                    &[0, *index as i32],
                    self.location(),
                )
            }
            typecheck::TypedExprKind::BufGet { buf, index } => self
                .lower_buffer_element(*buf, *index, block, symtab)
                .map(|(address, _)| address),
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
        let pointer = block.alloca_typed(self.mlir, self.ty_to_mlir(&binding.ty), self.location());
        block.store_typed(self, pointer, binding.value, self.location())?;
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
