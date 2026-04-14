use crate::{lower_function, prelude::*, ty_to_mlir};
use internment::Intern;
use typeck::TyInfer;

impl<'c> Lower<'c> for Bind {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        match &self.value() {
            BindValue::Body { exprs: _, ret: _ } => {
                let func_op = lower_function(ctx, &self.name(), self)?;
                block.append_operation(func_op);

                // Return a placeholder value (TODO: consider returning function reference)
                Some(block.const_i64(ctx.mlir, 0))
            }
            BindValue::Expr(expr) => {
                let name_str = self.name().as_str().to_string();
                if self.is_const {
                    // Const bind (`:=`): direct SSA value in symtab — no alloca.
                    let value = expr.lower(ctx, block, symtab)?;
                    symtab.insert(name_str.clone(), value);
                    let ty =
                        expr.infer_ty(&ctx.ty_env.infer_env(&std::collections::HashMap::new()));
                    ctx.var_types.borrow_mut().insert(name_str, ty);
                    Some(value)
                } else {
                    let loc = ctx.location();
                    if ctx.mutable_slots.borrow().contains(&name_str) {
                        // Rebind (`:`) of an existing mutable variable — store new value.
                        let ptr = match symtab.get(&name_str).copied() {
                            Some(p) => p,
                            None => {
                                ctx.emit_internal(format!(
                                    "mutable slot '{name_str}' not found in symtab"
                                ));
                                return None;
                            }
                        };
                        let new_val = expr.lower(ctx, block, symtab)?;
                        block.store_typed(ctx, ptr, new_val, loc)?;
                        Some(block.const_i64(ctx.mlir, 0))
                    } else {
                        // First mutable bind (`:`) — alloca + store.
                        // Build locals from var_types so infer_expr can resolve base types
                        // (e.g., elem type of arrays in TupleGet expressions).
                        let locals: std::collections::HashMap<Intern<String>, typeck::Ty> = ctx
                            .var_types
                            .borrow()
                            .iter()
                            .map(|(k, v)| (Intern::<String>::from_ref(k), v.clone()))
                            .collect();
                        let ty = expr.infer_ty(&ctx.ty_env.infer_env(&locals));
                        let elem_mlir_ty = ty_to_mlir(&ty, ctx.mlir);
                        let slot = block.alloca_typed(ctx.mlir, elem_mlir_ty, loc);
                        let init_val = expr.lower(ctx, block, symtab)?;
                        block.store_typed(ctx, slot, init_val, loc)?;
                        symtab.insert(name_str.clone(), slot);
                        ctx.var_types.borrow_mut().insert(name_str.clone(), ty);
                        ctx.mutable_slots.borrow_mut().insert(name_str);
                        Some(slot)
                    }
                }
            }
            BindValue::Extern => {
                let func_op = lower_function(ctx, &self.name(), self)?;
                block.append_operation(func_op);
                Some(block.const_i64(ctx.mlir, 0))
            }
        }
    }
}
