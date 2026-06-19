mod asm;
mod context;
mod format_string;
mod if_;
mod loop_;
mod mlir;
mod ty_mapping;
mod when;

pub use context::{CodegenContext, TypeInfo};

use crate::prelude::*;

use ast::HashFloat;
use ast::SymbolTable as CompileTimeSymbolTable;
use diagnostic::Diagnostic;
use melior::ir::Location;
use typecheck::TypedFileAst;
use typecheck::compile_time_trait::CompileTimeTraitRegistry;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Build an MLIR module from a `TypedFileAst` using the ExprId-based codegen path.
    ///
    /// Returns the module and any codegen-level symptoms (errors, warnings).
    /// This is the primary entry point for the typed-arena codegen path.
    pub fn build_module_from_typed_ast(
        context: &'c Context,
        typed: &'a TypedFileAst,
        source: &'a str,
        filename: &str,
        trait_registry: Option<&'a CompileTimeTraitRegistry>,
    ) -> (Option<melior::ir::Module<'c>>, Vec<Diagnostic>) {
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

        let module = melior::ir::Module::new(Location::new(context, filename, 0, 0));

        for (def_id, bind) in &typed.defs {
            let mut symtab = crate::ScopedSymbolTable::new();
            let func_name = def_id.0.as_str();
            ctx.current_span.set(bind.name_span);
            let loc = ctx.location();

            // Resolve parameter MLIR types and create block arguments.
            let param_types_mlir: Vec<melior::ir::Type<'c>> = bind
                .params
                .iter()
                .map(|(_, ty)| ctx.ty_to_mlir(ty))
                .collect();
            let param_loc_pairs: Vec<(melior::ir::Type<'c>, Location<'c>)> =
                param_types_mlir.iter().map(|t| (*t, loc)).collect();

            // Create function body region and entry block.
            let region = Region::new();
            let block = Block::new(&param_loc_pairs);
            region.append_block(block);
            let blk = region.first_block().unwrap();

            // Insert parameter names into the symbol table.
            for (i, (name, _)) in bind.params.iter().enumerate() {
                let val: melior::ir::Value<'c, 'c> = blk.argument(i).unwrap().into();
                symtab.insert(name.as_str().to_string(), val);
            }

            // Lower the body expression.
            let result_val = match &bind.body {
                typecheck::BindBody::Expr(eid) => ctx.lower_typed_expr(*eid, &blk, &mut symtab),
                typecheck::BindBody::Body { exprs, ret } => {
                    for eid in exprs {
                        ctx.lower_typed_expr(*eid, &blk, &mut symtab);
                    }
                    ret.and_then(|ret_id| ctx.lower_typed_expr(ret_id, &blk, &mut symtab))
                }
                typecheck::BindBody::Extern => None,
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
            let func_type =
                melior::ir::r#type::FunctionType::new(context, &param_types_mlir, &ret_types);
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
    pub fn lower_typed_expr(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.typed_ast?;
        let expr_ref = typed_ast.expr(expr_id)?;

        // Propagate source location from the original expression span.
        self.current_span.set(*expr_ref.span);

        match expr_ref.kind {
            typecheck::TypedExprKind::Lit(lit) => match lit {
                ast::Literal::Number(n) => {
                    // usize always fits in i64 on all supported platforms.
                    Some(block.const_i64(self.mlir, *n as i64, self.location()))
                }
                ast::Literal::Int(n) => {
                    let n = *n;
                    if n > i64::MAX as u128 {
                        Some(block.const_int(
                            self.mlir,
                            self.mlir.i128(),
                            n as i128,
                            self.location(),
                        ))
                    } else {
                        Some(block.const_i64(self.mlir, n as i64, self.location()))
                    }
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
                    let len = block.const_i64(self.mlir, s.len() as i64, self.location());
                    let undef = block.append_op(
                        self.mlir
                            .llvm_undef(self.mlir.string_type(), self.location()),
                    );
                    let with_ptr =
                        block.append_op(self.mlir.llvm_insertvalue(undef, ptr, 0, self.location()));
                    Some(
                        block.append_op(self.mlir.llvm_insertvalue(
                            with_ptr,
                            len,
                            1,
                            self.location(),
                        )),
                    )
                }
            },
            typecheck::TypedExprKind::FnCall { target, args } => {
                // Lower arguments first.
                let lowered_args: Vec<Value<'c, 'c>> = args
                    .as_ref()
                    .map(|a| {
                        a.iter()
                            .filter_map(|arg_id| self.lower_typed_expr(*arg_id, block, symtab))
                            .collect()
                    })
                    .unwrap_or_default();

                let fn_name = target.0.as_str();
                let loc = self.location();

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
                    .map(|t| self.ty_to_mlir(t))
                    .unwrap_or_else(|| self.mlir.i64());

                Some(block.call(self.mlir, fn_name, &lowered_args, return_ty, loc))
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
                    let name_str = name.as_str().to_string();
                    symtab.insert(name_str, slot);
                    self.var_types
                        .borrow_mut()
                        .insert(*name, expr_ref.ty.clone());
                    self.mutable_slots
                        .borrow_mut()
                        .insert(name.as_str().to_string());
                    Some(slot)
                } else {
                    let val = self.lower_typed_expr(*body, block, symtab)?;
                    let name_str = name.as_str().to_string();
                    if self.mutable_slots.borrow().contains(&name_str) {
                        // Rebind — store new value to existing slot.
                        if let Some(ptr) = symtab.get(&name_str) {
                            block.store_typed(self, ptr, val, self.location())?;
                            // Update the var type in case it changed (e.g., narrowed).
                            self.var_types
                                .borrow_mut()
                                .insert(*name, expr_ref.ty.clone());
                            Some(val)
                        } else {
                            self.emit_internal(format!(
                                "mutable slot '{}' not found in symtab",
                                name_str
                            ));
                            None
                        }
                    } else {
                        symtab.insert(name_str, val);
                        self.var_types
                            .borrow_mut()
                            .insert(*name, expr_ref.ty.clone());
                        self.mutable_slots
                            .borrow_mut()
                            .insert(name.as_str().to_string());
                        Some(val)
                    }
                }
            }
            typecheck::TypedExprKind::Reassign { name, value } => {
                let name_str = name.as_str().to_string();
                let val = self.lower_typed_expr(*value, block, symtab)?;
                if let Some(ptr) = symtab.get(&name_str) {
                    block.store_typed(self, ptr, val, self.location())?;
                    self.var_types
                        .borrow_mut()
                        .insert(*name, expr_ref.ty.clone());
                    Some(val)
                } else {
                    self.emit_internal(format!(
                        "cannot reassign '{}': not found in symtab",
                        name_str
                    ));
                    None
                }
            }
            typecheck::TypedExprKind::Binary { op, lhs, rhs } => {
                let lhs_val = self.lower_typed_expr(*lhs, block, symtab)?;
                let rhs_val = self.lower_typed_expr(*rhs, block, symtab)?;
                let loc = self.location();

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
            typecheck::TypedExprKind::TagCall {
                variant_id: _,
                discriminant,
                args,
                field_names: _,
            } => {
                let lowered_args: Vec<Value<'c, 'c>> = args
                    .as_ref()
                    .map(|a| {
                        a.iter()
                            .filter_map(|arg_id| self.lower_typed_expr(*arg_id, block, symtab))
                            .collect()
                    })
                    .unwrap_or_default();
                // Emit a struct { disc, payload } for the tag variant.
                let _loc = self.location();
                let disc_val = block.const_i64(self.mlir, *discriminant as i64, self.location());
                let field_types: Vec<Type<'c>> = std::iter::once(self.mlir.i64())
                    .chain(lowered_args.iter().map(|v| v.r#type()))
                    .collect();
                use melior::dialect::llvm::r#type;
                let struct_ty = r#type::r#struct(self.mlir, &field_types, false);
                let undef = block.append_op(self.mlir.llvm_undef(struct_ty, self.location()));
                let with_disc = block.append_op(self.mlir.llvm_insertvalue(
                    undef,
                    disc_val,
                    0,
                    self.location(),
                ));
                let result = lowered_args
                    .into_iter()
                    .enumerate()
                    .fold(with_disc, |acc, (i, v)| {
                        block.append_op(self.mlir.llvm_insertvalue(
                            acc,
                            v,
                            (i + 1) as i64,
                            self.location(),
                        ))
                    });
                Some(result)
            }
            typecheck::TypedExprKind::SelfRef { target } => {
                // Look up `self` in the symbol table.
                symtab.get(target.0.as_str())
            }
            typecheck::TypedExprKind::Range { start, end } => {
                let _s = self.lower_typed_expr(*start, block, symtab)?;
                let _e = self.lower_typed_expr(*end, block, symtab)?;
                None // Range lowering not yet implemented
            }
            typecheck::TypedExprKind::TupleLit(items) => {
                let vals: Vec<Value<'c, 'c>> = items
                    .iter()
                    .filter_map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect();
                vals.first().copied()
            }
            typecheck::TypedExprKind::Cast { expr, .. } => {
                self.lower_typed_expr(*expr, block, symtab)
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
            typecheck::TypedExprKind::TupleAlloc { init, .. }
            | typecheck::TypedExprKind::TakePtr(init)
            | typecheck::TypedExprKind::Ref(init)
            | typecheck::TypedExprKind::ConsumeArg(init)
            | typecheck::TypedExprKind::Eat(init) => self.lower_typed_expr(*init, block, symtab),
            typecheck::TypedExprKind::TupleGet { base, index } => {
                let base_val = self.lower_typed_expr(*base, block, symtab)?;
                // Extract the field at the given index from the struct/tuple.
                // Use the expression's own type as the result type.
                let result_ty = self.ty_to_mlir(expr_ref.ty);
                Some(block.append_op(self.mlir.llvm_extractvalue(
                    base_val,
                    *index as i64,
                    result_ty,
                    self.location(),
                )))
            }
            typecheck::TypedExprKind::Deref(base) => self.lower_typed_expr(*base, block, symtab),
            typecheck::TypedExprKind::TupleSet { base, value, .. } => {
                self.lower_typed_expr(*base, block, symtab)?;
                self.lower_typed_expr(*value, block, symtab)
            }
            typecheck::TypedExprKind::BufGet { buf, index }
            | typecheck::TypedExprKind::BufSet { buf, index, .. } => {
                self.lower_typed_expr(*buf, block, symtab)?;
                self.lower_typed_expr(*index, block, symtab)?;
                None
            }
            typecheck::TypedExprKind::List(items) => {
                let vals: Vec<Value<'c, 'c>> = items
                    .iter()
                    .filter_map(|id| self.lower_typed_expr(*id, block, symtab))
                    .collect();
                vals.first().copied()
            }
            typecheck::TypedExprKind::FormatString(fs) => {
                self.lower_typed_format_string(fs, block, symtab)
            }
            typecheck::TypedExprKind::If(if_expr) => self.lower_typed_if(if_expr, block, symtab),
            typecheck::TypedExprKind::When(when_expr) => {
                if let Some(cv) = &expr_ref.const_value {
                    self.lower_const_value(block, cv)
                } else {
                    self.lower_typed_when(when_expr, block, symtab)
                }
            }
            typecheck::TypedExprKind::Loop(loop_expr) => {
                self.lower_typed_loop(loop_expr, block, symtab)
            }
            typecheck::TypedExprKind::Asm(asm_expr) => {
                self.lower_typed_asm(asm_expr, block, symtab)
            }
            typecheck::TypedExprKind::Destructure { .. }
            | typecheck::TypedExprKind::RecordSet { .. } => None,
        }
    }
}
