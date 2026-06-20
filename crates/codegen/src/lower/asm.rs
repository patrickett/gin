use crate::prelude::*;

use ast::expr::{ClobberSpec, OperandKind, OperandSpec};

impl<'a, 'c> CodegenContext<'a, 'c> {
    fn build_constraint_string(operands: &[OperandSpec], clobbers: &[ClobberSpec]) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(operands.len() + clobbers.len());

        for op in operands {
            let prefix = match op.kind {
                OperandKind::Input => "",
                OperandKind::Output => "=",
                OperandKind::InOut => "=",
                OperandKind::LateOut => "=&",
            };
            parts.push(format!("{prefix}{{{}}}", op.register.as_str()));
        }

        for clobber in clobbers {
            match clobber {
                ClobberSpec::Register(r) => parts.push(format!("~{{{}}}", r.as_str())),
                ClobberSpec::Memory => parts.push("~{memory}".into()),
            }
        }

        parts.join(",")
    }
    /// Lower a typed-arena inline asm expression.
    pub(crate) fn lower_typed_asm(
        &self,
        asm_expr: &ast::expr::AsmExpr,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();

        let constraint_str = Self::build_constraint_string(&asm_expr.operands, &asm_expr.clobbers);

        // Lower operand values that consume a runtime value (Input, InOut).
        // ASM operands are typically simple variable references or integer literals.
        let values: Vec<Value<'c, 'c>> = asm_expr
            .operands
            .iter()
            .zip(&asm_expr.operand_values)
            .filter(|(op, _)| matches!(op.kind, OperandKind::Input | OperandKind::InOut))
            .map(|(_, v)| self.lower_asm_operand(block, symtab, v))
            .collect::<Option<Vec<_>>>()?;

        let bool_true = IntegerAttribute::new(IntegerType::new(self.mlir, 1).into(), 1).into();

        let asm_op = OperationBuilder::new("llvm.inline_asm", loc)
            .add_attributes(&[
                (
                    Identifier::new(self.mlir, "asm_string"),
                    StringAttribute::new(self.mlir, asm_expr.template.as_str()).into(),
                ),
                (
                    Identifier::new(self.mlir, "constraints"),
                    StringAttribute::new(self.mlir, &constraint_str).into(),
                ),
                (Identifier::new(self.mlir, "has_side_effects"), bool_true),
            ])
            .add_operands(&values)
            .add_results(&[self.mlir.i64()])
            .build();

        match asm_op {
            Ok(op) => Some(block.append_op(op)),
            Err(e) => {
                self.emit_internal(format!("llvm.inline_asm: {e}"));
                None
            }
        }
    }

    /// Lower a single ASM operand value (variable reference or integer literal).
    fn lower_asm_operand(
        &self,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
        expr: &ast::Typed<ast::Expr>,
    ) -> Option<Value<'c, 'c>> {
        // Simple variable lookup.
        if let ast::Expr::FnCall(call) = &expr.value
            && call.path.segments.is_empty()
            && call.args.is_none()
            && let Some(val) = symtab.get_value(&call.path.root)
        {
            return Some(val);
        }
        // Integer literal.
        if let ast::Expr::Lit(lit) = &expr.value {
            match lit {
                ast::Literal::Number(n) => {
                    return Some(block.const_i64(self.mlir, *n as i64, self.location()));
                }
                ast::Literal::Int(n) => {
                    return Some(block.const_i64(self.mlir, *n as i64, self.location()));
                }
                _ => {}
            }
        }
        self.emit_internal("unsupported ASM operand type (expected variable or literal)");
        None
    }
}
