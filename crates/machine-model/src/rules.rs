//! Lowering rules: which instructions compute a Gin operation.
//!
//! A rule is an equation with a precondition,
//!
//! ```text
//! rule add(a in Word, b in Word) where a + b in Word:
//!     a + b  ==  [Add(result, a, b)]
//! ```
//!
//! read: for operands in the register width, when the checked domain proves the exact
//! result also fits, the hardware instruction computes the mathematical result. Gin
//! integers are exact, hardware integers wrap, and the domain is what reconciles them.
//!
//! The rule table is Rust data until recorded bodies can express it in Gin. A rule is not
//! trusted: [`Rule::verify`] runs its instructions through [`crate::semantics::execute`]
//! and compares with the Gin operation as the compiler defines it ([`typecheck::apply`]).

use i256::I256;
use num_traits::ToPrimitive;
use typecheck::{CheckedArithmetic, apply};

use crate::abstract_machine::Abstract;
use crate::concrete::Concrete;
use crate::instruction::{Instruction, Reg, X0, X1};
use crate::program::Data;
use crate::semantics::{Machine, execute};

pub struct Rule {
    pub operator: CheckedArithmetic,
    /// Operands may be swapped, so the allocator may reuse either operand's register.
    pub commutative: bool,
    /// The instructions that leave `left <operator> right` in `dest`.
    pub select: fn(dest: Reg, left: Reg, right: Reg) -> Vec<Instruction>,
}

fn add(dest: Reg, left: Reg, right: Reg) -> Vec<Instruction> {
    vec![Instruction::Add { dest, left, right }]
}

fn sub(dest: Reg, left: Reg, right: Reg) -> Vec<Instruction> {
    vec![Instruction::Sub { dest, left, right }]
}

fn mul(dest: Reg, left: Reg, right: Reg) -> Vec<Instruction> {
    vec![Instruction::Mul { dest, left, right }]
}

/// The Arm64 rules. Every operand and result must lie in the signed 64-bit range.
pub const ARM64: [Rule; 3] = [
    Rule {
        operator: CheckedArithmetic::Add,
        commutative: true,
        select: add,
    },
    Rule {
        operator: CheckedArithmetic::Subtract,
        commutative: false,
        select: sub,
    },
    Rule {
        operator: CheckedArithmetic::Multiply,
        commutative: true,
        select: mul,
    },
];

pub fn find(operator: CheckedArithmetic) -> &'static Rule {
    ARM64
        .iter()
        .find(|rule| rule.operator == operator)
        .expect("every checked arithmetic operator has a rule")
}

/// A pair of operands for which the rule computes the wrong thing.
#[derive(Debug, PartialEq, Eq)]
pub struct Counterexample {
    pub operator: CheckedArithmetic,
    pub left: i64,
    pub right: i64,
    pub expected: i128,
    pub got: String,
}

/// What a successful verification covered.
#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
    /// Samples whose exact result fits: the instructions must match Gin.
    pub applicable: usize,
    /// Samples whose exact result does not fit: the checker must refuse them.
    pub refused: usize,
}

/// Small values exhaustively, then the edges of the signed range and its square root.
pub fn samples() -> Vec<(i64, i64)> {
    let mut wide: Vec<i128> = (-6..=6).collect();
    for edge in [
        1_i128 << 31,
        1 << 32,
        i128::from(i64::MAX),
        i128::from(i64::MIN).abs(),
    ] {
        wide.extend([edge - 1, edge, edge + 1, -edge, -(edge - 1), -(edge + 1)]);
    }
    let mut values: Vec<i64> = wide
        .into_iter()
        .filter_map(|v| i64::try_from(v).ok())
        .collect();
    values.sort_unstable();
    values.dedup();
    values
        .iter()
        .flat_map(|a| values.iter().map(move |b| (*a, *b)))
        .collect()
}

impl Rule {
    pub fn verify(&self, samples: &[(i64, i64)]) -> Result<Verified, Counterexample> {
        let mut verified = Verified {
            applicable: 0,
            refused: 0,
        };
        for &(left, right) in samples {
            let expected = apply(self.operator, I256::from(left), I256::from(right))
                .and_then(|value| value.to_i128())
                .expect("products of 64-bit values fit 128 bits");
            let wrong = |got: String| Counterexample {
                operator: self.operator,
                left,
                right,
                expected,
                got,
            };
            if i64::try_from(expected).is_ok() {
                // Distinct and aliased destinations must both compute the result.
                for dest in [Reg(2), X0] {
                    let mut machine = Concrete::new(&Data::new());
                    machine.set(X0, left as u64);
                    machine.set(X1, right as u64);
                    for instruction in (self.select)(dest, X0, X1) {
                        execute(&mut machine, &instruction).map_err(|e| wrong(e.to_string()))?;
                    }
                    let got = machine.register(dest).map(|value| value as i64);
                    if got.map(i128::from) != Some(expected) {
                        return Err(wrong(format!("{got:?} in {dest}")));
                    }
                }
                verified.applicable += 1;
            } else {
                // The hardware would wrap: the checker must not accept the sequence.
                let mut machine = Abstract::new(&Data::new()).with_arguments(&[
                    (X0, (left.into(), left.into())),
                    (X1, (right.into(), right.into())),
                ]);
                let refused = (self.select)(Reg(2), X0, X1)
                    .iter()
                    .try_for_each(|instruction| execute(&mut machine, instruction))
                    .is_err();
                if !refused {
                    return Err(wrong("accepted a result that wraps".to_owned()));
                }
                verified.refused += 1;
            }
        }
        Ok(verified)
    }
}
