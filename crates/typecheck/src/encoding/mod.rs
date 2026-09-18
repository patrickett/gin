mod allocation;
mod contract;
mod source;

use std::collections::HashMap;

use crate::DefId;
use crate::assembly::{AssemblyInput, AssemblyMemory, AssemblyOutput, AssemblySpec};
use crate::eval::{CompileTimeHost, Evaluator};
use crate::resolved_program::ResolvedProgram;
use ast::ConstValue;
use diagnostic::Diagnostic;
use flask::{CompileTarget, TargetArch};
use internment::Intern;

use allocation::Allocation;
use contract::Contract;
pub(crate) use contract::Fields;

#[derive(Clone, Debug)]
pub enum EncodingArgument {
    Register(Intern<String>),
    Immediate(i64),
    Relative {
        label: Intern<String>,
        min: i64,
        max: i64,
    },
}

#[derive(Clone, Debug)]
pub struct InstructionEncoder {
    callable: DefId,
    size: u32,
    arguments: Vec<EncodingArgument>,
}

impl InstructionEncoder {
    pub fn new(callable: DefId, size: u32) -> Self {
        Self {
            callable,
            size,
            arguments: Vec::new(),
        }
    }

    pub fn with_arguments(mut self, arguments: Vec<EncodingArgument>) -> Self {
        self.arguments = arguments;
        self
    }
}

#[derive(Clone, Debug)]
pub enum EncodingItem {
    Instruction(InstructionEncoder),
    Label(Intern<String>),
}

enum ContractSource {
    Callable(DefId),
    Value(ConstValue),
}

pub struct EncodingRegion {
    contract: ContractSource,
    items: Vec<EncodingItem>,
}

impl EncodingRegion {
    pub fn new(contract: DefId, items: Vec<EncodingItem>) -> Self {
        Self {
            contract: ContractSource::Callable(contract),
            items,
        }
    }

