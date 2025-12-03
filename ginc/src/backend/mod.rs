//! Backend is responsible for transforming intermediate representations of code into machine code or assembly language.

use crate::frontend::prelude::Expr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// These are symbolic placeholders that survive until register allocation.
pub struct VReg(pub u32);

#[derive(Debug, Clone)]
pub enum Instruction {
    /// dst = imm64
    LoadImm { dst: VReg, value: i64 },

    /// dst = src1 + src2
    Add { dst: VReg, lhs: VReg, rhs: VReg },

    /// dst = src1 - src2
    Sub { dst: VReg, lhs: VReg, rhs: VReg },

    /// Return a value in a register.
    Ret { src: VReg },
}

#[derive(Debug)]
pub struct IRBuilder {
    pub instructions: Vec<Instruction>,
    next_reg: u32,
}

impl IRBuilder {
    /// Allocate a fresh virtual register.
    pub fn alloc(&mut self) -> VReg {
        let r = VReg(self.next_reg);
        self.next_reg += 1;
        r
    }

    /// Push an instruction.
    pub fn push(&mut self, inst: Instruction) {
        self.instructions.push(inst);
    }
}

impl Expr {
    pub fn lower(&self, _b: &mut IRBuilder) {
        todo!()
        // match self {
        //     Expr::Int(v) => {
        //         let dst = b.alloc();
        //         b.push(Instruction::LoadImm { dst, value: *v });
        //         dst
        //     }

        //     Expr::Add(lhs, rhs) => {
        //         let l = lhs.lower(b);
        //         let r = rhs.lower(b);

        //         let dst = b.alloc();
        //         b.push(Instruction::Add {
        //             dst,
        //             lhs: l,
        //             rhs: r,
        //         });
        //         dst
        //     }

        //     Expr::Sub(lhs, rhs) => {
        //         let l = lhs.lower(b);
        //         let r = rhs.lower(b);

        //         let dst = b.alloc();
        //         b.push(Instruction::Sub {
        //             dst,
        //             lhs: l,
        //             rhs: r,
        //         });
        //         dst
        //     }
        // }
    }
}
