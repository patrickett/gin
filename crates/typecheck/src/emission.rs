use ast::{ConstValue, Ty, TyArg};
use diagnostic::Diagnostic;
use diagnostic::code::EmissionCode;
use flask::{Architecture, CompileTarget};
use internment::Intern;
use std::collections::HashSet;

use crate::TypeRegistry;
use crate::assembly::{AssemblyInput, AssemblyMemory, AssemblyOutput, AssemblySpec};
mod context;
mod operands;
mod value;

pub(crate) use context::EmissionContext;
use value::{Fields, sequence};
mod staging;

pub(crate) use staging::evaluate;

pub(crate) fn signature(ty: &Ty, registry: &TypeRegistry) -> Option<(Ty, Ty)> {
    const DESCRIPTOR_TYPES_MAX: usize = 1024;
    let mut pending = vec![ty.clone()];
    let mut visited = HashSet::new();
    let mut common = None;
    for _ in 0..DESCRIPTOR_TYPES_MAX {
        let Some(ty) = pending.pop() else {
            return common;
        };
        let definition = registry.resolved_definition_for_type(&ty);
        if !visited.insert(definition.clone()) {
            continue;
        }
        if let Ty::Union { variants, .. } = &definition {
            if variants.is_empty() {
                return None;
            }
            for variant in variants {
                if variant.kind != ast::ty::UnionVariantKind::Type {
                    return None;
                }
                let [(_, payload)] = variant.fields.as_slice() else {
                    return None;
                };
                pending.push((**payload).clone());
            }
            continue;
        }
        let candidate = record_signature(&definition)?;
        if common.as_ref().is_some_and(|common| common != &candidate) {
            return None;
        }
        common = Some(candidate);
    }
    None
}

fn record_signature(definition: &Ty) -> Option<(Ty, Ty)> {
    let (_, signature) = definition.named_field(Intern::from_ref("signature"))?;
    let Ty::Named { instance, .. } = signature else {
        return None;
    };
    let [(_, TyArg::Type(input)), (_, TyArg::Type(output))] = instance.arguments.as_slice() else {
        return None;
    };
    Some(((**input).clone(), (**output).clone()))
}

pub(crate) fn operand_fields(ty: &Ty, registry: &TypeRegistry) -> Vec<(Intern<String>, Ty)> {
    match registry.resolved_definition_for_type(ty) {
        Ty::Record { fields, .. } => fields.into_iter().map(|(name, ty)| (name, *ty)).collect(),
        Ty::Sequence(fields) => fields
            .into_iter()
            .filter_map(|field| field.name.map(|name| (name, *field.ty)))
            .collect(),
        _ => Vec::new(),
    }
}

