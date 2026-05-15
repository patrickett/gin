use crate::prelude::*;

use ast::Expr;

/// Derive an LLVM constraint string from a typed constraint expression.
///
/// Each constraint is a value of the `Constraint` union type:
/// - `Output(X)` → `"={x}"`
/// - `LateOut(X)` → `"=&{x}"`
/// - `Input(X)` → `"{x}"`
/// - `InOut(X)` → `"={x}"`
/// - `Clobber(X)` → `"~{x}"`
/// - `ClobberMemory` → `"~{memory}"`
///
/// The register name is extracted from the constraint's type parameter
/// (e.g. `X0` in `Output[X0]`).
fn derive_constraint_string(
    constraints: &[ast::Spanned<Expr>],
    ctx: &CodegenContext<'_, '_>,
) -> (String, Vec<usize>) {
    let mut parts = Vec::new();
    let mut output_indices = Vec::new();

    for (i, constraint) in constraints.iter().enumerate() {
        let (prefix, register_name) = match &constraint.value {
            Expr::TagCall(tc) => {
                let prefix = match tc.name.as_str() {
                    "Output" => "=",
                    "LateOut" => "=&",
                    "Input" => "",
                    "InOut" => "=",
                    "Clobber" => "~",
                    _ => {
                        ctx.emit_internal(format!(
                            "unknown constraint variant: {}",
                            tc.name.as_str()
                        ));
                        return (String::new(), Vec::new());
                    }
                };
                let reg = tc.args.first().and_then(|arg| match &arg.value {
                    Expr::AnonymousTag(name, _) => Some(name.as_str().to_lowercase()),
                    _ => None,
                });
                if matches!(tc.name.as_str(), "Output" | "LateOut" | "InOut") {
                    output_indices.push(i);
                }
                (prefix, reg)
            }
            Expr::AnonymousTag(name, _) if name.as_str() == "ClobberMemory" => {
                ("~", Some("memory".to_string()))
            }
            _ => {
                ctx.emit_internal(format!(
                    "unsupported constraint expression: {:?}",
                    constraint.value
                ));
                return (String::new(), Vec::new());
            }
        };

        match register_name {
            Some(rn) => parts.push(format!("{prefix}{{{rn}}}")),
            None => {
                ctx.emit_internal("constraint missing register name");
                return (String::new(), Vec::new());
            }
        }
    }

    (parts.join(","), output_indices)
}

impl<'c> Lower<'c> for ast::expr::AsmExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();

        let mut operand_values: Vec<Value<'c, 'c>> = Vec::with_capacity(self.operands.len());
        for operand in &self.operands {
            operand_values.push(operand.lower(ctx, block, symtab)?);
        }

        let (constraint_str, _output_indices) = derive_constraint_string(&self.constraints, ctx);

        let bool_true = IntegerAttribute::new(IntegerType::new(ctx.mlir, 1).into(), 1).into();

        let asm_op = OperationBuilder::new("llvm.inline_asm", loc)
            .add_attributes(&[
                (
                    Identifier::new(ctx.mlir, "asm_string"),
                    StringAttribute::new(ctx.mlir, self.template.as_str()).into(),
                ),
                (
                    Identifier::new(ctx.mlir, "constraints"),
                    StringAttribute::new(ctx.mlir, &constraint_str).into(),
                ),
                (Identifier::new(ctx.mlir, "has_side_effects"), bool_true),
            ])
            .add_operands(&operand_values)
            .add_results(&[ctx.mlir.i64()])
            .build();

        match asm_op {
            Ok(op) => Some(block.append_op(op)),
            Err(e) => {
                ctx.emit_internal(format!("llvm.inline_asm: {e}"));
                None
            }
        }
    }
}