    pub fn compile(
        &self,
        program: ResolvedProgram<'_>,
        target: &CompileTarget,
    ) -> Result<EncodedRegion, Diagnostic> {
        let CompileTarget::Concrete(target_info) = target else {
            return Err(error(
                "target",
                "instruction encoding requires a concrete target",
            ));
        };
        if !matches!(target_info.arch, TargetArch::X86_64 | TargetArch::Arm64) {
            return Err(error(
                "target",
                "instruction encoding currently supports x86-64 and arm64",
            ));
        }
        let mut evaluator = Evaluator::new(program, CompileTimeHost::new(target));
        let value = match &self.contract {
            ContractSource::Callable(callable) => {
                Self::check_callable(program, *callable, &[])?;
                evaluator.call(*callable, &[])?
            }
            ContractSource::Value(value) => value.clone(),
        };
        let contract = Contract::from_value(&value, &target_info.raw, program)?;
        let allocation = Allocation::assign(&contract.slots, &contract.clobbers)
            .map_err(|message| error("allocation", message))?;
        let registers: HashMap<_, _> = contract
            .slots
            .iter()
            .zip(&allocation.registers)
            .map(|(slot, register)| (slot.name, register.clone()))
            .collect();

        let mut labels = HashMap::new();
        let mut length = 0_u32;
        for item in &self.items {
            match item {
                EncodingItem::Label(name) => {
                    if labels.insert(*name, length).is_some() {
                        return Err(error("label", format!("duplicate code label `{name}`")));
                    }
                }
                EncodingItem::Instruction(encoder) => {
                    if encoder.size == 0 {
                        return Err(error("size", "instruction size must be positive"));
                    }
                    if target_info.arch == TargetArch::Arm64 && !encoder.size.is_multiple_of(4) {
                        return Err(error(
                            "size",
                            "arm64 instructions must contain whole 32-bit words",
                        ));
                    }
                    length = length
                        .checked_add(encoder.size)
                        .filter(|size| *size <= 1_048_576)
                        .ok_or_else(|| {
                            error("size", "encoded region exceeds the prototype's 1 MiB limit")
                        })?;
                }
            }
        }
        let mut bytes = Vec::with_capacity(length as usize);
        for item in &self.items {
            let EncodingItem::Instruction(encoder) = item else {
                continue;
            };

            let end = bytes.len() as i64 + i64::from(encoder.size);
            let arguments = encoder.arguments.iter().map(|arg| {
                let value = match arg {
                    EncodingArgument::Register(slot) => {
                        let register = registers.get(slot)
                            .ok_or_else(|| error("operand", format!("unknown register slot `{slot}`")))?;
                        i64::from(register.code)
                    }
                    EncodingArgument::Immediate(value) => *value,
                    EncodingArgument::Relative { label, min, max } => {
                        let offset = labels.get(label)
                            .ok_or_else(|| error("label", format!("unknown code label `{label}`")))?;
                        let displacement = i64::from(*offset) - end;
                        if min > max || !(*min..=*max).contains(&displacement) {
                            return Err(error("displacement", format!(
                                "branch to `{label}` needs displacement {displacement}, outside {min} to {max}"
                            )));
                        }
                        displacement
                    }
                };
                Ok(ConstValue::Int(value.into()))
            }).collect::<Result<Vec<_>, Diagnostic>>()?;
            Self::check_callable(program, encoder.callable, &arguments)?;
            let result = evaluator.call(encoder.callable, &arguments)?;
            let encoded = contract::sequence(&result)?;
            if encoded.len() != encoder.size as usize {
                return Err(error(
                    "size",
                    format!(
                        "encoder `{}` returned {} bytes, expected {}",
                        encoder.callable.0,
                        encoded.len(),
                        encoder.size
                    ),
                ));
            }
            for value in encoded {
                let byte = value
                    .as_const_size_int()
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| {
                        error(
                            "byte",
                            "instruction encoder returned a value outside 0 to 255",
                        )
                    })?;
                bytes.push(byte);
            }
        }
        if bytes.is_empty() {
            return Err(error(
                "size",
                "an encoded region must contain an instruction",
            ));
        }
        let outputs: Vec<_> = contract
            .outputs
            .iter()
            .map(|slot| AssemblyOutput {
                name: slot.to_string(),
                register: registers[slot].name.to_string(),
            })
            .collect();
        let inputs: Vec<_> = contract
            .inputs
            .iter()
            .map(|slot| AssemblyInput {
                parameter: slot.to_string(),
                register: registers[slot].name.to_string(),
                tie: contract.outputs.contains(slot),
            })
            .collect();
        let mut clobbers: Vec<_> = contract.clobbers.iter().map(ToString::to_string).collect();
        for slot in &contract.slots {
            let register = &registers[&slot.name];
            if !contract.inputs.contains(&slot.name) && !contract.outputs.contains(&slot.name) {
                clobbers.push(register.name.to_string());
            }
        }
        let memory = AssemblyMemory {
            reads: contract
                .reads
                .iter()
                .map(|(pointer, length)| {
                    (
                        contract.inputs[*pointer].to_string(),
                        contract.inputs[*length].to_string(),
                    )
                })
                .collect(),
            writes: contract
                .writes
                .iter()
                .map(|(pointer, length)| {
                    (
                        contract.inputs[*pointer].to_string(),
                        contract.inputs[*length].to_string(),
                    )
                })
                .collect(),
        };
        let template = if target_info.arch == TargetArch::Arm64 {
            format!(
                ".inst {}",
                bytes
                    .chunks_exact(4)
                    .map(|word| u32::from_le_bytes(word.try_into().unwrap()).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!(
                ".byte {}",
                bytes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let spec = AssemblySpec {
            target: target_info.raw.clone(),
            dialect: "att".into(),
            inputs,
            result: None,
            outputs,
            clobbers,
            memory,
            side_effects: true,
            template,
        };
        Ok(EncodedRegion {
            bytes,
            spec,
            registers: registers
                .into_iter()
                .map(|(slot, register)| (slot, register.name))
                .collect(),
        })
    }

    fn check_callable(
        program: ResolvedProgram<'_>,
        callable: DefId,
        args: &[ConstValue],
    ) -> Result<(), Diagnostic> {
        let arity = args.len();
        let bind = program.defs.get(&callable).ok_or_else(|| {
            error(
                "encoder",
                format!("unknown encoder callable `{}`", callable.0),
            )
        })?;
        if matches!(bind.body, crate::typed::BindBody::Unlowered) || bind.params.len() != arity {
            return Err(error(
                "encoder",
                format!(
                    "`{}` must be a Gin function with {arity} parameters",
                    callable.0
                ),
            ));
        }
        for ((name, ty), value) in bind.params.iter().zip(args) {
            let ConstValue::Int(value) = value else {
                return Err(error("argument", "encoder arguments must be integers"));
            };
            let valid = program
                .type_registry
                .integer_validity_for_type(ty)
                .is_some_and(|validity| validity.domain().contains(*value));
            if !valid {
                return Err(error(
                    "argument",
                    format!("encoder argument `{name}` is outside its declared range"),
                ));
            }
        }
        Ok(())
    }
}

pub struct EncodedRegion {
    bytes: Vec<u8>,
    spec: AssemblySpec,
    registers: HashMap<Intern<String>, Intern<String>>,
}

impl EncodedRegion {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn register(&self, slot: &str) -> Option<&str> {
        self.registers
            .get(&Intern::from_ref(slot))
            .map(|name| name.as_str())
    }
    pub fn assembly(&self) -> &AssemblySpec {
        &self.spec
    }
    pub fn into_assembly(self) -> AssemblySpec {
        self.spec
    }
}

fn error(kind: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(format!("type-encoding-{kind}"), message.into())
}
