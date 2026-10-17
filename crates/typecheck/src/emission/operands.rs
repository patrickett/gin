use ast::{ConstValue, Ty, TyArg};
use diagnostic::Diagnostic;
use internment::Intern;

use super::value::Fields;
use super::{EmissionContext, error, operand_fields, signature};
use crate::representation::Repr;

pub(super) struct Inputs<'a> {
    code: &'a Ty,
    ty: Ty,
    fields: Vec<(Intern<String>, Ty)>,
    context: EmissionContext<'a>,
}

impl<'a> Inputs<'a> {
    pub(super) fn new(code: &'a Ty, context: EmissionContext<'a>) -> Result<Self, Diagnostic> {
        let ty = signature(code, context.program.type_registry)
            .ok_or_else(|| error("code requires a typed signature"))?
            .0;
        let fields = operand_fields(&ty, context.program.type_registry);
        Ok(Self {
            code,
            ty,
            fields,
            context,
        })
    }

    pub(super) fn lookup(
        &self,
        value: &ConstValue,
        access: &str,
        position: usize,
        field: &str,
    ) -> Result<String, Diagnostic> {
        let index = if let Some(index) = value.as_const_size_int() {
            index
        } else {
            Fields::new(value, self.context.program, &["index"])?.integer("index")?
        };
        let index = usize::try_from(index).map_err(|_| error("invalid memory operand index"))?;
        let (name, ty) = self
            .fields
            .get(index)
            .ok_or_else(|| error("memory effect refers to an unknown input"))?;
        if value.as_const_size_int().is_none() {
            let accesses = self.field_type(self.code, access)?;
            let access = self.element_type(&accesses, position)?;
            let handle = self.field_type(&access, field)?;
            let Some(instance) = handle.named_instance() else {
                return Err(error(
                    "operand handle requires input and value type arguments",
                ));
            };
            let [(_, TyArg::Type(input)), (_, TyArg::Type(value))] = instance.arguments.as_slice()
            else {
                return Err(error(
                    "operand handle requires input and value type arguments",
                ));
            };
            if **input != self.ty || **value != *ty {
                return Err(error(
                    "operand handle type does not match the signature input at its index",
                ));
            }
        }
        Ok(name.to_string())
    }

    fn field_type(&self, ty: &Ty, field: &str) -> Result<Ty, Diagnostic> {
        self.context
            .program
            .type_registry
            .resolved_definition_for_type(ty)
            .named_field(Intern::from_ref(field))
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| error(format!("missing memory effect field `{field}`")))
    }

    fn element_type(&self, ty: &Ty, index: usize) -> Result<Ty, Diagnostic> {
        if let Some((element, length)) =
            Repr::array_parts(ty, Some(self.context.program.type_registry))
            && (index as u64) < length
        {
            return Ok(element);
        }
        operand_fields(ty, self.context.program.type_registry)
            .get(index)
            .map(|(_, ty)| ty.clone())
            .or_else(|| match ty {
                Ty::Sequence(fields) => fields.get(index).map(|field| *field.ty.clone()),
                _ => None,
            })
            .ok_or_else(|| error("memory effects require a sequence of accesses"))
    }
}
