//! From a checked Gin body to instructions: selection by [`crate::rules`], and a register
//! allocation that needs no spilling and no coloring.
//!
//! Operations whose operands are all known never arrive here: the recorder already holds
//! their value (the compile-time evaluation rule), so a constant costs one `Movz` and a
//! constant expression costs nothing more.
//!
//! Where arguments arrive, where the result leaves and which registers may be used is the
//! [`Convention`], read from the Gin library; nothing about AAPCS64 is written here. A
//! leaf function needs no frame. Every lowering is checked against the semantics before
//! it is returned.

use typecheck::{CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceFunction, SourceType};
use num_traits::ToPrimitive;

use crate::encode::Convention;
use crate::instruction::{Instruction, Reg};
use crate::program::{Data, check_with};
use crate::rules;
use crate::semantics::ModelError;

/// A register file this large is the most any convention can name (`x0..x30`).
const REGISTERS: usize = 31;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LowerError {
    #[error("`{0}` is not lowered yet")]
    Unsupported(&'static str),
    #[error("{0} parameters; at most {1} arrive in registers")]
    TooManyParameters(usize, usize),
    #[error("a value of {low}..={high} does not fit the signed 64-bit register")]
    NotRepresentable { low: String, high: String },
    #[error("constant {0} needs more than a single `Movz`")]
    ConstantOutOfRange(String),
    #[error("more live values than allocatable registers; there is no spilling")]
    OutOfRegisters,
    #[error("the lowering does not satisfy the semantics: {0}")]
    Unproved(ModelError),
}

/// A lowered function and the contract it was proven against.
#[derive(Debug, PartialEq, Eq)]
pub struct Lowered {
    pub instructions: Vec<Instruction>,
    pub arguments: Vec<(Reg, (i128, i128))>,
    pub result: (i128, i128),
}

fn bounds(ty: &SourceType) -> Result<(i128, i128), LowerError> {
    let SourceType::Integer { lower, upper } = ty else {
        return Err(LowerError::Unsupported("a value that is not an integer"));
    };
    let (low, high) = (lower.to_i128(), upper.to_i128());
    match (low, high) {
        (Some(low), Some(high)) if i64::try_from(low).is_ok() && i64::try_from(high).is_ok() => {
            Ok((low, high))
        }
        _ => Err(LowerError::NotRepresentable {
            low: format!("{lower:?}"),
            high: format!("{upper:?}"),
        }),
    }
}

/// The node the single `return` of the body yields.
fn returned(body: &CheckedSourceBody) -> Option<u32> {
    let root = body.expressions.get(body.root as usize)?;
    let CheckedSourceBodyKind::Block { statements, .. } = &root.kind else {
        return None;
    };
    match body.expressions.get(*statements.first()? as usize)?.kind {
        CheckedSourceBodyKind::Return { value } => Some(value),
        _ => None,
    }
}

pub fn lower(
    convention: &Convention,
    function: &CheckedSourceFunction,
    body: &CheckedSourceBody,
) -> Result<Lowered, LowerError> {
    if function.parameters.len() > convention.arguments.len() {
        return Err(LowerError::TooManyParameters(
            function.parameters.len(),
            convention.arguments.len(),
        ));
    }
    let arguments: Vec<(Reg, (i128, i128))> = function
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| Ok((convention.arguments[index], bounds(ty)?)))
        .collect::<Result<_, LowerError>>()?;
    let result = bounds(&function.result)?;
    let value = returned(body).ok_or(LowerError::Unsupported("a body that is not one return"))?;

    // Only what the returned value depends on is lowered; the arena is postorder, so
    // every operand precedes its user and one reverse pass finds the live nodes.
    let mut live = vec![false; body.expressions.len()];
    live[value as usize] = true;
    let mut uses = vec![0_u32; body.expressions.len()];
    uses[value as usize] = 1;
    for id in (0..body.expressions.len()).rev() {
        if !live[id] {
            continue;
        }
        if let CheckedSourceBodyKind::Binary { left, right, .. } = body.expressions[id].kind {
            for operand in [left, right] {
                live[operand as usize] = true;
                uses[operand as usize] += 1;
            }
        }
    }

    // `references[r]` counts the uses still to come of the value now held in `x_r`.
    let mut references = [0_u32; REGISTERS];
    for (id, expression) in body
        .expressions
        .iter()
        .enumerate()
        .filter(|(id, _)| live[*id])
    {
        if let CheckedSourceBodyKind::Parameter { index } = expression.kind {
            references[usize::from(convention.arguments[index as usize].0)] += uses[id];
        }
    }
    let free = |references: &[u32; REGISTERS]| {
        references[..convention.allocatable]
            .iter()
            .position(|count| *count == 0)
            .ok_or(LowerError::OutOfRegisters)
    };

    let mut registers: Vec<Option<usize>> = vec![None; body.expressions.len()];
    let mut instructions = Vec::new();
    for (id, expression) in body
        .expressions
        .iter()
        .enumerate()
        .filter(|(id, _)| live[*id])
    {
        match &expression.kind {
            CheckedSourceBodyKind::Parameter { index } => {
                registers[id] = Some(usize::from(convention.arguments[*index as usize].0));
            }
            CheckedSourceBodyKind::Integer(constant) => {
                let value = constant
                    .to_i128()
                    .filter(|value| (0..=convention.constant_max).contains(value))
                    .ok_or_else(|| LowerError::ConstantOutOfRange(format!("{constant:?}")))?;
                let dest = free(&references)?;
                references[dest] = uses[id];
                registers[id] = Some(dest);
                instructions.push(Instruction::Movz {
                    dest: Reg(dest as u8),
                    imm: value as u32,
                });
            }
            CheckedSourceBodyKind::Binary {
                operator,
                left,
                right,
            } => {
                for ty in [
                    &expression.ty,
                    &body.expressions[*left as usize].ty,
                    &body.expressions[*right as usize].ty,
                ] {
                    bounds(ty)?;
                }
                let rule = rules::find(*operator);
                let (l, r) = (
                    registers[*left as usize].unwrap(),
                    registers[*right as usize].unwrap(),
                );
                references[l] -= 1;
                references[r] -= 1;
                // The returned value belongs in the result register: take it when free.
                let home = usize::from(convention.result.0);
                let dest = if id == value as usize && references[home] == 0 {
                    home
                } else if references[l] == 0 {
                    l
                } else if rule.commutative && references[r] == 0 {
                    r
                } else {
                    free(&references)?
                };
                references[dest] = uses[id];
                registers[id] = Some(dest);
                instructions.extend((rule.select)(Reg(dest as u8), Reg(l as u8), Reg(r as u8)));
            }
            CheckedSourceBodyKind::Call { .. } => return Err(LowerError::Unsupported("a call")),
            _ => return Err(LowerError::Unsupported("this body construct")),
        }
    }
    let at = registers[value as usize].expect("the returned value was lowered");
    if at != usize::from(convention.result.0) {
        instructions.push(Instruction::Mov {
            dest: convention.result,
            src: Reg(at as u8),
        });
    }
    instructions.push(Instruction::Ret);

    check_with(
        &instructions,
        &Data::new(),
        &arguments,
        (convention.result, result),
    )
    .map_err(LowerError::Unproved)?;
    Ok(Lowered {
        instructions,
        arguments,
        result,
    })
}
