use crate::prelude::*;
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
    ) -> Option<Value<'c, 'c>> {
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
            self.lower_typed_expr(*expr_id, block, symtab);
        }
        ret.and_then(|ret_id| self.lower_typed_expr(ret_id, block, symtab))
    }

    fn lower_typed_returning_if(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        continuation: &[ExprId],
        continuation_ret: Option<ExprId>,
        result_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();
        let condition = self.lower_typed_condition(&if_expr.condition, block, symtab)?;

        let then_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block()?;
        let mut then_symtab = symtab.clone();
        self.bind_typed_condition_patterns(&if_expr.condition, &then_block, &mut then_symtab)?;
        let (then_ret, then_stmts) = if_expr.stmts.split_last()?;
        let then_value = self.lower_typed_returning_sequence(
            then_stmts,
            Some(*then_ret),
            result_ty,
            &then_block,
            &mut then_symtab,
        )?;
        then_block.append_operation(scf_dialect::r#yield(&[then_value], loc));

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
        else_block.append_operation(scf_dialect::r#yield(&[else_value], loc));

        let if_op = block.append_operation(scf_dialect::r#if(
            condition,
            &[self.ty_to_mlir(result_ty)],
            then_region,
            else_region,
            loc,
        ));
        Some(if_op.result(0).ok()?.into())
    }

    /// Lower a typed-arena `if` expression.
    pub(crate) fn lower_typed_if(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();
        let cond_val = self.lower_typed_condition(&if_expr.condition, block, symtab)?;
        let result_mlir = self.ty_to_mlir(expr_ty);
        let typed = self.program;
        let joins = if_expr
            .place_joins
            .iter()
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

        // 3. Build then region
        let then_region = Region::new();
        {
            let blk = Block::new(&[]);
            then_region.append_block(blk);
            let blk_ref = then_region.first_block().unwrap();
            let mut inner_symtab = symtab.clone();

            self.bind_typed_condition_patterns(&if_expr.condition, &blk_ref, &mut inner_symtab)?;

            for stmt_id in &if_expr.stmts {
                self.lower_typed_expr(*stmt_id, &blk_ref, &mut inner_symtab)?;
            }

            let ret_val = match if_expr.ret {
                Some(ret_id) => self.lower_typed_expr(ret_id, &blk_ref, &mut inner_symtab)?,
                None => blk_ref.unit_value(self, loc),
            };
            let mut yielded = vec![ret_val];
            for (name, ty, _) in &joins {
                let binding = inner_symtab.get(name)?.clone();
                yielded.push(if binding.is_slot {
                    blk_ref.load_typed(self, binding.value, self.ty_to_mlir(ty), loc)?
                } else {
                    binding.value
                });
            }
            blk_ref.append_operation(scf_dialect::r#yield(&yielded, loc));
        }

        // 4. Build else region (placeholder — shouldn't execute with early return)
        let else_region = Region::new();
        {
            let blk = Block::new(&[]);
            else_region.append_block(blk);
            let blk_ref = else_region.first_block().unwrap();
            let mut yielded = vec![blk_ref.unit_value(self, loc)];
            yielded.extend(joins.iter().map(|(_, _, incoming)| *incoming));
            blk_ref.append_operation(scf_dialect::r#yield(&yielded, loc));
        }

        // 5. Emit scf.if
        let mut result_types = vec![result_mlir];
        result_types.extend(joins.iter().map(|(_, ty, _)| self.ty_to_mlir(ty)));
        let if_op = block.append_operation(scf_dialect::r#if(
            cond_val,
            &result_types,
            then_region,
            else_region,
            loc,
        ));
        for (index, (name, ty, _)) in joins.iter().enumerate() {
            let value = if_op.result(index + 1).ok()?.into();
            symtab.assign(
                name,
                crate::Slot {
                    value,
                    ty: ty.clone(),
                    is_slot: false,
                },
            );
        }
        Some(if_op.result(0).unwrap().into())
    }
}
