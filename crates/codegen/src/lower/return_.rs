use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use melior::ir::operation::OperationLike;
use typecheck::ty::Ty;
use typecheck::{ExprId, TypedCondition, TypedExprKind};

#[derive(Clone)]
pub(super) struct ReturnStorage<'c> {
    pub(super) flag: Value<'c, 'c>,
    values: Vec<(Value<'c, 'c>, Type<'c>)>,
    result_ty: Ty,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_local_callable(
        &self,
        call: ExprId,
        definition: ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let TypedExprKind::FnCall { args, .. } = self.program.exprs.kind_of(call)? else {
            return None;
        };
        let mut symbols = symtab.clone();
        for ((name, ty), argument) in self
            .program
            .local_parameters(definition)
            .zip(args.as_deref().unwrap_or_default())
        {
            let source = self.program.expr(*argument)?.ty;
            let value = self.lower_typed_expr(*argument, block, symtab)?;
            let value = if IntegerType::try_from(value.r#type()).is_ok()
                && let target = self.ty_to_mlir(ty)
                && IntegerType::try_from(target).is_ok()
            {
                self.convert_integer_value(value, source, target, block)?
            } else {
                value
            };
            symbols.insert(name, value, ty.clone(), false);
        }
        let TypedExprKind::Bind { stmts, body, .. } = self.program.exprs.kind_of(definition)?
        else {
            return None;
        };
        let previous = self.return_storage.take();
        let subjects = self.when_subjects.take();
        let operands = self.operand_values.take();
        let result = self.lower_function_returns(
            stmts,
            Some(*body),
            self.program.expr(call)?.ty,
            block,
            &mut symbols,
        );
        self.when_subjects.replace(subjects);
        self.operand_values.replace(operands);
        self.return_storage.replace(previous);
        result
    }

    pub(super) fn has_returning_operands(&self, expression: ExprId) -> bool {
        if self.return_storage.borrow().is_none()
            || self.active_operands.borrow().contains(&expression)
        {
            return false;
        }
        let Some(kind) = self.program.exprs.kind_of(expression) else {
            return false;
        };
        if matches!(
            kind,
            TypedExprKind::Block { .. }
                | TypedExprKind::Return { .. }
                | TypedExprKind::Bind { .. }
                | TypedExprKind::When(_)
                | TypedExprKind::If(_)
                | TypedExprKind::Loop(_)
        ) {
            return false;
        }
        let mut operands = Vec::new();
        let _ = typecheck::typed::typed_walk_expr_children(kind, &mut |child| {
            operands.push(child);
            std::ops::ControlFlow::Continue(())
        });
        self.contains_explicit_return(&operands)
    }

    fn unless_returned(
        &self,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        lower: impl FnOnce(&BlockRef<'c, 'c>, &mut ScopedSymbolTable<'c>) -> Option<LoweredValue<'c>>,
    ) -> Option<LoweredValue<'c>> {
        let storage = self.return_storage.borrow().clone()?;
        let loc = self.location();
        let returned = block.load_typed(self, storage.flag, self.mlir.i1(), loc)?;
        let types = self.result_types_to_mlir(ty);
        let then_region = Region::new();
        then_region.append_block(Block::new(&[]));
        let then_block = then_region.first_block()?;
        let unavailable = types
            .iter()
            .map(|ty| then_block.append_op(self.mlir.llvm_undef(*ty, loc)))
            .collect::<Vec<_>>();
        then_block.append_operation(scf_dialect::r#yield(&unavailable, loc));
        let else_region = Region::new();
        else_region.append_block(Block::new(&[]));
        let else_block = else_region.first_block()?;
        let values = lower(&else_block, &mut symtab.clone())?;
        else_block.append_operation(scf_dialect::r#yield(values.values(), loc));
        let operation = block.append_scf_if(returned, &types, then_region, else_region, loc);
        Some(LoweredValue::new(
            operation.results().map(Value::from).collect(),
        ))
    }

    pub(super) fn lower_returning_operands(
        &self,
        expression: ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let kind = self.program.exprs.kind_of(expression)?;
        let mut operands = Vec::new();
        let _ = typecheck::typed::typed_walk_expr_children(kind, &mut |child| {
            if !operands.contains(&child) {
                operands.push(child);
            }
            std::ops::ControlFlow::Continue(())
        });
        let previous = self.operand_values.borrow().clone();
        self.active_operands.borrow_mut().insert(expression);
        let result = (|| {
            for operand in operands {
                let ty = self.program.expr(operand)?.ty;
                let values = self.unless_returned(ty, block, symtab, |block, symbols| {
                    self.lower_typed_values(operand, block, symbols)
                })?;
                self.operand_values.borrow_mut().insert(operand, values);
            }
            self.unless_returned(
                self.program.expr(expression)?.ty,
                block,
                symtab,
                |block, symbols| self.lower_typed_values(expression, block, symbols),
            )
        })();
        self.active_operands.borrow_mut().remove(&expression);
        self.operand_values.replace(previous);
        result
    }

    fn contains_explicit_return(&self, expressions: &[ExprId]) -> bool {
        let mut pending = expressions.to_vec();
        let mut visited = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) || self.program.is_local_callable(id) {
                continue;
            }
            let Some(kind) = self.program.exprs.kind_of(id) else {
                continue;
            };
            if matches!(kind, TypedExprKind::Return { .. } | TypedExprKind::If(_)) {
                return true;
            }
            let _ = typecheck::typed::typed_walk_expr_children(kind, &mut |child| {
                pending.push(child);
                std::ops::ControlFlow::Continue(())
            });
        }
        false
    }

    pub(super) fn lower_block_values(
        &self,
        statements: &[ExprId],
        value: Option<ExprId>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        for (index, statement) in statements.iter().enumerate() {
            self.lower_typed_values(*statement, block, symtab)?;
            if self.return_storage.borrow().is_some()
                && self.contains_explicit_return(&[*statement])
            {
                return self.lower_block_continuation(
                    &statements[index + 1..],
                    value,
                    ty,
                    block,
                    symtab,
                );
            }
        }
        match value {
            Some(value) if !typecheck::expression_returns(self.program, value) => {
                self.lower_return_values(value, ty, block, symtab)
            }
            Some(value) => {
                self.lower_typed_values(value, block, symtab)?;
                Some(LoweredValue::new(
                    self.result_types_to_mlir(ty)
                        .iter()
                        .map(|ty| block.append_op(self.mlir.llvm_undef(*ty, self.location())))
                        .collect(),
                ))
            }
            None => Some(LoweredValue::new(Vec::new())),
        }
    }

    fn lower_block_continuation(
        &self,
        statements: &[ExprId],
        value: Option<ExprId>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        self.unless_returned(ty, block, symtab, |block, symbols| {
            self.lower_block_values(statements, value, ty, block, symbols)
        })
    }

    pub(super) fn store_function_return(
        &self,
        value: Option<ExprId>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        let storage = self.return_storage.borrow().clone()?;
        let values = match value {
            Some(value) => self.lower_return_values(value, &storage.result_ty, block, symtab)?,
            None => LoweredValue::new(Vec::new()),
        };
        if values.values().len() != storage.values.len() {
            return None;
        }
        let store = |block: &BlockRef<'c, 'c>| {
            for (value, (pointer, _)) in values.values().iter().zip(&storage.values) {
                block.store_typed(self, *pointer, *value, self.location())?;
            }
            let one = block.const_int(self.mlir, self.mlir.i1(), 1, self.location());
            block.store_typed(self, storage.flag, one, self.location())?;
            Some(())
        };
        if value.is_some_and(|value| self.contains_explicit_return(&[value])) {
            self.unless_returned(&Ty::Unit, block, symtab, |block, _| {
                store(block)?;
                Some(LoweredValue::new(Vec::new()))
            })?;
        } else {
            store(block)?;
        }
        Some(())
    }

    pub(crate) fn lower_typed_returning_sequence(
        &self,
        exprs: &[ExprId],
        ret: Option<ExprId>,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let typed = self.program;
        for (index, expr_id) in exprs.iter().enumerate() {
            if let Some(typecheck::TypedExprKind::If(if_expr)) = typed.exprs.kind_of(*expr_id) {
                return self.lower_typed_returning_if(
                    if_expr,
                    &exprs[index + 1..],
                    ret,
                    result_ty,
                    block,
                    symtab,
                );
            }
            self.lower_typed_values(*expr_id, block, symtab)?;
        }
        match ret {
            Some(ret) => self.lower_return_values(ret, result_ty, block, symtab),
            None if self.is_empty_product(result_ty) => Some(LoweredValue::new(Vec::new())),
            None => None,
        }
    }

    pub(super) fn lower_return_values(
        &self,
        expression: ExprId,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let source = self.program.expr(expression)?.ty;
        let result_types = self.result_types_to_mlir(result_ty);
        if let Some(TypedExprKind::ResultRow(items)) = self.program.exprs.kind_of(expression)
            && (!matches!(source, Ty::Sequence(fields) if items.len() == fields.len()
                && items.iter().zip(fields).all(|(item, field)| self.program.exprs.ty_of(*item) == Some(&*field.ty)))
                || items.len() != result_types.len())
        {
            // codegen-return-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ReturnTypeMismatch),
                "returned row does not match its typed result fields".into(),
            );
            return None;
        }
        let mut values = self
            .lower_typed_values(expression, block, symtab)?
            .into_values();
        if values.len() != result_types.len() {
            // codegen-return-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ReturnTypeMismatch),
                "returned result count differs from its callable signature".into(),
            );
            return None;
        }
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.convert_return_value(
                *value,
                Self::return_field_type(source, index)?,
                Self::return_field_type(result_ty, index)?,
                block,
            )?;
        }
        Some(LoweredValue::new(values))
    }

    fn return_field_type(ty: &Ty, index: usize) -> Option<&Ty> {
        match ty {
            Ty::Sequence(fields) => fields.get(index).map(|field| field.ty.as_ref()),
            ty => Some(ty),
        }
    }

    fn convert_return_value(
        &self,
        value: Value<'c, 'c>,
        source: &Ty,
        target: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let target_mlir = self.ty_to_mlir(target);
        if value.r#type() == target_mlir {
            return Some(value);
        }
        let (Ty::Sequence(sources), Ty::Sequence(targets)) = (source, target) else {
            return self.convert_integer_value(value, source, target_mlir, block);
        };
        if sources.len() != targets.len() {
            // codegen-return-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ReturnTypeMismatch),
                "nested returned sequence differs from its callable signature".into(),
            );
            return None;
        }
        let values = sources
            .iter()
            .zip(targets)
            .enumerate()
            .map(|(index, (source, target))| {
                let field = block.append_op(self.mlir.llvm_extractvalue(
                    value,
                    i64::try_from(index).ok()?,
                    self.ty_to_mlir(&source.ty),
                    self.location(),
                ));
                self.convert_return_value(field, &source.ty, &target.ty, block)
            });
        self.insert_aggregate(block, target_mlir, values)
    }

    fn lower_typed_returning_if(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        continuation: &[ExprId],
        continuation_ret: Option<ExprId>,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        self.require_if_without_joins(if_expr)?;
        let loc = self.location();
        let condition = self.lower_typed_condition(&if_expr.condition, block, symtab)?;

        let then_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block()?;
        let mut then_symtab = symtab.clone();
        self.bind_typed_condition_patterns(&condition, &then_block, &mut then_symtab)?;
        let then_value = self.lower_typed_returning_sequence(
            &if_expr.stmts,
            if_expr.ret,
            result_ty,
            &then_block,
            &mut then_symtab,
        )?;
        then_block.append_operation(scf_dialect::r#yield(then_value.values(), loc));

        let else_region = Region::new();
        let else_block = Block::new(&[]);
        else_region.append_block(else_block);
        let else_block = else_region.first_block()?;
        let mut else_symtab = symtab.clone();
        let else_value = self.lower_typed_returning_sequence(
            continuation,
            continuation_ret,
            result_ty,
            &else_block,
            &mut else_symtab,
        )?;
        else_block.append_operation(scf_dialect::r#yield(else_value.values(), loc));

        let result_types = self.result_types_to_mlir(result_ty);

        let if_op = block.append_scf_if(
            condition.value,
            &result_types,
            then_region,
            else_region,
            loc,
        );
        Some(LoweredValue::new(
            if_op.results().map(Value::from).collect(),
        ))
    }

    pub(super) fn lower_function_returns(
        &self,
        exprs: &[ExprId],
        ret: Option<ExprId>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        if !self.contains_explicit_return(exprs)
            && !self.contains_explicit_return(&ret.into_iter().collect::<Vec<_>>())
        {
            return self.lower_typed_returning_sequence(exprs, ret, ty, block, symtab);
        }
        let loc = self.location();
        let flag = self.allocate_stack_slot(self.mlir.i1())?;
        let zero = block.const_int(self.mlir, self.mlir.i1(), 0, loc);
        block.store_typed(self, flag, zero, loc)?;
        let values = self
            .result_types_to_mlir(ty)
            .into_iter()
            .map(|ty| Some((self.allocate_stack_slot(ty)?, ty)))
            .collect::<Option<Vec<_>>>()?;
        let storage = ReturnStorage {
            flag,
            values,
            result_ty: ty.clone(),
        };
        let names: Vec<_> = symtab
            .scopes
            .iter()
            .flat_map(|scope| scope.keys().copied())
            .collect();
        for name in names {
            self.materialize_named_place(name, block, symtab)?;
        }
        let previous = self.return_storage.replace(Some(storage.clone()));
        let result = (|| {
            let value = self.lower_block_values(exprs, ret, ty, block, symtab)?;
            let returned = block.load_typed(self, storage.flag, self.mlir.i1(), loc)?;
            let then_region = Region::new();
            then_region.append_block(Block::new(&[]));
            then_region
                .first_block()?
                .append_operation(scf_dialect::r#yield(&[], loc));
            let else_region = Region::new();
            else_region.append_block(Block::new(&[]));
            let else_block = else_region.first_block()?;
            let always_returns = ret
                .is_some_and(|id| typecheck::expression_returns(self.program, id))
                || exprs
                    .iter()
                    .any(|id| typecheck::expression_returns(self.program, *id));
            if !always_returns {
                if value.values().len() != storage.values.len() {
                    return None;
                }
                for (value, (pointer, _)) in value.values().iter().zip(&storage.values) {
                    else_block.store_typed(self, *pointer, *value, loc)?;
                }
            }
            else_block.append_operation(scf_dialect::r#yield(&[], loc));
            block.append_scf_if(returned, &[], then_region, else_region, loc);
            Some(())
        })();
        self.return_storage.replace(previous);
        result?;
        Some(LoweredValue::new(
            storage
                .values
                .iter()
                .map(|(pointer, ty)| block.load_typed(self, *pointer, *ty, loc))
                .collect::<Option<Vec<_>>>()?,
        ))
    }

    pub(super) fn lower_loop_statements(
        &self,
        exprs: &[ExprId],
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<()> {
        if let Some(storage) = self.return_storage.borrow().clone() {
            let loc = self.location();
            let returned = block.load_typed(self, storage.flag, self.mlir.i1(), loc)?;
            let then_region = Region::new();
            then_region.append_block(Block::new(&[]));
            then_region
                .first_block()?
                .append_operation(scf_dialect::r#yield(&[], loc));
            let else_region = Region::new();
            else_region.append_block(Block::new(&[]));
            let else_block = else_region.first_block()?;
            self.lower_block_values(exprs, None, &Ty::Unit, &else_block, &mut symtab.clone())?;
            else_block.append_operation(scf_dialect::r#yield(&[], loc));
            block.append_scf_if(returned, &[], then_region, else_region, loc);
            Some(())
        } else {
            for expr in exprs {
                self.lower_typed_values(*expr, block, symtab)?;
            }
            Some(())
        }
    }

    pub(super) fn lower_loop_condition<'p>(
        &self,
        condition: &'p TypedCondition,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<super::condition::LoweredCondition<'p, 'c>> {
        let Some(storage) = self.return_storage.borrow().clone() else {
            return self.lower_typed_condition(condition, block, symtab);
        };
        let loc = self.location();
        let returned = block.load_typed(self, storage.flag, self.mlir.i1(), loc)?;
        let then_region = Region::new();
        then_region.append_block(Block::new(&[]));
        let then_block = then_region.first_block()?;
        let zero = then_block.const_int(self.mlir, self.mlir.i1(), 0, loc);
        then_block.append_operation(scf_dialect::r#yield(&[zero], loc));
        let else_region = Region::new();
        else_region.append_block(Block::new(&[]));
        let else_block = else_region.first_block()?;
        let mut condition = self.lower_typed_condition(condition, &else_block, symtab)?;
        else_block.append_operation(scf_dialect::r#yield(&[condition.value], loc));
        let operation =
            block.append_scf_if(returned, &[self.mlir.i1()], then_region, else_region, loc);
        condition.value = operation.result(0).ok()?.into();
        Some(condition)
    }
}
