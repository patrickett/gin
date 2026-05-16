use crate::{lower_function, prelude::*, ty_to_mlir};
use ast::ty::Ty;
use ast::{TyInfer, Typed, flow::ConstValue};

impl<'c> Lower<'c> for Bind {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        match &self.value() {
            BindValue::Body { exprs: _, ret: _ } => {
                let func_op = lower_function(ctx, &self.name(), self)?;
                block.append_operation(func_op);

                // Return a placeholder value (TODO: consider returning function reference)
                Some(block.const_i64(ctx.mlir, 0))
            }
            BindValue::Expr(expr) => {
                let name = self.name();
                // Infer the target type — uses return_tag for typed binds (e.g.
                // `level LogLevel: 'debug'` resolves to ConstUnion, not Str).
                let ty = self.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));

                // Lower the value. If the target type is a ConstUnion and the
                // expression is a literal, emit just the discriminant integer
                // instead of a full base-type value (e.g. i8 not Str).
                let init_val = if let Ty::ConstUnion { values, .. } = &ty {
                    lower_const_union_literal(ctx, block, expr, values)
                        .or_else(|| expr.lower(ctx, block, symtab))
                } else {
                    expr.lower(ctx, block, symtab)
                }?;

                if self.is_const {
                    // Const bind (`:=`): direct SSA value in symtab — no alloca.
                    symtab.insert(name.as_str().to_string(), init_val);
                    ctx.var_types.borrow_mut().insert(name, ty);
                    Some(init_val)
                } else {
                    let loc = ctx.location();
                    let name_str = name.as_str().to_string();
                    if ctx.mutable_slots.borrow().contains(&name_str) {
                        // Rebind (`:`) of an existing mutable variable — store new value.
                        let ptr = match symtab.get(&name_str) {
                            Some(p) => p,
                            None => {
                                ctx.emit_internal(format!(
                                    "mutable slot '{name_str}' not found in symtab"
                                ));
                                return None;
                            }
                        };
                        block.store_typed(ctx, ptr, init_val, loc)?;
                        Some(block.const_i64(ctx.mlir, 0))
                    } else {
                        // First mutable bind (`:`) — alloca + store.
                        let elem_mlir_ty = ty_to_mlir(&ty, ctx.mlir);
                        let slot = block.alloca_typed(ctx.mlir, elem_mlir_ty, loc);
                        block.store_typed(ctx, slot, init_val, loc)?;
                        symtab.insert(name_str.clone(), slot);
                        ctx.var_types.borrow_mut().insert(name, ty);
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

/// Lower a literal to a ConstUnion discriminant integer.
/// Returns `None` if the expression isn't a literal or the value isn't in the set.
fn lower_const_union_literal<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    expr: &Typed<Expr>,
    values: &[ConstValue],
) -> Option<Value<'c, 'c>> {
    let lit = match &expr.value {
        Expr::Lit(lit) => lit,
        _ => return None,
    };
    let disc = ConstValue::find_discriminant(lit, values)? as i64;
    Some(super::emit_discriminant_constant(
        ctx,
        block,
        disc,
        values.len(),
    ))
}
