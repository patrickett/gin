//! The checker: [`crate::semantics::execute`] over facts instead of numbers.
//!
//! Registers hold what is known about them. Reading an unset register, reading past a
//! symbol, calling with an unproven length or leaving without restoring the callee-saved
//! registers are all errors of the same kind: a fact the semantics needed was not there.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::instruction::{Reg, SP, X29, X30};
use crate::semantics::{Machine, ModelError, fail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Const(u64),
    /// The page of a data symbol, from `AdrpPage`.
    Page(String),
    /// The address of a data symbol plus an offset.
    Address {
        symbol: String,
        offset: u64,
    },
    /// `sp` at function entry plus an offset.
    Sp(i64),
    /// The value a callee-saved register held at entry.
    Entry(Reg),
    /// A return address written by a call at this instruction index.
    Return(usize),
    /// The count a call returned: known to exist, value unknown.
    Count,
    /// An integer known to lie in `low..=high`, read as signed 64-bit.
    Range {
        low: i128,
        high: i128,
    },
}

impl Fact {
    /// The bounds of an integer fact, if it is one.
    pub fn bounds(&self) -> Option<(i128, i128)> {
        match self {
            Self::Const(value) => Some((i128::from(*value), i128::from(*value))),
            Self::Range { low, high } => Some((*low, *high)),
            _ => None,
        }
    }
}

/// A result of 64-bit two's-complement arithmetic is the mathematical one exactly when it
/// lies in the signed range; otherwise the hardware wraps and the rule does not apply.
fn signed64(operation: &str, low: i128, high: i128) -> Result<Fact, ModelError> {
    if low < i128::from(i64::MIN) || high > i128::from(i64::MAX) {
        return fail(format!(
            "{operation} may wrap: {low}..={high} is not within i64"
        ));
    }
    Ok(Fact::Range { low, high })
}

type Bounds = (i128, i128);

/// What one instruction read, wrote and destroyed, derived from the semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub reads: BTreeSet<Reg>,
    pub writes: BTreeSet<Reg>,
    pub clobbers: BTreeSet<Reg>,
}

pub struct Abstract {
    registers: [Option<Fact>; 32],
    stack: HashMap<i64, Fact>,
    sizes: BTreeMap<String, usize>,
    pub pc: usize,
    pub halted: bool,
    pub effects: Effects,
}

impl Abstract {
    pub fn new(data: &BTreeMap<String, Vec<u8>>) -> Self {
        let mut registers: [Option<Fact>; 32] = std::array::from_fn(|_| None);
        registers[X29.0 as usize] = Some(Fact::Entry(X29));
        registers[X30.0 as usize] = Some(Fact::Entry(X30));
        registers[SP.0 as usize] = Some(Fact::Sp(0));
        Self {
            registers,
            stack: HashMap::new(),
            sizes: data
                .iter()
                .map(|(name, bytes)| (name.clone(), bytes.len()))
                .collect(),
            pc: 0,
            halted: false,
            effects: Effects::default(),
        }
    }

    /// Registers that hold integers known to lie in the given ranges on entry, as the
    /// parameters of a function do.
    pub fn with_arguments(mut self, arguments: &[(Reg, (i128, i128))]) -> Self {
        for (register, (low, high)) in arguments {
            self.registers[register.0 as usize] = Some(Fact::Range {
                low: *low,
                high: *high,
            });
        }
        self
    }

    pub fn fact(&self, register: Reg) -> Option<&Fact> {
        self.registers[register.0 as usize].as_ref()
    }

    fn integers(
        &self,
        operation: &str,
        left: &Fact,
        right: &Fact,
    ) -> Result<(Bounds, Bounds), ModelError> {
        match (left.bounds(), right.bounds()) {
            (Some(left), Some(right)) => Ok((left, right)),
            _ => fail(format!(
                "{operation} of a value that is not a known integer"
            )),
        }
    }

    fn known(&self, symbol: &str) -> Result<(), ModelError> {
        match self.sizes.contains_key(symbol) {
            true => Ok(()),
            false => fail(format!("{symbol} is not a data symbol")),
        }
    }
}

impl Machine for Abstract {
    type Value = Fact;

