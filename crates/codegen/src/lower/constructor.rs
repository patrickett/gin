use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use internment::Intern;
use typecheck::ExprId;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_literal_union_value(
        &self,
        value: &ast::ConstValue,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let definition = self.program.type_registry.resolved_definition_for_type(ty);
        let values = definition.union_literal_values()?;
        let Some(index) = values.iter().position(|alternative| alternative == value) else {
            // codegen-literal-union-invalid
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::LiteralUnionInvalid),
                "literal value is absent from its union".into(),
            );
            return None;
        };
        Some(block.const_int(
            self.mlir,
            self.ty_to_mlir(ty),
            i128::try_from(index).ok()?,
            self.location(),
        ))
    }

    pub(super) fn lower_constructor_arguments(
        &self,
        args: &[ExprId],
        field_names: &[Intern<String>],
        fields: &[(Intern<String>, Box<Ty>)],
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Vec<Value<'c, 'c>>> {
        if args.len() != fields.len() {
            // codegen-constructor-invalid
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                format!(
                    "constructor requires {} fields, but supplies {} arguments",
                    fields.len(),
                    args.len()
                ),
            );
            return None;
        }
        if !field_names.is_empty() && field_names.len() != args.len() {
            // codegen-constructor-invalid
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                "constructor field names must correspond to its arguments".into(),
            );
            return None;
        }
        let mut occupied = vec![false; fields.len()];
        let positions = (0..args.len())
            .map(|index| {
                let position = match field_names
                    .get(index)
                    .filter(|name| !name.as_str().is_empty())
                {
                    Some(name) => {
                        let Some(position) = fields.iter().position(|(field, _)| field == name)
                        else {
                            // codegen-constructor-invalid
                            self.emit_symptom(
                                DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                                format!("constructor names unknown field `{name}`"),
                            );
                            return None;
                        };
                        position
                    }
                    None => index,
                };
                if std::mem::replace(&mut occupied[position], true) {
                    // codegen-constructor-invalid
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                        format!(
                            "constructor supplies field `{}` more than once",
                            fields[position].0
                        ),
                    );
                    return None;
                }
                Some(position)
            })
            .collect::<Option<Vec<_>>>()?;
        let mut values = vec![None; fields.len()];
        for (arg, position) in args.iter().zip(positions) {
            let source_ty = self.program.expr(*arg)?.ty;
            let mut value = self.lower_typed_expr(*arg, block, symtab)?;
            let expected = self.ty_to_mlir(&fields[position].1);
            if value.r#type() != expected {
                if IntegerType::try_from(value.r#type()).is_ok()
                    && IntegerType::try_from(expected).is_ok()
                {
                    value = self.convert_integer_value(value, source_ty, expected, block)?;
                } else {
                    // codegen-constructor-invalid
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::ConstructorInvalid),
                        format!(
                            "constructor value for field `{}` has storage type {}, expected {} ({} to {})",
                            fields[position].0,
                            value.r#type(),
                            expected,
                            source_ty.format_for_hover(),
                            fields[position].1.format_for_hover()
                        ),
                    );
                    return None;
                }
            }
            values[position] = Some(value);
        }
        Some(
            values
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .expect("validated constructor positions must initialize every field"),
        )
    }
}
