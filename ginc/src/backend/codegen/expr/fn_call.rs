//! Lower function call expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for FnCall {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let func_name = if self.path.segments.is_empty() {
            self.path.root
        } else {
            let segs: Vec<&str> = self.path.segments.iter().map(|s| s.as_str()).collect();
            IStr::new(format!("{}.{}", self.path.root, segs.join(".")))
        };

        if func_name.as_str() == "print" {
            return self.lower_print_call(ctx, block, symtab);
        }

        if let Some(value) = symtab.get(func_name.as_str()) {
            return Ok(*value);
        }

        if let Some(symbol) = ctx.symbol_table.get(&func_name)
            && symbol.is_bind()
        {
            return Err(CodegenSymptom::Internal(format!(
                "Cannot call '{func_name}': it is a value definition (bind), not a function"
            )));
        }

        let mut args = Vec::new();
        if let Some(arg_exprs) = &self.args {
            for arg_expr in arg_exprs {
                args.push(arg_expr.lower(ctx, block, symtab)?);
            }
        }

        Ok(block.call(ctx.mlir, &func_name, &args))
    }
}

impl FnCall {
    fn lower_print_call<'c>(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let args = self.args.as_ref().ok_or_else(|| {
            CodegenSymptom::Internal("print requires exactly one argument".to_string())
        })?;

        if args.len() != 1 {
            return Err(CodegenSymptom::Internal(
                "print requires exactly one argument".to_string(),
            ));
        }

        let arg = &args[0];
        arg.lower(ctx, block, symtab)?;

        Ok(block.unit_value(ctx))
    }
}
