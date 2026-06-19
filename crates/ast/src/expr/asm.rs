use crate::Expr;
use crate::Typed;
use internment::Intern;

/// How a register operand participates in an inline assembly block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandKind {
    Input,
    Output,
    InOut,
    LateOut,
}

/// A single operand slot in the assembly block.
/// Positionally paired with entries in [`AsmExpr::operand_values`]:
///   - `Input` / `InOut` → a runtime value is consumed
///   - `Output` / `LateOut` → no runtime value (result comes via the return)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperandSpec {
    pub kind: OperandKind,
    pub register: Intern<String>,
}

/// A register or memory location clobbered by the assembly block.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClobberSpec {
    Register(Intern<String>),
    Memory,
}

/// A structured inline assembly expression.
///
/// The `template`, `operands`, and `clobbers` fields are populated at compile
/// time by the `fold_compile_time_binds` pass from a const-folded `AsmSpec`.
/// The parser only fills `spec_expr` (reference to the `:=` bind) and
/// `operand_values` (the runtime arguments to `asm()`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AsmExpr {
    /// Assembly template string (e.g. "svc #0x80").
    /// Empty until filled by the compile-time folding pass.
    pub template: Intern<String>,
    /// Operand descriptors — filled by the compile-time folding pass
    /// from the AsmSpec constant.
    pub operands: Vec<OperandSpec>,
    /// Clobber descriptors — filled by the compile-time folding pass
    /// from the AsmSpec constant.
    pub clobbers: Vec<ClobberSpec>,
    /// Runtime operand expressions — the actual values passed to `asm()`.
    /// Positionally paired with `operands`: `Input`/`InOut` consume a value,
    /// `Output`/`LateOut` do not.
    pub operand_values: Vec<Typed<Expr>>,
    /// Reference to the AsmSpec expression (a `:=` bind with a `const_value`).
    /// Used by the folding pass to populate `template`/`operands`/`clobbers`.
    pub spec_expr: Option<Box<Typed<Expr>>>,
}
