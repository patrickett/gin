//! The machine model written in Gin, run by the compile-time evaluator.
//!
//! `gin/arm64.gin` defines the register file (`Machine`, a record of linear `Register`s),
//! the operand names (`ArmRegister`) and one `execute_*` function per instruction. This
//! file only builds the argument values and reads the result back: it holds no knowledge of
//! what an instruction does. [`crate::semantics::execute`] remains the Rust model, and the
//! tests hold the two to the same answers.

use i256::I256;
use num_traits::ToPrimitive;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated,
    evaluate_value,
};

use crate::instruction::{Instruction, Reg};

const FUEL: u64 = 100_000;

/// Registers the Gin machine models: `x0..x2`.
pub const REGISTERS: usize = 3;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GinError {
    #[error("the Gin machine models x0..x2; `{0}` is outside it")]
    Register(Reg),
    #[error("`{0}` has no Gin semantics yet")]
    Instruction(String),
    #[error("the Gin library has no `{0}`")]
    Missing(&'static str),
    #[error("`{0}` could not be evaluated")]
    Unevaluated(&'static str),
}

/// The checked Gin library, ready to run instructions on a register file.
pub struct GinMachine<'a> {
    file: &'a CheckedSourceFile,
}

struct Table<'a>(&'a CheckedSourceFile);

impl BodyResolver for Table<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        let CheckedSourceCallee::Local { function } = callee else {
            return None;
        };
        self.0.bodies.get(function as usize)?.as_ref()
    }
}

impl<'a> GinMachine<'a> {
    pub fn new(file: &'a CheckedSourceFile) -> Self {
        Self { file }
    }

    fn schema(&self, name: &'static str) -> Result<u32, GinError> {
        self.file
            .aggregates
            .iter()
            .position(|aggregate| aggregate.name == name)
            .map(|index| index as u32)
            .ok_or(GinError::Missing(name))
    }

    /// A `Word` is a nominal domain: a record around the one integer it holds.
    fn word(&self, value: u64) -> Result<Evaluated, GinError> {
        Ok(Evaluated::Record {
            schema: self.schema("Word")?,
            fields: vec![Evaluated::Integer(I256::from(i128::from(value)))],
        })
    }

    fn machine(&self, registers: [u64; REGISTERS]) -> Result<Evaluated, GinError> {
        let register = self.schema("Register")?;
        let mut fields = Vec::new();
        for contents in registers {
            fields.push(Evaluated::Record {
                schema: register,
                fields: vec![self.word(contents)?],
            });
        }
        Ok(Evaluated::Record {
            schema: self.schema("Machine")?,
            fields,
        })
    }

    fn name(&self, register: Reg) -> Result<Evaluated, GinError> {
        let name = ["ArmRegister.X0", "ArmRegister.X1", "ArmRegister.X2"]
            .get(usize::from(register.0))
            .ok_or(GinError::Register(register))?;
        Ok(Evaluated::Record {
            schema: self.schema(name)?,
            fields: Vec::new(),
        })
    }

    fn registers(&self, machine: &Evaluated) -> Result<[u64; REGISTERS], GinError> {
        let Evaluated::Record { fields, .. } = machine else {
            return Err(GinError::Unevaluated("Machine"));
        };
        let mut registers = [0; REGISTERS];
        for (slot, register) in registers.iter_mut().zip(fields) {
            let Evaluated::Record { fields, .. } = register else {
                return Err(GinError::Unevaluated("Register"));
            };
            let [Evaluated::Record { fields: word, .. }] = fields.as_slice() else {
                return Err(GinError::Unevaluated("Register"));
            };
            let [Evaluated::Integer(value)] = word.as_slice() else {
                return Err(GinError::Unevaluated("Word"));
            };
            *slot = value
                .to_u64()
                .ok_or(GinError::Unevaluated("Register"))?;
        }
        Ok(registers)
    }

    /// Run one instruction, in Gin, on `registers`.
    pub fn step(
        &self,
        instruction: &Instruction,
        registers: [u64; REGISTERS],
    ) -> Result<[u64; REGISTERS], GinError> {
        let (function, operands): (&'static str, Vec<Reg>) = match instruction {
            Instruction::Add { dest, left, right } => ("execute_add", vec![*dest, *left, *right]),
            Instruction::Sub { dest, left, right } => ("execute_sub", vec![*dest, *left, *right]),
            Instruction::Mul { dest, left, right } => ("execute_mul", vec![*dest, *left, *right]),
            Instruction::Mov { dest, src } => ("execute_mov", vec![*dest, *src]),
            other => return Err(GinError::Instruction(other.to_string())),
        };
        let index = self
            .file
            .functions
            .iter()
            .position(|candidate| candidate.name == function)
            .ok_or(GinError::Missing(function))?;
        let body = self.file.bodies[index]
            .as_ref()
            .ok_or(GinError::Unevaluated(function))?;
        let mut arguments = vec![self.machine(registers)?];
        for operand in operands {
            arguments.push(self.name(operand)?);
        }
        let result = evaluate_value(body, &arguments, &Table(self.file), &mut { FUEL })
            .ok_or(GinError::Unevaluated(function))?;
        self.registers(&result)
    }
}
