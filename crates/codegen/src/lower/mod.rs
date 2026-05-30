mod asm;
mod context;
mod format_string;
mod helpers;
mod if_;
mod loop_;
mod mlir;
mod ty_mapping;
mod when;

pub use context::{CodegenContext, TypeInfo};
pub(crate) use helpers::{bind_pattern_payload_fields, emit_discriminant_extend};
pub use mlir::{addressof_string_global, create_string_global};
pub use ty_mapping::ty_to_mlir;

use crate::prelude::*;

use ast::SymbolTable as CompileTimeSymbolTable;
use diagnostic::Diagnostic;
use typecheck::compile_time_trait::CompileTimeTraitRegistry;
use typed_ast::TypedFileAst;

/// Build an MLIR module from a `TypedFileAst` using the ExprId-based codegen path.
///
/// Returns the module and any codegen-level symptoms (errors, warnings).
/// This is the primary entry point for the typed-arena codegen path.
pub fn build_module_from_typed_ast<'a, 'c>(
    context: &'c Context,
    typed: &'a TypedFileAst,
    source: &'a str,
    filename: &str,
    trait_registry: Option<&'a CompileTimeTraitRegistry>,
) -> (Option<melior::ir::Module<'c>>, Vec<Diagnostic>) {
    use crate::prelude::*;
    use melior::dialect::func;
    use melior::ir::{Block, Region};
    use std::collections::HashMap;

    let empty: HashMap<_, _> = HashMap::new();
    let sym_table = CompileTimeSymbolTable::new();
    let ctx = CodegenContext::new(
        context,
        Some(typed),
        trait_registry,
        &empty,
        &sym_table,
        source,
        filename,
        &typed.span_table,
    );

    let module = melior::ir::Module::new(context.unknown_loc());

    for (def_id, bind) in &typed.defs {
        let mut symtab = crate::ScopedSymbolTable::new();
        let func_name = def_id.0.as_str();
        let loc = context.unknown_loc();

        // Create function body region and entry block.
        let region = Region::new();
        let block = Block::new(&[]);
        region.append_block(block);
        let blk = region.first_block().unwrap();

        // Lower the body expression.
        let result_val = match &bind.body {
            typed_ast::BindBody::Expr(eid) => lower_typed_expr(&ctx, *eid, &blk, &mut symtab),
            typed_ast::BindBody::Body { exprs, ret } => {
                for eid in exprs {
                    lower_typed_expr(&ctx, *eid, &blk, &mut symtab);
                }
                ret.and_then(|ret_id| lower_typed_expr(&ctx, ret_id, &blk, &mut symtab))
            }
            typed_ast::BindBody::Extern => None,
        };

        // Emit return with the proper value and compute the function type.
        let ret_types: Vec<melior::ir::Type<'c>> = match &result_val {
            Some(val) => {
                let ret_ty = val.r#type();
                blk.append_operation(func::r#return(&[*val], loc));
                vec![ret_ty]
            }
            None => {
                blk.append_operation(func::r#return(&[], loc));
                vec![]
            }
        };

        // Create the func.func operation with the correct function type.
        let sym_name = Identifier::new(context, "sym_name");
        let func_type_id = Identifier::new(context, "function_type");
        let func_type = melior::ir::r#type::FunctionType::new(context, &[], &ret_types);
        match OperationBuilder::new("func.func", loc)
            .add_attributes(&[
                (sym_name, StringAttribute::new(context, func_name).into()),
                (func_type_id, TypeAttribute::new(func_type.into()).into()),
            ])
            .add_regions([region])
            .build()
        {
            Ok(func_op) => {
                let _ = module.body().append_operation(func_op);
            }
            Err(_) => {
                let symptoms = ctx.drain_symptoms();
                return (None, symptoms);
            }
        }
    }

    let symptoms = ctx.drain_symptoms();
    (Some(module), symptoms)
}

/// This is the ExprId-based codegen path. It reads from `ctx.typed_ast` (the typed
/// expression arena) instead of traversing `Box<Typed<Expr>>` pointers.
pub fn lower_typed_expr<'c>(
    ctx: &CodegenContext<'_, 'c>,
    expr_id: typed_ast::ExprId,
    block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
) -> Option<Value<'c, 'c>> {
    let typed_ast = ctx.typed_ast?;
    let expr_ref = typed_ast.expr(expr_id)?;

    match expr_ref.kind {
        typed_ast::TypedExprKind::Lit(lit) => match lit {
            ast::Literal::Number(n) => Some(block.const_i64(ctx.mlir, *n as i64)),
            ast::Literal::Int(n) => Some(block.const_i64(ctx.mlir, *n as i64)),
            ast::Literal::Float(f) => {
                let f_attr = ctx.mlir.f64_attr(*f);
                Some(block.append_op(ctx.mlir.const_op(f_attr, ctx.mlir.f64())))
            }
            ast::Literal::String(s) => {
                let name = ctx.register_string(s);
                let ptr = addressof_string_global(ctx.mlir, block, &name)?;
                let len = block.const_i64(ctx.mlir, s.len() as i64);
                let undef = block.append_op(ctx.mlir.llvm_undef(ctx.mlir.string_type()));
                let with_ptr = block.append_op(ctx.mlir.llvm_insertvalue(undef, ptr, 0));
                Some(block.append_op(ctx.mlir.llvm_insertvalue(with_ptr, len, 1)))
            }
        },
        typed_ast::TypedExprKind::FnCall { target, args } => {
            // Lower arguments first.
            let lowered_args: Vec<Value<'c, 'c>> = args
                .as_ref()
                .map(|a| {
                    a.iter()
                        .filter_map(|arg_id| lower_typed_expr(ctx, *arg_id, block, symtab))
                        .collect()
                })
                .unwrap_or_default();

            let fn_name = target.0.as_str();
            let loc = ctx.location();

            // Look up the var in the symtab if no args (variable reference).
            if (args.is_none() || args.as_ref().is_none_or(|a| a.is_empty()))
                && let Some(val) = symtab.get(fn_name)
            {
                return Some(val);
            }

            // Determine return type from the typed AST.
            let return_ty = typed_ast
                .fn_return_types
                .get(target)
                .map(|t| ty_to_mlir(t, ctx.mlir, ctx.typed_ast.unwrap(), ctx.trait_registry))
                .unwrap_or_else(|| ctx.mlir.i64());

            Some(block.call(ctx.mlir, fn_name, &lowered_args, return_ty, loc))
        }
        typed_ast::TypedExprKind::Bind {
            name,
            stmts,
            body,
            unassigned,
        } => {
            for stmt_id in stmts {
                lower_typed_expr(ctx, *stmt_id, block, symtab)?;
            }

            if *unassigned {
                // Declaration without value: allocate stack space but don't initialize.
                // Use the declared type to determine the alloca size.
                let loc = ctx.location();
                let mlir_ty = ty_to_mlir(
                    expr_ref.ty,
                    ctx.mlir,
                    ctx.typed_ast.unwrap(),
                    ctx.trait_registry,
                );
                let slot = block.alloca_typed(ctx.mlir, mlir_ty, loc);
                let name_str = name.as_str().to_string();
                symtab.insert(name_str, slot);
                ctx.var_types
                    .borrow_mut()
                    .insert(*name, expr_ref.ty.clone());
                ctx.mutable_slots
                    .borrow_mut()
                    .insert(name.as_str().to_string());
                Some(slot)
            } else {
                let val = lower_typed_expr(ctx, *body, block, symtab)?;
                let name_str = name.as_str().to_string();
                if ctx.mutable_slots.borrow().contains(&name_str) {
                    // Rebind — store new value to existing slot.
                    if let Some(ptr) = symtab.get(&name_str) {
                        block.store_typed(ctx, ptr, val, ctx.location())?;
                        // Update the var type in case it changed (e.g., narrowed).
                        ctx.var_types
                            .borrow_mut()
                            .insert(*name, expr_ref.ty.clone());
                        Some(val)
                    } else {
                        ctx.emit_internal(format!(
                            "mutable slot '{}' not found in symtab",
                            name_str
                        ));
                        None
                    }
                } else {
                    symtab.insert(name_str, val);
                    ctx.var_types
                        .borrow_mut()
                        .insert(*name, expr_ref.ty.clone());
                    ctx.mutable_slots
                        .borrow_mut()
                        .insert(name.as_str().to_string());
                    Some(val)
                }
            }
        }
        typed_ast::TypedExprKind::Reassign { name, value } => {
            let name_str = name.as_str().to_string();
            let val = lower_typed_expr(ctx, *value, block, symtab)?;
            if let Some(ptr) = symtab.get(&name_str) {
                block.store_typed(ctx, ptr, val, ctx.location())?;
                ctx.var_types
                    .borrow_mut()
                    .insert(*name, expr_ref.ty.clone());
                Some(val)
            } else {
                ctx.emit_internal(format!(
                    "cannot reassign '{}': not found in symtab",
                    name_str
                ));
                None
            }
        }
        typed_ast::TypedExprKind::Binary { op, lhs, rhs } => {
            let lhs_val = lower_typed_expr(ctx, *lhs, block, symtab)?;
            let rhs_val = lower_typed_expr(ctx, *rhs, block, symtab)?;
            let loc = ctx.location();

            use melior::ir::operation::OperationBuilder;

            let bin_op_name = match op {
                BinOp::Add => "arith.addi",
                BinOp::Subtract => "arith.subi",
                BinOp::Multiply => "arith.muli",
                _ => return None,
            };

            let result_ty = lhs_val.r#type();
            let op = OperationBuilder::new(bin_op_name, loc)
                .add_operands(&[lhs_val, rhs_val])
                .add_results(&[result_ty])
                .build()
                .ok()?;
            Some(block.append_op(op))
        }
        typed_ast::TypedExprKind::TagCall {
            variant_id: _,
            discriminant,
            args,
        } => {
            let lowered_args: Vec<Value<'c, 'c>> = args
                .as_ref()
                .map(|a| {
                    a.iter()
                        .filter_map(|arg_id| lower_typed_expr(ctx, *arg_id, block, symtab))
                        .collect()
                })
                .unwrap_or_default();
            // Emit a struct { disc, payload } for the tag variant.
            let _loc = ctx.location();
            let disc_val = block.const_i64(ctx.mlir, *discriminant as i64);
            let field_types: Vec<Type<'c>> = std::iter::once(ctx.mlir.i64())
                .chain(lowered_args.iter().map(|v| v.r#type()))
                .collect();
            use melior::dialect::llvm::r#type;
            let struct_ty = r#type::r#struct(ctx.mlir, &field_types, false);
            let undef = block.append_op(ctx.mlir.llvm_undef(struct_ty));
            let with_disc = block.append_op(ctx.mlir.llvm_insertvalue(undef, disc_val, 0));
            let result = lowered_args
                .into_iter()
                .enumerate()
                .fold(with_disc, |acc, (i, v)| {
                    block.append_op(ctx.mlir.llvm_insertvalue(acc, v, (i + 1) as i64))
                });
            Some(result)
        }
        typed_ast::TypedExprKind::SelfRef { target } => {
            // Look up `self` in the symbol table.
            symtab.get(target.0.as_str())
        }
        typed_ast::TypedExprKind::Range { start, end } => {
            let _s = lower_typed_expr(ctx, *start, block, symtab)?;
            let _e = lower_typed_expr(ctx, *end, block, symtab)?;
            None // Range lowering not yet implemented
        }
        typed_ast::TypedExprKind::TupleLit(items) => {
            let vals: Vec<Value<'c, 'c>> = items
                .iter()
                .filter_map(|id| lower_typed_expr(ctx, *id, block, symtab))
                .collect();
            vals.first().copied()
        }
        typed_ast::TypedExprKind::Cast { expr, .. } => lower_typed_expr(ctx, *expr, block, symtab),
        typed_ast::TypedExprKind::TupleAlloc { init, .. }
        | typed_ast::TypedExprKind::TakePtr(init)
        | typed_ast::TypedExprKind::Ref(init)
        | typed_ast::TypedExprKind::ConsumeArg(init)
        | typed_ast::TypedExprKind::Negate(init)
        | typed_ast::TypedExprKind::Eat(init) => lower_typed_expr(ctx, *init, block, symtab),
        typed_ast::TypedExprKind::TupleGet { base, .. } | typed_ast::TypedExprKind::Deref(base) => {
            lower_typed_expr(ctx, *base, block, symtab)
        }
        typed_ast::TypedExprKind::TupleSet { base, value, .. } => {
            lower_typed_expr(ctx, *base, block, symtab)?;
            lower_typed_expr(ctx, *value, block, symtab)
        }
        typed_ast::TypedExprKind::BufGet { buf, index }
        | typed_ast::TypedExprKind::BufSet { buf, index, .. } => {
            lower_typed_expr(ctx, *buf, block, symtab)?;
            lower_typed_expr(ctx, *index, block, symtab)?;
            None
        }
        typed_ast::TypedExprKind::List(items) => {
            let vals: Vec<Value<'c, 'c>> = items
                .iter()
                .filter_map(|id| lower_typed_expr(ctx, *id, block, symtab))
                .collect();
            vals.first().copied()
        }
        typed_ast::TypedExprKind::FormatString(fs) => {
            format_string::lower_typed_format_string(ctx, fs, block, symtab)
        }
        typed_ast::TypedExprKind::If(if_expr) => if_::lower_typed_if(ctx, if_expr, block, symtab),
        typed_ast::TypedExprKind::When(when_expr) => {
            if let Some(cv) = &expr_ref.const_value {
                mlir::lower_const_value(ctx, block, cv)
            } else {
                when::lower_typed_when(ctx, when_expr, block, symtab)
            }
        }
        typed_ast::TypedExprKind::Loop(loop_expr) => {
            loop_::lower_typed_loop(ctx, loop_expr, block, symtab)
        }
        typed_ast::TypedExprKind::Asm(asm_expr) => {
            asm::lower_typed_asm(ctx, asm_expr, block, symtab)
        }
    }
}
