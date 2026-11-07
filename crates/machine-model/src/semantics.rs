//! The meaning of each instruction, written once.
//!
//! [`execute`] touches the world only through [`Machine`]. A concrete machine gives it
//! numbers and runs the program; an abstract machine gives it facts and checks it. The
//! semantics never branches on a value, so both machines see the same effects.

use crate::instruction::{Instruction, Reg, X0, X1, X2, X30};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ModelError(pub String);

pub fn fail<T>(message: impl Into<String>) -> Result<T, ModelError> {
    Err(ModelError(message.into()))
}

pub trait Machine {
    type Value: Clone;

    fn get(&mut self, register: Reg) -> Result<Self::Value, ModelError>;
    fn set(&mut self, register: Reg, value: Self::Value);
    /// Caller-saved registers whose contents a call destroys.
    fn clobber(&mut self, registers: std::ops::RangeInclusive<u8>);
    fn constant(&self, value: u64) -> Self::Value;
    fn page(&self, symbol: &str) -> Result<Self::Value, ModelError>;
    fn low12_add(&self, page: &Self::Value, symbol: &str) -> Result<Self::Value, ModelError>;
    fn offset(&self, address: &Self::Value, by: i64) -> Result<Self::Value, ModelError>;
    fn load64(&mut self, address: &Self::Value) -> Result<Self::Value, ModelError>;
    fn store64(&mut self, address: &Self::Value, value: Self::Value) -> Result<(), ModelError>;
    /// 64-bit two's-complement arithmetic. A machine that tracks facts must refuse a
    /// result it cannot prove is the mathematical one: that is the rule's precondition.
    fn add(&self, left: &Self::Value, right: &Self::Value) -> Result<Self::Value, ModelError>;
    fn sub(&self, left: &Self::Value, right: &Self::Value) -> Result<Self::Value, ModelError>;
    fn mul(&self, left: &Self::Value, right: &Self::Value) -> Result<Self::Value, ModelError>;
    fn return_address(&self) -> Self::Value;
    fn count(&self, length: &Self::Value) -> Self::Value;
    fn write_stdout(
        &mut self,
        fd: &Self::Value,
        buffer: &Self::Value,
        length: &Self::Value,
    ) -> Result<(), ModelError>;
    fn ret(&mut self) -> Result<(), ModelError>;
}

/// C `write(fd, buf, n)` through `libSystem`: reads `x0..x2`, returns the count in `x0`,
/// and destroys the other caller-saved registers (AAPCS64: `x0`-`x17`).
fn libsystem_write<M: Machine>(m: &mut M) -> Result<(), ModelError> {
    let fd = m.get(X0)?;
    let buffer = m.get(X1)?;
    let length = m.get(X2)?;
    m.write_stdout(&fd, &buffer, &length)?;
    m.clobber(1..=17);
    let count = m.count(&length);
    m.set(X0, count);
    Ok(())
}

type External<M> = fn(&mut M) -> Result<(), ModelError>;

fn external<M: Machine>(name: &str) -> Result<External<M>, ModelError> {
    match name {
        "_write" => Ok(libsystem_write::<M>),
        other => fail(format!("no model for external symbol {other}")),
    }
}

pub fn execute<M: Machine>(m: &mut M, instruction: &Instruction) -> Result<(), ModelError> {
    match instruction {
        Instruction::Movz { dest, imm } => {
            let value = m.constant(u64::from(*imm));
            m.set(*dest, value);
        }
        Instruction::AdrpPage { dest, symbol } => {
            let page = m.page(symbol)?;
            m.set(*dest, page);
        }
        Instruction::AddLow12 { dest, src, symbol } => {
            let page = m.get(*src)?;
            let address = m.low12_add(&page, symbol)?;
            m.set(*dest, address);
        }
        Instruction::Bl { target } => {
            let model = external::<M>(target)?;
            let link = m.return_address();
            m.set(X30, link);
            model(m)?;
        }
        Instruction::Stp {
            first,
            second,
            base,
            offset,
        } => {
            let sp = m.get(*base)?;
            let address = m.offset(&sp, i64::from(*offset))?;
            let low = m.get(*first)?;
            m.store64(&address, low)?;
            let high_address = m.offset(&address, 8)?;
            let high = m.get(*second)?;
            m.store64(&high_address, high)?;
            m.set(*base, address);
        }
        Instruction::Ldp {
            first,
            second,
            base,
            offset,
        } => {
            let address = m.get(*base)?;
            let low = m.load64(&address)?;
            m.set(*first, low);
            let high_address = m.offset(&address, 8)?;
            let high = m.load64(&high_address)?;
            m.set(*second, high);
            let next = m.offset(&address, i64::from(*offset))?;
            m.set(*base, next);
        }
        Instruction::Add { dest, left, right } => {
            let (a, b) = (m.get(*left)?, m.get(*right)?);
            let sum = m.add(&a, &b)?;
            m.set(*dest, sum);
        }
        Instruction::Sub { dest, left, right } => {
            let (a, b) = (m.get(*left)?, m.get(*right)?);
            let difference = m.sub(&a, &b)?;
            m.set(*dest, difference);
        }
        Instruction::Mul { dest, left, right } => {
            let (a, b) = (m.get(*left)?, m.get(*right)?);
            let product = m.mul(&a, &b)?;
            m.set(*dest, product);
        }
        Instruction::Mov { dest, src } => {
            let value = m.get(*src)?;
            m.set(*dest, value);
        }
        Instruction::Ret => m.ret()?,
    }
    Ok(())
}
