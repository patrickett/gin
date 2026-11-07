//! An emulator for the modelled subset: [`crate::semantics::execute`] over numbers.

use std::collections::{BTreeMap, HashMap};

use crate::instruction::{Reg, SP, X29, X30};
use crate::semantics::{Machine, ModelError, fail};

/// Return address the process entry would hold: `ret` to it ends the run.
pub const ENTRY_LINK: u64 = 0xFEED_0000;

pub struct Concrete {
    registers: [Option<u64>; 32],
    stack: HashMap<u64, u64>,
    data: BTreeMap<String, Vec<u8>>,
    addresses: BTreeMap<String, u64>,
    pub stdout: Vec<u8>,
    pub pc: usize,
    pub halted: bool,
}

impl Concrete {
    pub fn new(data: &BTreeMap<String, Vec<u8>>) -> Self {
        let addresses = data
            .keys()
            .enumerate()
            .map(|(index, name)| (name.clone(), 0x1_0000_4000 + 0x1000 * index as u64))
            .collect();
        let mut registers = [None; 32];
        registers[X29.0 as usize] = Some(0xAAAA_0000);
        registers[X30.0 as usize] = Some(ENTRY_LINK);
        registers[SP.0 as usize] = Some(0x7FFF_0000);
        Self {
            registers,
            stack: HashMap::new(),
            data: data.clone(),
            addresses,
            stdout: Vec::new(),
            pc: 0,
            halted: false,
        }
    }

    pub fn register(&self, register: Reg) -> Option<u64> {
        self.registers[register.0 as usize]
    }

    fn address(&self, symbol: &str) -> Result<u64, ModelError> {
        match self.addresses.get(symbol) {
            Some(address) => Ok(*address),
            None => fail(format!("{symbol} is not a data symbol")),
        }
    }
}

impl Machine for Concrete {
    type Value = u64;

    fn get(&mut self, register: Reg) -> Result<u64, ModelError> {
        match self.registers[register.0 as usize] {
            Some(value) => Ok(value),
            None => fail(format!("read of unset {register}")),
        }
    }

    fn set(&mut self, register: Reg, value: u64) {
        self.registers[register.0 as usize] = Some(value);
    }

    fn clobber(&mut self, registers: std::ops::RangeInclusive<u8>) {
        for register in registers {
            self.registers[register as usize] = None;
        }
    }

    fn constant(&self, value: u64) -> u64 {
        value
    }

    fn page(&self, symbol: &str) -> Result<u64, ModelError> {
        Ok(self.address(symbol)? & !0xFFF)
    }

    fn low12_add(&self, page: &u64, symbol: &str) -> Result<u64, ModelError> {
        Ok(page.wrapping_add(self.address(symbol)? & 0xFFF))
    }

    fn offset(&self, address: &u64, by: i64) -> Result<u64, ModelError> {
        Ok(address.wrapping_add_signed(by))
    }

    fn load64(&mut self, address: &u64) -> Result<u64, ModelError> {
        match self.stack.get(address) {
            Some(value) => Ok(*value),
            None => fail(format!("read of unwritten stack slot {address:#x}")),
        }
    }

    fn store64(&mut self, address: &u64, value: u64) -> Result<(), ModelError> {
        self.stack.insert(*address, value);
        Ok(())
    }

    fn add(&self, left: &u64, right: &u64) -> Result<u64, ModelError> {
        Ok(left.wrapping_add(*right))
    }

    fn sub(&self, left: &u64, right: &u64) -> Result<u64, ModelError> {
        Ok(left.wrapping_sub(*right))
    }

    fn mul(&self, left: &u64, right: &u64) -> Result<u64, ModelError> {
        Ok(left.wrapping_mul(*right))
    }

    fn return_address(&self) -> u64 {
        0x1_0000_0000 + 4 * (self.pc as u64 + 1)
    }

    fn count(&self, length: &u64) -> u64 {
        *length
    }

    fn write_stdout(&mut self, fd: &u64, buffer: &u64, length: &u64) -> Result<(), ModelError> {
        if *fd != 1 {
            return fail(format!("write to fd {fd}; only stdout is modelled"));
        }
        for (symbol, bytes) in &self.data {
            let base = self.addresses[symbol];
            if (base..base + bytes.len() as u64).contains(buffer) {
                let start = (buffer - base) as usize;
                let end = start + *length as usize;
                if end > bytes.len() {
                    return fail(format!("write of {length} bytes reads past {symbol}"));
                }
                self.stdout.extend_from_slice(&bytes[start..end]);
                return Ok(());
            }
        }
        fail(format!(
            "write from {buffer:#x}, which is not a data symbol"
        ))
    }

    fn ret(&mut self) -> Result<(), ModelError> {
        if self.get(X30)? != ENTRY_LINK {
            return fail("ret to an address that is not the caller");
        }
        self.halted = true;
        Ok(())
    }
}
