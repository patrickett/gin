use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn require_if_without_joins(&self, if_expr: &typecheck::TypedIfExpr) -> Option<()> {
        if !if_expr.place_joins.is_empty() {
            // codegen-if-join-unresolved
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::IfJoinUnresolved),
                "returning if branches cannot merge place versions".into(),
            );
            return None;
        }
        Some(())
    }

    pub(crate) fn lower_typed_if_values(
        &self,
        if_expr: &typecheck::TypedIfExpr,
        expr_ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<LoweredValue<'c>> {
        self.require_if_without_joins(if_expr)?;
        if !matches!(expr_ty, Ty::Unit) {
            // codegen-if-result-unavailable
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::IfResultUnavailable),
                "when result values require a returning sequence with a fallthrough return then"
                    .into(),
            );
            return None;
        }
        let loc = self.location();
        let condition = self.lower_typed_condition(&if_expr.condition, block, symtab)?;
        let then_region = Region::new();
        let then_block = Block::new(&[]);
        then_region.append_block(then_block);
        let then_block = then_region.first_block()?;
        let mut then_symbols = symtab.clone();
        self.bind_typed_condition_patterns(&condition, &then_block, &mut then_symbols)?;
        for statement in &if_expr.stmts {
            self.lower_typed_values(*statement, &then_block, &mut then_symbols)?;
        }
        if let Some(ret) = if_expr.ret {
            let value = self.lower_typed_values(ret, &then_block, &mut then_symbols)?;
            if !value.values().is_empty() {
                // codegen-if-result-unavailable
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::IfResultUnavailable),
                    "when result values require a returning sequence with a fallthrough return then"
                        .into(),
                );
                return None;
            }
        }
        then_block.append_operation(scf_dialect::r#yield(&[], loc));

        let else_region = Region::new();
        let else_block = Block::new(&[]);
        else_region.append_block(else_block);
        let else_block = else_region.first_block()?;
        else_block.append_operation(scf_dialect::r#yield(&[], loc));

        block.append_scf_if(condition.value, &[], then_region, else_region, loc);
        Some(LoweredValue::new(Vec::new()))
    }
}
