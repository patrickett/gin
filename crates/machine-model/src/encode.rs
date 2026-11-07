//! Encoding through Gin. The words come from evaluating `gin/arm64.gin`; this file only
//! maps an instruction value to the Gin function and fields that describe it.

use i256::I256;
use num_traits::ToPrimitive;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceModule, Limits,
    check_module, evaluate_body,
};

use crate::instruction::{Instruction, Reg};

const LIBRARY: &str = include_str!("../gin/arm64.gin");
const FUEL: u64 = 1_000;

/// The library is trusted source of this crate and larger than a typical module: its
/// machine model is checked once, so it gets a bigger work budget than the default.
const LIBRARY_LIMITS: Limits = Limits {
    work_max: 10_000_000,
    flow_work_max: 100_000_000,
    definitions_max: 4096,
    parameters_max: 64,
    target: typecheck::Target::host(),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocKind {
    Page21,
    PageOff12,
    Branch26,
}

impl RelocKind {
    /// The Mach-O relocation type name.
    pub fn macho(self) -> &'static str {
        match self {
            Self::Page21 => "ARM64_RELOC_PAGE21",
            Self::PageOff12 => "ARM64_RELOC_PAGEOFF12",
            Self::Branch26 => "ARM64_RELOC_BRANCH26",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reloc {
    pub kind: RelocKind,
    pub symbol: String,
}

/// One instruction's word, plus the relocation that fills a field not known yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub word: u32,
    pub reloc: Option<Reloc>,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("the arm64 Gin library does not check: {0}")]
    Library(String),
    #[error("the arm64 Gin library has no function `{0}`")]
    MissingFunction(&'static str),
    #[error("`{function}`: a field is outside its domain")]
    OutOfDomain { function: &'static str },
    #[error("pair offset {0} is not a multiple of 8 in -512 to 504")]
    PairOffset(i32),
}

struct NoBodies;

impl BodyResolver for NoBodies {
    fn body(&self, _: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        None
    }
}

/// How a function meets its caller, as the Gin library states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Convention {
    /// Where the n-th argument arrives.
    pub arguments: Vec<Reg>,
    /// Where the result leaves.
    pub result: Reg,
    /// Registers `x0..x{allocatable-1}` may hold values in a leaf function.
    pub allocatable: usize,
    /// The largest constant a single `movz` loads.
    pub constant_max: i128,
}

/// The checked Arm64 library.
pub struct Arm64 {
    module: CheckedSourceModule,
}

impl Arm64 {
    pub fn load() -> Result<Self, EncodeError> {
        Self::from_source(LIBRARY)
    }

    /// A library other than the shipped one: another encoding set or another convention.
    pub fn from_source(library: &str) -> Result<Self, EncodeError> {
        let file = parser::parse_strict(library)
            .map_err(|diagnostics| EncodeError::Library(format!("{diagnostics:?}")))?;
        let module = check_module("arm64", &file, &[], LIBRARY_LIMITS)
            .map_err(|error| EncodeError::Library(format!("{error:?}")))?;
        Ok(Self { module })
    }

    /// The checked capsule, so other Gin modules can import and fold these encodings.
    pub fn module(&self) -> &CheckedSourceModule {
        &self.module
    }

    /// The machine model of the library, run by the compile-time evaluator.
    pub fn machine(&self) -> crate::gin_semantics::GinMachine<'_> {
        crate::gin_semantics::GinMachine::new(self.module.checked())
    }

    /// Evaluate a library function on known arguments.
    fn value(&self, function: &'static str, arguments: &[i64]) -> Result<i128, EncodeError> {
        let checked = self.module.checked();
        let index = checked
            .functions
            .iter()
            .position(|candidate| candidate.name == function)
            .ok_or(EncodeError::MissingFunction(function))?;
        let body = checked.bodies[index]
            .as_ref()
            .ok_or_else(|| EncodeError::Library(format!("`{function}` has no recorded body")))?;
        let arguments: Vec<I256> = arguments.iter().map(|value| I256::from(*value)).collect();
        let value = evaluate_body(body, &arguments, &NoBodies, &mut { FUEL })
            .ok_or(EncodeError::OutOfDomain { function })?;
        value.to_i128().ok_or(EncodeError::OutOfDomain { function })
    }

    fn word(&self, function: &'static str, arguments: &[i64]) -> Result<u32, EncodeError> {
        u32::try_from(self.value(function, arguments)?)
            .map_err(|_| EncodeError::OutOfDomain { function })
    }

    fn register(&self, function: &'static str, arguments: &[i64]) -> Result<Reg, EncodeError> {
        u8::try_from(self.value(function, arguments)?)
            .ok()
            .filter(|register| *register <= 30)
            .map(Reg)
            .ok_or(EncodeError::OutOfDomain { function })
    }

    /// The calling convention, read from the Gin library: which registers carry the
    /// arguments and the result, how many are allocatable, and the largest constant one
    /// `movz` loads. Each is a Gin function whose result domain is a register field, so
    /// the checker has already proven it names a register.
    pub fn convention(&self) -> Result<Convention, EncodeError> {
        let count = self.value("argument_count", &[])?;
        let arguments = (0..count as i64)
            .map(|index| self.register("argument_register", &[index]))
            .collect::<Result<Vec<_>, _>>()?;
        let allocatable = usize::try_from(self.value("allocatable", &[])?).map_err(|_| {
            EncodeError::OutOfDomain {
                function: "allocatable",
            }
        })?;
        let result = self.register("result_register", &[])?;
        if arguments
            .iter()
            .chain([&result])
            .any(|register| usize::from(register.0) >= allocatable)
        {
            return Err(EncodeError::Library(
                "the convention names a register outside `allocatable`".into(),
            ));
        }
        Ok(Convention {
            arguments,
            result,
            allocatable,
            constant_max: self.value("constant_max", &[])?,
        })
    }

    pub fn encode(&self, instruction: &Instruction) -> Result<Encoded, EncodeError> {
        let plain = |word| Encoded { word, reloc: None };
        let relocated = |word, kind, symbol: &str| Encoded {
            word,
            reloc: Some(Reloc {
                kind,
                symbol: symbol.to_owned(),
            }),
        };
        Ok(match instruction {
            Instruction::Movz { dest, imm } => {
                plain(self.word("movz", &[dest.0.into(), (*imm).into()])?)
            }
            Instruction::AdrpPage { dest, symbol } => relocated(
                self.word("adrp", &[dest.0.into()])?,
                RelocKind::Page21,
                symbol,
            ),
            Instruction::AddLow12 { dest, src, symbol } => relocated(
                self.word("add_immediate", &[dest.0.into(), src.0.into()])?,
                RelocKind::PageOff12,
                symbol,
            ),
            Instruction::Bl { target } => {
                relocated(self.word("branch_link", &[])?, RelocKind::Branch26, target)
            }
            Instruction::Stp {
                first,
                second,
                base,
                offset,
            } => plain(self.word(
                "store_pair_pre",
                &[
                    first.0.into(),
                    second.0.into(),
                    base.0.into(),
                    imm7(*offset)?,
                ],
            )?),
            Instruction::Ldp {
                first,
                second,
                base,
                offset,
            } => plain(self.word(
                "load_pair_post",
                &[
                    first.0.into(),
                    second.0.into(),
                    base.0.into(),
                    imm7(*offset)?,
                ],
            )?),
            Instruction::Add { dest, left, right } => plain(self.word(
                "add_register",
                &[dest.0.into(), left.0.into(), right.0.into()],
            )?),
            Instruction::Sub { dest, left, right } => plain(self.word(
                "sub_register",
                &[dest.0.into(), left.0.into(), right.0.into()],
            )?),
            Instruction::Mul { dest, left, right } => {
                plain(self.word("multiply", &[dest.0.into(), left.0.into(), right.0.into()])?)
            }
            Instruction::Mov { dest, src } => {
                plain(self.word("move_register", &[dest.0.into(), src.0.into()])?)
            }
            Instruction::Ret => plain(self.word("ret", &[])?),
        })
    }
}

/// A pair offset as the 7-bit two's-complement field of a scaled offset. This is generic
/// field packing, not knowledge of an instruction.
fn imm7(offset: i32) -> Result<i64, EncodeError> {
    if offset % 8 != 0 || !(-512..=504).contains(&offset) {
        return Err(EncodeError::PairOffset(offset));
    }
    Ok(i64::from((offset / 8) & 0x7f))
}
