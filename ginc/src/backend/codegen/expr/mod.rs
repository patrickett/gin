//! Expression lowering implementations.

mod binary;
mod bind;
mod fn_call;
mod literal;
mod r#loop;

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};
use std::collections::HashMap;

impl<'c> Lower<'c> for Expr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match self {
            Expr::Lit(lit) => lit.lower(ctx, block, symtab),
            Expr::Binary(bin) => bin.lower(ctx, block, symtab),
            Expr::FnCall(call) => call.lower(ctx, block, symtab),
            Expr::Bind(bind) => bind.lower(ctx, block, symtab),
            Expr::Loop(_) => Err(CodegenSymptom::Internal(
                "Loop lowering not yet implemented".to_string(),
            )),
            Expr::FormatString(_) => Err(CodegenSymptom::Internal(
                "FormatString lowering not yet implemented".to_string(),
            )),
            Expr::Range(_) => Err(CodegenSymptom::Internal(
                "Range lowering not yet implemented".to_string(),
            )),
            Expr::Nothing => Ok(block.unit_value(ctx)),
        }
    }
}

pub fn lower_function<'c>(
    ctx: &CodegenContext<'_, 'c>,
    def_name: &IStr,
    bind: &Bind,
) -> Result<Operation<'c>, CodegenSymptom> {
    let name = def_name.as_str();
    let loc = ctx.location();

    let (param_names, input_types): (Vec<&IStr>, Vec<Type<'c>>) =
        if let Some(params) = bind.params().as_ref() {
            let names: Vec<&IStr> = params.keys().collect();
            let types: Vec<Type<'c>> = names.iter().map(|_| ctx.mlir.i64()).collect();
            (names, types)
        } else {
            (vec![], vec![])
        };

    let return_type = infer_return_type(ctx, bind)?;
    let func_type = FunctionType::new(ctx.mlir, &input_types, &[return_type]);

    let region = Region::new();
    {
        let block_args: Vec<_> = input_types.iter().map(|ty| (*ty, loc)).collect();
        let block = Block::new(&block_args);
        region.append_block(block);
        let block = region.first_block().unwrap();

        let mut symtab: RuntimeSymbolTable<'c> = HashMap::new();
        for (i, param_name) in param_names.iter().enumerate() {
            let arg = block.argument(i).unwrap();
            symtab.insert(param_name.as_str().to_string(), arg.into());
        }

        let result = lower_bind_value(ctx, &block, bind.value(), &symtab)?;

        let ret_op = if let Some(result1) = result {
            block.ret(ctx.mlir, &[result1])
        } else {
            block.ret(ctx.mlir, &[])
        };
        block.append_operation(ret_op);
    }

    let sym_name = Identifier::new(ctx.mlir, "sym_name");
    let func_type_id = Identifier::new(ctx.mlir, "function_type");

    OperationBuilder::new("func.func", loc)
        .add_attributes(&[
            (sym_name, ctx.mlir.str_attr(name)),
            (func_type_id, ctx.mlir.type_attr(Type::from(func_type))),
        ])
        .add_regions([region])
        .build()
        .map_err(|e| CodegenSymptom::Internal(format!("Failed to build func: {}", e)))
}

fn lower_bind_value<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    bind_value: &BindValue,
    symtab: &RuntimeSymbolTable<'c>,
) -> Result<Option<Value<'c, 'c>>, CodegenSymptom> {
    match bind_value {
        BindValue::Expr(expr) => Ok(Some(expr.lower(ctx, block, &mut symtab.clone())?)),
        BindValue::Body { exprs, ret } => {
            let mut local_symtab = symtab.clone();
            for expr in exprs {
                expr.lower(ctx, block, &mut local_symtab)?;
            }
            match &ret.0 {
                Some(expr) => Ok(Some(expr.lower(ctx, block, &mut local_symtab)?)),
                None => Ok(None),
            }
        }
    }
}

fn infer_return_type<'c>(
    ctx: &CodegenContext<'_, 'c>,
    bind: &Bind,
) -> Result<Type<'c>, CodegenSymptom> {
    match bind.value() {
        BindValue::Expr(expr) => infer_expr_type(ctx, expr),
        BindValue::Body { ret, .. } => match &ret.0 {
            Some(expr) => infer_expr_type(ctx, expr),
            None => Ok(ctx.mlir.i64()),
        },
    }
}

fn infer_expr_type<'c>(
    ctx: &CodegenContext<'_, 'c>,
    expr: &Expr,
) -> Result<Type<'c>, CodegenSymptom> {
    match expr {
        Expr::Lit(literal) => match literal {
            Literal::Int(_) | Literal::Number(_) => Ok(ctx.mlir.i64()),
            Literal::Float(_) => Ok(ctx.mlir.f64()),
            Literal::String(_) => Ok(ctx.mlir.string_type()),
        },
        _ => Ok(ctx.mlir.i64()), // TODO: proper type inference
    }
}
