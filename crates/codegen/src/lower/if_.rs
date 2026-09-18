use crate::prelude::*;
use melior::ir::operation::OperationLike;
use typecheck::ExprId;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
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
            if let Some(typecheck::TypedExprKind::If(if_expr)) = typed.exprs.kind_of(*expr_id)
                && if_expr.place_joins.is_empty()
            {
                return self.lower_typed_returning_if(
                    if_expr,
                    &exprs[index + 1..],
                    ret,
                    result_ty,
                    block,
                    symtab,
                );
            }
            self.lower_typed_values(*expr_id, block, symtab);
        }
        match ret {
            Some(ret) => self.lower_typed_values(ret, block, symtab),
            None if self.is_empty_product(result_ty) => Some(LoweredValue::new(Vec::new())),
            None => None,
        }
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
        let loc = self.location();
        let condition = self.lower_typed_condition(&if_expr.condition, block, symtab)?;

        let then_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block()?;
        let mut then_symtab = symtab.clone();
        self.bind_typed_condition_patterns(&if_expr.condition, &then_block, &mut then_symtab)?;
        let (then_stmts, then_ret) = if_expr
            .stmts
            .split_last()
            .map(|(ret, stmts)| (stmts, Some(*ret)))
            .unwrap_or((&[], None));
        let then_value = self.lower_typed_returning_sequence(
            then_stmts,
            then_ret,
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
        let (else_exprs, else_ret) = if let Some(ret) = if_expr.ret {
            (&[][..], Some(ret))
        } else {
            (continuation, continuation_ret)
        };
        let else_value = self.lower_typed_returning_sequence(
            else_exprs,
            else_ret,
            result_ty,
            &else_block,
            &mut else_symtab,
        )?;
        else_block.append_operation(scf_dialect::r#yield(else_value.values(), loc));

        let result_types = self.result_types_to_mlir(result_ty);

        let if_op = block.append_operation(scf_dialect::r#if(
            condition,
            &result_types,
            then_region,
            else_region,
            loc,
        ));
        Some(LoweredValue::new(
            if_op.results().map(Value::from).collect(),
        ))
    }

    pub(crate) fn lower_typed_if_values(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        let loc = self.location();
        let condition = self.lower_typed_condition(&if_expr.condition, block, symtab)?;
        let value_types = self.result_types_to_mlir(expr_ty);
        let typed = self.program;
        let joins = if_expr
            .place_joins
            .iter()
            .filter(|version| {
                let place = typed.place_versions[version.0 as usize].place;
                symtab
                    .get(&typed.places[place.0 as usize].name)
                    .is_none_or(|binding| !binding.is_slot)
            })
            .map(|version| {
                let place = typed.place_versions[version.0 as usize].place;
                let place = &typed.places[place.0 as usize];
                let binding = symtab.get(&place.name)?.clone();
                let incoming = if binding.is_slot {
                    block.load_typed(self, binding.value, self.ty_to_mlir(&binding.ty), loc)?
                } else {
                    binding.value
                };
                Some((place.name, place.ty.clone(), incoming))
            })
            .collect::<Option<Vec<_>>>()?;

        let then_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block()?;
        let mut then_symbols = symtab.clone();
        self.bind_typed_condition_patterns(&if_expr.condition, &then_block, &mut then_symbols)?;
        for statement in &if_expr.stmts {
            self.lower_typed_values(*statement, &then_block, &mut then_symbols)?;
        }
        let value = match if_expr.ret {
            Some(ret) => self.lower_typed_values(ret, &then_block, &mut then_symbols)?,
            None if value_types.is_empty() => LoweredValue::new(Vec::new()),
            None => return None,
        };
        let mut yielded = value.into_values();
        for (name, ty, _) in &joins {
            let binding = then_symbols.get(name)?;
            yielded.push(if binding.is_slot {
                then_block.load_typed(self, binding.value, self.ty_to_mlir(ty), loc)?
            } else {
                binding.value
            });
        }
        then_block.append_operation(scf_dialect::r#yield(&yielded, loc));

        let else_region = Region::new();
        let else_block = Block::new(&[]);
        else_region.append_block(else_block);
        let else_block = else_region.first_block()?;
        let mut yielded = value_types
            .iter()
            .map(|ty| else_block.append_op(self.mlir.llvm_undef(*ty, loc)))
            .collect::<Vec<_>>();
        yielded.extend(joins.iter().map(|(_, _, incoming)| *incoming));
        else_block.append_operation(scf_dialect::r#yield(&yielded, loc));

        let mut result_types = value_types.clone();
        result_types.extend(joins.iter().map(|(_, ty, _)| self.ty_to_mlir(ty)));
        let operation = block.append_operation(scf_dialect::r#if(
            condition,
            &result_types,
            then_region,
            else_region,
            loc,
        ));
        for (index, (name, ty, _)) in joins.iter().enumerate() {
            symtab.assign(
                name,
                crate::Slot {
                    value: operation.result(value_types.len() + index).ok()?.into(),
                    ty: ty.clone(),
                    is_slot: false,
                },
            );
        }
        Some(LoweredValue::new(
            operation
                .results()
                .take(value_types.len())
                .map(Value::from)
                .collect(),
        ))
    }

    pub(crate) fn lower_typed_if(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let values = self.lower_typed_if_values(if_expr, expr_ty, block, symtab)?;
        if self.is_empty_product(expr_ty) {
            Some(block.unit_value(self, self.location()))
        } else {
            values.single()
        }
    }
}
