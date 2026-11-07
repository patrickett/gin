//! Assembly text for the native test: every instruction word comes from the Gin
//! encodings. Instructions that carry a relocation are written as mnemonics, because the
//! Mach-O relocation entry is still the assembler's job (the one remaining gap).

use std::fmt::Write;

use crate::encode::{Arm64, EncodeError};
use crate::instruction::Instruction;
use crate::program::Data;

pub fn assembly(
    program: &[Instruction],
    data: &Data,
    arm64: &Arm64,
) -> Result<String, EncodeError> {
    let mut text = String::from(".globl _main\n.p2align 2\n_main:\n");
    for instruction in program {
        let encoded = arm64.encode(instruction)?;
        let mnemonic = instruction.mnemonic();
        let _ = match encoded.reloc {
            Some(reloc) => writeln!(
                text,
                "    {mnemonic}    // relocation: {}",
                reloc.kind.macho()
            ),
            None => writeln!(text, "    .word 0x{:08X}    // {mnemonic}", encoded.word),
        };
    }
    text.push_str(".section __TEXT,__const\n");
    for (symbol, bytes) in data {
        let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
        let _ = writeln!(text, "{symbol}:\n    .byte {}", list.join(", "));
    }
    Ok(text)
}

/// Assembly for leaf functions that C can call: `_name` for each, no `main`, no data.
pub fn functions(
    functions: &[(&str, &[Instruction])],
    arm64: &Arm64,
) -> Result<String, EncodeError> {
    let mut text = String::new();
    for (name, program) in functions {
        let _ = writeln!(text, ".globl _{name}\n.p2align 2\n_{name}:");
        for instruction in *program {
            let encoded = arm64.encode(instruction)?;
            let mnemonic = instruction.mnemonic();
            let _ = match encoded.reloc {
                Some(reloc) => writeln!(
                    text,
                    "    {mnemonic}    // relocation: {}",
                    reloc.kind.macho()
                ),
                None => writeln!(text, "    .word 0x{:08X}    // {mnemonic}", encoded.word),
            };
        }
    }
    Ok(text)
}
