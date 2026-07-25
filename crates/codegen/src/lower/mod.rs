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

            let param_types_mlir: Vec<melior::ir::Type<'c>> = bind
                .params
                .iter()
                .enumerate()
                .map(|(i, (_, ty))| match bind.param_conventions.get(i) {
                    Some(ParamConvention::Observe | ParamConvention::Mutate) => context.llvm_ptr(),
                    _ => ctx.ty_to_mlir(ty),
                })
                .collect();
            let param_loc_pairs: Vec<(melior::ir::Type<'c>, Location<'c>)> =
                param_types_mlir.iter().map(|t| (*t, loc)).collect();

            // Create function body region and entry block.
            let region = Region::new();
            let block = Block::new(&param_loc_pairs);
            region.append_block(block);
            let blk = region.first_block().unwrap();

            for (i, (name, param_ty)) in bind.params.iter().enumerate() {
                let val: melior::ir::Value<'c, 'c> = blk.argument(i).unwrap().into();
                let borrowed = matches!(
                    bind.param_conventions.get(i),
                    Some(ParamConvention::Observe | ParamConvention::Mutate)
                );
                if borrowed {
                    symtab.insert(*name, val, param_ty.clone(), true);
                } else {
                    let slot = blk.alloca_typed(context, ctx.ty_to_mlir(param_ty), loc);
                    blk.store_typed(&ctx, slot, val, loc);
                    symtab.insert(*name, slot, param_ty.clone(), true);
                }
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

        ctx.emit_string_globals(&module);

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
                ast::Literal::Number(0) => {
                    if let Some(value) = self.lower_synthetic_record_field(expr_id, block, symtab) {
                        return Some(value);
                    }
                    Some(block.const_i64(self.mlir, 0, self.location()))
                }
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
            typecheck::TypedExprKind::FnCall { target, args, .. } => {
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

                let fn_name = target.0.as_str();
                let loc = self.location();

                if (args.is_none() || args.as_ref().is_none_or(|a| a.is_empty()))
                    && let Some(slot) = symtab.get(&target.0)
                {
                    return if slot.is_slot {
                        block.load_typed(
                            self,
                            slot.value,
                            self.ty_to_mlir(&slot.ty),
                            self.location(),
                        )
                    } else {
                        Some(slot.value)
                    };
                }

                let return_ty = self.ty_to_mlir(expr_ref.ty);

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
                    symtab.insert(*name, slot, expr_ref.ty.clone(), true);
                    Some(slot)
                } else {
                    let val = self.lower_typed_expr(*body, block, symtab)?;
                    if let Some(ptr) = symtab.get_value(name) {
                        block.store_typed(self, ptr, val, self.location())?;
                    } else {
                        let slot = block.alloca_typed(
                            self.mlir,
                            self.ty_to_mlir(expr_ref.ty),
                            self.location(),
                        );
                        block.store_typed(self, slot, val, self.location())?;
                        symtab.insert(*name, slot, expr_ref.ty.clone(), true);
                    }
                    Some(val)
                }
            }
            typecheck::TypedExprKind::Reassign { name, value } => {
                let val = self.lower_typed_expr(*value, block, symtab)?;
                if let Some(ptr) = symtab.get_value(name) {
                    block.store_typed(self, ptr, val, self.location())?;
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
                let slot = symtab.get(&target.0)?;
                if slot.is_slot {
                    block.load_typed(self, slot.value, self.ty_to_mlir(&slot.ty), self.location())
                } else {
                    Some(slot.value)
                }
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
            typecheck::TypedExprKind::TupleAlloc { init, .. }
            | typecheck::TypedExprKind::ConsumeArg(init)
            | typecheck::TypedExprKind::Eat(init) => self.lower_typed_expr(*init, block, symtab),
            typecheck::TypedExprKind::TakePtr(init) | typecheck::TypedExprKind::Ref(init) => {
                self.lower_typed_place(*init, block, symtab)
            }
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
            typecheck::TypedExprKind::Deref(base) => {
                let ptr = self.lower_typed_expr(*base, block, symtab)?;
                block.load_typed(self, ptr, self.ty_to_mlir(expr_ref.ty), self.location())
            }
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

    fn lower_synthetic_record_field(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.typed_ast?;
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
        let field_index = match (&slot.ty, field_ty) {
            (Ty::Opaque(name), Ty::Opaque(field))
                if (name.as_str() == "Str" || name.as_str() == "String")
                    && field.as_str() == "pointer" =>
            {
                0
            }
            (Ty::Opaque(name), Ty::Opaque(field))
                if (name.as_str() == "Str" || name.as_str() == "String")
                    && field.as_str() == "len" =>
            {
                1
            }
            (Ty::Record { fields, .. }, field_ty) => {
                let mut matches = fields
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, ty))| ty.as_ref() == field_ty);
                let (index, _) = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                index
            }
            _ => return None,
        };
        let aggregate = if slot.is_slot {
            block.load_typed(self, slot.value, self.ty_to_mlir(&slot.ty), self.location())?
        } else {
            slot.value
        };
        let result_ty = match field_ty {
            Ty::Opaque(name) if name.as_str() == "pointer" => self.mlir.llvm_ptr(),
            Ty::Opaque(name) if name.as_str() == "len" => self.mlir.i64(),
            ty => self.ty_to_mlir(ty),
        };
        Some(block.append_op(self.mlir.llvm_extractvalue(
            aggregate,
            field_index as i64,
            result_ty,
            self.location(),
        )))
    }

    fn lower_typed_place(
        &self,
        expr_id: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let typed_ast = self.typed_ast?;
        let expr_ref = typed_ast.expr(expr_id)?;
        self.current_span.set(*expr_ref.span);

        match &expr_ref.kind {
            typecheck::TypedExprKind::FnCall { target, args, .. }
                if args.as_ref().is_none_or(Vec::is_empty) =>
            {
                symtab.get_value(&target.0)
            }
            typecheck::TypedExprKind::SelfRef { target } => symtab.get_value(&target.0),
            typecheck::TypedExprKind::Ref(inner) | typecheck::TypedExprKind::TakePtr(inner) => {
                self.lower_typed_place(*inner, block, symtab)
            }
            typecheck::TypedExprKind::Deref(inner) => self.lower_typed_expr(*inner, block, symtab),
            typecheck::TypedExprKind::TupleGet { base, index } => {
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
            typecheck::TypedExprKind::BufGet { buf, index } => {
                let base_ptr = self.lower_typed_place(*buf, block, symtab)?;
                let index = self.lower_typed_expr(*index, block, symtab)?;
                let base_ty = typed_ast.expr(*buf)?.ty;
                block.gep_typed(
                    self,
                    base_ptr,
                    self.ty_to_mlir(base_ty),
                    &[index],
                    &[i32::MIN],
                    self.location(),
                )
            }
            _ => {
                self.emit_internal("expression is not an addressable place");
                None
            }
        }
    }
}
