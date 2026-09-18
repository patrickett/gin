use ast::ConstValue;
use diagnostic::Diagnostic;
use internment::Intern;

use crate::{DefId, ResolvedProgram};

use super::contract::Fields;
use super::{
    ContractSource, EncodingArgument, EncodingItem, EncodingRegion, InstructionEncoder, error,
};

impl EncodingRegion {
    pub(crate) fn evaluation_method(
        &self,
        program: ResolvedProgram<'_>,
    ) -> Result<Option<DefId>, Diagnostic> {
        let ContractSource::Value(value) = &self.contract else {
            return Ok(None);
        };
        let fields = Fields::read(value, program)?;
        InstructionEncoder::associated_method(
            value,
            "evaluate",
            fields.sequence("inputs")?.len(),
            program,
        )
    }

    pub fn from_value(
        value: &ConstValue,
        program: ResolvedProgram<'_>,
    ) -> Result<Self, Diagnostic> {
        let fields = Fields::new(value, program, &["contract", "items"])?;
        let items = fields
            .sequence("items")?
            .iter()
            .map(|item| EncodingItem::from_value(item, program))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            contract: ContractSource::Value(fields.value("contract").clone()),
            items,
        })
    }
}

impl EncodingItem {
    fn from_value(value: &ConstValue, program: ResolvedProgram<'_>) -> Result<Self, Diagnostic> {
        let fields = Fields::read(value, program)?;
        if fields.has("label") {
            let fields = fields.require(&["label"])?;
            return Ok(Self::Label(Intern::from_ref(
                fields.text("label")?.as_ref(),
            )));
        }
        let fields = fields.require(&["encoder", "size", "arguments"])?;
        let size = u32::try_from(fields.integer("size")?).map_err(|_| {
            error(
                "size",
                "instruction size must fit an unsigned 32-bit integer",
            )
        })?;
        let arguments: Vec<_> = fields
            .sequence("arguments")?
            .iter()
            .map(|arg| EncodingArgument::from_value(arg, program))
            .collect::<Result<_, _>>()?;
        let callable =
            InstructionEncoder::resolve_method(fields.value("encoder"), arguments.len(), program)?;
        Ok(Self::Instruction(
            InstructionEncoder::new(callable, size).with_arguments(arguments),
        ))
    }
}

impl InstructionEncoder {
    fn resolve_method(
        value: &ConstValue,
        arity: usize,
        program: ResolvedProgram<'_>,
    ) -> Result<DefId, Diagnostic> {
        Self::associated_method(value, "encode", arity, program)?.ok_or_else(|| {
            error(
                "encoder",
                format!(
                    "encoder needs an associated `encode` method with {arity} integer parameters"
                ),
            )
        })
    }

    fn associated_method(
        value: &ConstValue,
        method: &str,
        arity: usize,
        program: ResolvedProgram<'_>,
    ) -> Result<Option<DefId>, Diagnostic> {
        let ConstValue::Tag {
            qual_path: Some(path),
            ..
        } = value
        else {
            return Err(error(
                "encoder",
                "encoder must be a Gin value with an associated `encode` method",
            ));
        };
        let tag = program
            .tags
            .get(
                &program
                    .tag_id_for_value(value)
                    .ok_or_else(|| error("encoder", "unknown encoder declaration"))?,
            )
            .ok_or_else(|| error("encoder", "unknown encoder declaration"))?;
        let mut candidates = program.defs.iter().filter(|(_, bind)| {
            bind.receiver_type.as_ref().is_some_and(|receiver| {
                receiver == &tag.resolved_ty
                    || receiver
                        .type_id()
                        .zip(tag.resolved_ty.type_id())
                        .is_some_and(|(receiver, declaration)| receiver == declaration)
            }) && bind
                .name
                .as_str()
                .rsplit('.')
                .next()
                .is_some_and(|name| name.split("$arity").next() == Some(method))
        });
        let Some((target, _)) = candidates.next() else {
            return Ok(None);
        };
        if candidates.next().is_some() {
            return Err(error(
                "encoder",
                format!("`{path}` has ambiguous `{method}` methods"),
            ));
        }
        let bind = &program.defs[target];
        if bind.params.len() != arity || bind.params.iter().any(|(name, _)| name.as_str() == "self")
        {
            return Err(error(
                "encoder",
                format!("`{method}` requires {arity} parameters and no self receiver"),
            ));
        }
        Ok(Some(*target))
    }
}

impl EncodingArgument {
    fn from_value(value: &ConstValue, program: ResolvedProgram<'_>) -> Result<Self, Diagnostic> {
        let fields = Fields::read(value, program)?;
        if fields.has("register") {
            let fields = fields.require(&["register"])?;
            return Ok(Self::Register(Intern::from_ref(
                fields.text("register")?.as_ref(),
            )));
        }
        let integer = |fields: &Fields<'_>, name| {
            i64::try_from(fields.integer(name)?).map_err(|_| {
                error(
                    "argument",
                    format!("`{name}` must fit a signed 64-bit integer"),
                )
            })
        };
        if fields.has("immediate") {
            let fields = fields.require(&["immediate"])?;
            return Ok(Self::Immediate(integer(&fields, "immediate")?));
        }
        let fields = fields.require(&["relative", "min", "max"])?;
        Ok(Self::Relative {
            label: Intern::from_ref(fields.text("relative")?.as_ref()),
            min: integer(&fields, "min")?,
            max: integer(&fields, "max")?,
        })
    }
}