    fn get(&mut self, register: Reg) -> Result<Fact, ModelError> {
        match &self.registers[register.0 as usize] {
            Some(fact) => {
                self.effects.reads.insert(register);
                Ok(fact.clone())
            }
            None => fail(format!("read of unset {register}")),
        }
    }

    fn set(&mut self, register: Reg, value: Fact) {
        self.effects.writes.insert(register);
        self.registers[register.0 as usize] = Some(value);
    }

    fn clobber(&mut self, registers: std::ops::RangeInclusive<u8>) {
        for register in registers {
            if self.registers[register as usize].take().is_some() {
                self.effects.clobbers.insert(Reg(register));
            }
        }
    }

    fn constant(&self, value: u64) -> Fact {
        Fact::Const(value)
    }

    fn page(&self, symbol: &str) -> Result<Fact, ModelError> {
        self.known(symbol)?;
        Ok(Fact::Page(symbol.to_owned()))
    }

    fn low12_add(&self, page: &Fact, symbol: &str) -> Result<Fact, ModelError> {
        match page {
            Fact::Page(of) if of == symbol => Ok(Fact::Address {
                symbol: symbol.to_owned(),
                offset: 0,
            }),
            _ => fail(format!(
                "AddLow12 of {symbol} needs the page of {symbol} from AdrpPage"
            )),
        }
    }

    fn offset(&self, address: &Fact, by: i64) -> Result<Fact, ModelError> {
        match address {
            Fact::Sp(base) => Ok(Fact::Sp(base + by)),
            _ => fail("only sp-relative offsets are modelled"),
        }
    }

    fn load64(&mut self, address: &Fact) -> Result<Fact, ModelError> {
        let Fact::Sp(slot) = address else {
            return fail("only the stack is loaded from");
        };
        match self.stack.get(slot) {
            Some(fact) => Ok(fact.clone()),
            None => fail(format!("read of unwritten stack slot sp{slot:+}")),
        }
    }

    fn store64(&mut self, address: &Fact, value: Fact) -> Result<(), ModelError> {
        let Fact::Sp(slot) = address else {
            return fail("only the stack is stored to");
        };
        self.stack.insert(*slot, value);
        Ok(())
    }

    fn add(&self, left: &Fact, right: &Fact) -> Result<Fact, ModelError> {
        let ((a, b), (c, d)) = self.integers("add", left, right)?;
        signed64("add", a + c, b + d)
    }

    fn sub(&self, left: &Fact, right: &Fact) -> Result<Fact, ModelError> {
        let ((a, b), (c, d)) = self.integers("sub", left, right)?;
        signed64("sub", a - d, b - c)
    }

    fn mul(&self, left: &Fact, right: &Fact) -> Result<Fact, ModelError> {
        let ((a, b), (c, d)) = self.integers("mul", left, right)?;
        let corners = [a * c, a * d, b * c, b * d];
        let low = corners.into_iter().min().expect("four corners");
        let high = corners.into_iter().max().expect("four corners");
        signed64("mul", low, high)
    }

    fn return_address(&self) -> Fact {
        Fact::Return(self.pc)
    }

    fn count(&self, _: &Fact) -> Fact {
        Fact::Count
    }

    fn write_stdout(&mut self, fd: &Fact, buffer: &Fact, length: &Fact) -> Result<(), ModelError> {
        if *fd != Fact::Const(1) {
            return fail("write fd is not proven to be 1 (stdout)");
        }
        let (Fact::Address { symbol, offset }, Fact::Const(length)) = (buffer, length) else {
            return fail("write needs a data address and a constant length");
        };
        let size = self.sizes[symbol];
        if *offset + *length > size as u64 {
            return fail(format!(
                "write of {length} bytes reads past {symbol} ({size} bytes)"
            ));
        }
        Ok(())
    }

    fn ret(&mut self) -> Result<(), ModelError> {
        let restored = self.fact(X30) == Some(&Fact::Entry(X30))
            && self.fact(X29) == Some(&Fact::Entry(X29))
            && self.fact(SP) == Some(&Fact::Sp(0));
        if !restored {
            return fail("ret without restoring x29, x30 and sp (callee-saved)");
        }
        self.halted = true;
        Ok(())
    }
}
