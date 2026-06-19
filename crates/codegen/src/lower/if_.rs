use crate::prelude::*;
use internment::Intern;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Lower a typed-arena `if` expression.
    pub(crate) fn lower_typed_if(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();
        let typed_ast = self.typed_ast?;

        // 1. Lower the condition (always a pattern match)
        let subject_val = self.lower_typed_expr(if_expr.subject, block, symtab)?;
        let surface_name = if_expr.pattern.value.surface_mangle_name();
        let variant_name = Intern::<String>::from_ref(surface_name);
        let (_, expected_disc, _) = match self.lookup_variant(variant_name) {
            Some(v) => v,
            None => {
                self.emit_internal(format!("Unknown variant '{surface_name}' in if pattern"));
                return None;
            }
        };
        let disc = block.append_op(self.mlir.llvm_extractvalue(
            subject_val,
            0,
            self.mlir.i64(),
            self.location(),
        ));
        let expected_val = block.const_i64(self.mlir, expected_disc as i64, self.location());
        let cond_val = block.append_op(self.mlir.build_cmpi(
            Predicates::EQ,
            disc,
            expected_val,
            self.location(),
        ));

        // 2. Determine return type from typed AST
        let ret_ty = match if_expr.ret {
            Some(ret_id) => typed_ast
                .expr(ret_id)
                .map(|e| e.ty.clone())
                .unwrap_or(Ty::i64()),
            None => Ty::i64(),
        };
        let result_mlir = self.ty_to_mlir(&ret_ty);

        // 3. Build then region
        let then_region = Region::new();
        {
            let blk = Block::new(&[]);
            then_region.append_block(blk);
            let blk_ref = then_region.first_block().unwrap();
            let mut inner_symtab = symtab.clone();

            self.bind_pattern_payload_fields(
                &blk_ref,
                &if_expr.pattern.value,
                subject_val,
                &mut inner_symtab,
            );

            for stmt_id in &if_expr.stmts {
                self.lower_typed_expr(*stmt_id, &blk_ref, &mut inner_symtab)?;
            }

            let ret_val = match if_expr.ret {
                Some(ret_id) => self.lower_typed_expr(ret_id, &blk_ref, &mut inner_symtab)?,
                None => blk_ref.unit_value(self, loc),
            };
            blk_ref.append_operation(scf_dialect::r#yield(&[ret_val], loc));
        }

        // 4. Build else region (placeholder — shouldn't execute with early return)
        let else_region = Region::new();
        {
            let blk = Block::new(&[]);
            else_region.append_block(blk);
            let blk_ref = else_region.first_block().unwrap();
            blk_ref.append_operation(scf_dialect::r#yield(&[blk_ref.unit_value(self, loc)], loc));
        }

        // 5. Emit scf.if
        let if_op = block.append_operation(scf_dialect::r#if(
            cond_val,
            &[result_mlir],
            then_region,
            else_region,
            loc,
        ));
        Some(if_op.result(0).unwrap().into())
    }
}
