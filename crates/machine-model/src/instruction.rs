use std::fmt;

/// A register number: `x0`..`x30`, and `sp` as 31.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg(pub u8);

pub const X0: Reg = Reg(0);
pub const X1: Reg = Reg(1);
pub const X2: Reg = Reg(2);
pub const X29: Reg = Reg(29);
pub const X30: Reg = Reg(30);
pub const SP: Reg = Reg(31);

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            31 => f.write_str("sp"),
            n => write!(f, "x{n}"),
        }
    }
}

/// One instruction form, as data. Field ranges are not checked here: the Gin encoding's
/// parameter domains are the single statement of what each field may hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Movz {
        dest: Reg,
        imm: u32,
    },
    AdrpPage {
        dest: Reg,
        symbol: String,
    },
    AddLow12 {
        dest: Reg,
        src: Reg,
        symbol: String,
    },
    Bl {
        target: String,
    },
    /// `stp first, second, [base, #offset]!`
    Stp {
        first: Reg,
        second: Reg,
        base: Reg,
        offset: i32,
    },
    /// `ldp first, second, [base], #offset`
    Ldp {
        first: Reg,
        second: Reg,
        base: Reg,
        offset: i32,
    },
    /// `add dest, left, right`: 64-bit two's-complement, wrapping.
    Add {
        dest: Reg,
        left: Reg,
        right: Reg,
    },
    Sub {
        dest: Reg,
        left: Reg,
        right: Reg,
    },
    /// `mul dest, left, right`: the low 64 bits of the product.
    Mul {
        dest: Reg,
        left: Reg,
        right: Reg,
    },
    Mov {
        dest: Reg,
        src: Reg,
    },
    Ret,
}

impl Instruction {
    pub fn movz(dest: Reg, imm: u32) -> Self {
        Self::Movz { dest, imm }
    }

    pub fn adrp_page(dest: Reg, symbol: &str) -> Self {
        Self::AdrpPage {
            dest,
            symbol: symbol.to_owned(),
        }
    }

    pub fn add_low12(dest: Reg, src: Reg, symbol: &str) -> Self {
        Self::AddLow12 {
            dest,
            src,
            symbol: symbol.to_owned(),
        }
    }

    pub fn bl(target: &str) -> Self {
        Self::Bl {
            target: target.to_owned(),
        }
    }

    /// The assembler spelling. Used for relocation-bearing instructions in emitted
    /// assembly and to cross-check the Gin encodings against the system assembler.
    pub fn mnemonic(&self) -> String {
        match self {
            Self::Movz { dest, imm } => format!("mov {dest}, #{imm}"),
            Self::AdrpPage { dest, symbol } => format!("adrp {dest}, {symbol}@PAGE"),
            Self::AddLow12 { dest, src, symbol } => {
                format!("add {dest}, {src}, {symbol}@PAGEOFF")
            }
            Self::Bl { target } => format!("bl {target}"),
            Self::Stp {
                first,
                second,
                base,
                offset,
            } => {
                format!("stp {first}, {second}, [{base}, #{offset}]!")
            }
            Self::Ldp {
                first,
                second,
                base,
                offset,
            } => {
                format!("ldp {first}, {second}, [{base}], #{offset}")
            }
            Self::Add { dest, left, right } => format!("add {dest}, {left}, {right}"),
            Self::Sub { dest, left, right } => format!("sub {dest}, {left}, {right}"),
            Self::Mul { dest, left, right } => format!("mul {dest}, {left}, {right}"),
            Self::Mov { dest, src } => format!("mov {dest}, {src}"),
            Self::Ret => "ret".to_owned(),
        }
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Movz { dest, imm } => write!(f, "Movz({dest}, {imm})"),
            Self::AdrpPage { dest, symbol } => write!(f, "AdrpPage({dest}, {symbol})"),
            Self::AddLow12 { dest, src, symbol } => write!(f, "AddLow12({dest}, {src}, {symbol})"),
            Self::Bl { target } => write!(f, "Bl({target})"),
            Self::Stp {
                first,
                second,
                base,
                offset,
            } => {
                write!(f, "Stp({first}, {second}, {base}, {offset})")
            }
            Self::Ldp {
                first,
                second,
                base,
                offset,
            } => {
                write!(f, "Ldp({first}, {second}, {base}, {offset})")
            }
            Self::Add { dest, left, right } => write!(f, "Add({dest}, {left}, {right})"),
            Self::Sub { dest, left, right } => write!(f, "Sub({dest}, {left}, {right})"),
            Self::Mul { dest, left, right } => write!(f, "Mul({dest}, {left}, {right})"),
            Self::Mov { dest, src } => write!(f, "Mov({dest}, {src})"),
            Self::Ret => f.write_str("Ret"),
        }
    }
}