fn error(message: impl Into<String>) -> Diagnostic {
    // type-invalid-emission
    Diagnostic::new(EmissionCode::Invalid, message.into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Register {
    bank: String,
    index: u8,
    width: u32,
    backend: String,
}

impl Register {
    fn read(
        value: &ConstValue,
        context: EmissionContext<'_>,
        arch: Architecture,
    ) -> Result<Self, Diagnostic> {
        let fields = Fields::new(
            value,
            context.program,
            &["architecture", "bank", "index", "width"],
        )?;
        let architecture = fields.text("architecture")?;
        let bank = fields.text("bank")?.into_owned();
        let index = u8::try_from(fields.integer("index")?)
            .map_err(|_| error("register index must fit in a byte"))?;
        let width =
            u32::try_from(fields.integer("width")?).map_err(|_| error("invalid register width"))?;
        let expected = match arch {
            Architecture::Aarch64(_) => "aarch64",
            Architecture::X86_64 => "x86_64",
            _ => return Err(error("#emit does not support this target")),
        };
        if architecture != expected {
            return Err(error(
                "register descriptor architecture does not match the target",
            ));
        }
        let backend = match (arch, bank.as_str(), index, width) {
            (_, "flags", 0, 4) => "cc".into(),
            (Architecture::Aarch64(_), "general", 0..=28, 64) => format!("x{index}"),
            (Architecture::Aarch64(_), "general", 0..=28, 32) => format!("w{index}"),
            (Architecture::X86_64, "general", 0..=15, 64) if !matches!(index, 4 | 5) => {
                crate::assembly::X86_64_GENERAL_REGISTERS[index as usize].into()
            }
            _ => return Err(error("unsupported register bank, index, or width")),
        };
        Ok(Self {
            bank,
            index,
            width,
            backend,
        })
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.bank == other.bank && self.index == other.index
    }
}

pub(crate) fn compile(
    value: &ConstValue,
    ty: &Ty,
    context: EmissionContext<'_>,
    target: &CompileTarget,
) -> Result<AssemblySpec, Diagnostic> {
    let CompileTarget::Concrete(target) = target else {
        return Err(error("#emit requires a concrete target"));
    };
    let fields = Fields::new(
        value,
        context.program,
        &[
            "signature",
            "target",
            "bytes",
            "inputs",
            "outputs",
            "clobbers",
            "reads",
            "writes",
        ],
    )?;
    let signature = Fields::new(fields.value("signature"), context.program, &["version"])?;
    if signature.integer("version")? != 1 {
        return Err(error("unsupported emission signature version"));
    }
    if fields.text("target")? != target.raw {
        return Err(error(
            "emitted code target does not match the active target",
        ));
    }
    let mut pending = instruction_bytes(fields.value("bytes"), context)?;
    pending.reverse();
    let mut bytes = Vec::new();
    while let Some(value) = pending.pop() {
        if let Some(value) = value.as_const_size_int() {
            bytes
                .push(u8::try_from(value).map_err(|_| {
                    error("emitted instruction bytes must be integers in 0 to 255")
                })?);
        } else {
            let mut group = instruction_bytes(value, context)?;
            group.reverse();
            pending.extend(group);
        }
    }
    if bytes.is_empty()
        || (matches!(target.architecture(), Architecture::Aarch64(_))
            && !bytes.len().is_multiple_of(4))
    {
        return Err(error(
            "emitted code must be nonempty and aligned to the target instruction width",
        ));
    }
    let bindings = |name| -> Result<Vec<_>, Diagnostic> {
        let values = fields.value(name);
        if sequence(values).is_ok_and(|values| values.is_empty()) {
            return Ok(Vec::new());
        }
        let bindings = Fields::read(values, context.program)?;
        bindings
            .entries()
            .map(|(name, value)| {
                let register = Register::read(value, context, target.architecture())?;
                if register.bank != "general" {
                    return Err(error("flags may be clobbered but cannot be value operands"));
                }
                Ok((name.to_string(), register))
            })
            .collect()
    };
    let inputs = bindings("inputs")?;
    let outputs = bindings("outputs")?;
    let clobbers = sequence(fields.value("clobbers"))?
        .into_iter()
        .map(|value| Register::read(value, context, target.architecture()))
        .collect::<Result<Vec<_>, _>>()?;
    for bindings in [&inputs, &outputs] {
        for (index, (_, register)) in bindings.iter().enumerate() {
            if bindings[..index]
                .iter()
                .any(|(_, other)| register.overlaps(other))
            {
                return Err(error("overlapping register operands"));
            }
        }
    }
    for (index, register) in clobbers.iter().enumerate() {
        if inputs
            .iter()
            .chain(&outputs)
            .any(|(_, other)| register.overlaps(other))
            || clobbers[..index]
                .iter()
                .any(|other| register.overlaps(other))
        {
            return Err(error(
                "clobbered register overlaps an operand or another clobber",
            ));
        }
    }
    let operands = operands::Inputs::new(ty, context)?;
    let accesses = |name| -> Result<Vec<_>, Diagnostic> {
        sequence(fields.value(name))?
            .into_iter()
            .enumerate()
            .map(|(position, value)| {
                let access = Fields::new(value, context.program, &["pointer", "length"])?;
                let lookup = |field| -> Result<String, Diagnostic> {
                    operands.lookup(access.value(field), name, position, field)
                };
                Ok((lookup("pointer")?, lookup("length")?))
            })
            .collect()
    };
    let memory = AssemblyMemory {
        reads: accesses("reads")?,
        writes: accesses("writes")?,
    };
    let inputs = inputs
        .into_iter()
        .map(|(parameter, register)| {
            let overlap = outputs.iter().find(|(_, other)| register.overlaps(other));
            if overlap.is_some_and(|(_, other)| register.width != other.width) {
                return Err(error(
                    "tied input and output must use the same register width",
                ));
            }
            Ok(AssemblyInput {
                parameter,
                register: register.backend,
                tie: overlap.is_some(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let outputs = outputs
        .into_iter()
        .map(|(name, register)| AssemblyOutput {
            name,
            register: register.backend,
        })
        .collect();
    Ok(AssemblySpec {
        target: target.raw.clone(),
        dialect: "att".into(),
        inputs,
        result: None,
        outputs,
        clobbers: clobbers
            .into_iter()
            .map(|register| register.backend)
            .collect(),
        memory,
        side_effects: true,
        template: format!(
            ".byte {}",
            bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn instruction_bytes<'a>(
    value: &'a ConstValue,
    context: EmissionContext<'_>,
) -> Result<Vec<&'a ConstValue>, Diagnostic> {
    if matches!(value, ConstValue::Record { .. } | ConstValue::Tag { .. }) {
        let fields = Fields::read(value, context.program)?;
        if fields.has("storage") {
            let fields = fields.require(&["storage", "length"])?;
            let length = usize::try_from(fields.integer("length")?)
                .map_err(|_| error("instruction buffer length must be nonnegative"))?;
            let storage = sequence(fields.value("storage"))?;
            if length > storage.len() {
                return Err(error("instruction buffer length exceeds storage capacity"));
            }
            return Ok(storage.into_iter().take(length).collect());
        }
        return Ok(fields.entries().map(|(_, value)| value).collect());
    }
    sequence(value)
}
