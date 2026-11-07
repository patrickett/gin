//! Running and checking a straight-line program.

use std::collections::BTreeMap;

use crate::abstract_machine::{Abstract, Effects, Fact};
use crate::concrete::Concrete;
use crate::instruction::{Instruction, Reg, X0};
use crate::semantics::{ModelError, execute, fail};

/// Named constant data, by symbol.
pub type Data = BTreeMap<String, Vec<u8>>;

fn at(index: usize, instruction: &Instruction, error: ModelError) -> ModelError {
    ModelError(format!("[{index}] {instruction}: {error}"))
}

/// Check `program` against its semantics. `result` is the domain `x0` must be proven to
/// lie in at `ret`. Returns the effects derived for each instruction.
pub fn check(
    program: &[Instruction],
    data: &Data,
    result: (u64, u64),
) -> Result<Vec<Effects>, ModelError> {
    check_with(
        program,
        data,
        &[],
        (X0, (i128::from(result.0), i128::from(result.1))),
    )
}

/// [`check`] for a function whose parameters arrive in registers known to lie in the
/// given ranges, and whose result leaves in a register and must lie in the given range.
pub fn check_with(
    program: &[Instruction],
    data: &Data,
    arguments: &[(Reg, (i128, i128))],
    result: (Reg, (i128, i128)),
) -> Result<Vec<Effects>, ModelError> {
    let mut machine = Abstract::new(data).with_arguments(arguments);
    let mut effects = Vec::with_capacity(program.len());
    for (index, instruction) in program.iter().enumerate() {
        if machine.halted {
            return fail(format!("[{index}] {instruction} is unreachable after ret"));
        }
        machine.pc = index;
        machine.effects = Effects::default();
        execute(&mut machine, instruction).map_err(|error| at(index, instruction, error))?;
        effects.push(std::mem::take(&mut machine.effects));
    }
    if !machine.halted {
        return fail("falls off the end without ret");
    }
    let (register, range) = result;
    match machine.fact(register).and_then(Fact::bounds) {
        Some((low, high)) if range.0 <= low && high <= range.1 => Ok(effects),
        _ => fail(format!(
            "{register} is not proven to be in {} to {} at ret",
            range.0, range.1
        )),
    }
}

/// Emulate `program`: its standard output and the exit status in `x0`.
pub fn run(program: &[Instruction], data: &Data) -> Result<(Vec<u8>, u64), ModelError> {
    let mut machine = Concrete::new(data);
    for (index, instruction) in program.iter().enumerate() {
        machine.pc = index;
        execute(&mut machine, instruction).map_err(|error| at(index, instruction, error))?;
        if machine.halted {
            let status = machine
                .register(X0)
                .ok_or_else(|| ModelError("x0 unset at ret".into()))?;
            return Ok((machine.stdout, status));
        }
    }
    fail("falls off the end without ret")
}
