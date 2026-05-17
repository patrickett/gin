use std::ops::ControlFlow;

use crate::{
    AsmExpr, Binary, Bind, BindValue, Expr, FileAst, FnCall, ForInLoop, FormatPart, FormatString,
    IfCondition, IfExpr, Loop, Range, Return, TagCall, WhenArm, WhenExpr, WhileLoop,
};

use ControlFlow::Continue;

pub trait Folder: Sized {
    fn fold_file_ast(&mut self, ast: &mut FileAst) -> ControlFlow<()> {
        walk_file_ast_mut(self, ast)
    }
    fn fold_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        walk_bind_mut(self, bind)
    }
    fn fold_bind_value(&mut self, val: &mut BindValue) -> ControlFlow<()> {
        walk_bind_value_mut(self, val)
    }
    fn fold_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        walk_expr_mut(self, expr)
    }
    fn fold_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        walk_fn_call_mut(self, call)
    }
    fn fold_binary(&mut self, bin: &mut Binary) -> ControlFlow<()> {
        walk_binary_mut(self, bin)
    }
    fn fold_when_expr(&mut self, when: &mut WhenExpr) -> ControlFlow<()> {
        walk_when_mut(self, when)
    }
    fn fold_when_arm(&mut self, arm: &mut WhenArm) -> ControlFlow<()> {
        walk_when_arm_mut(self, arm)
    }
    fn fold_if_expr(&mut self, ifx: &mut IfExpr) -> ControlFlow<()> {
        walk_if_mut(self, ifx)
    }
    fn fold_if_condition(&mut self, c: &mut IfCondition) -> ControlFlow<()> {
        walk_if_condition_mut(self, c)
    }
    fn fold_loop(&mut self, l: &mut Loop) -> ControlFlow<()> {
        walk_loop_mut(self, l)
    }
    fn fold_while_loop(&mut self, w: &mut WhileLoop) -> ControlFlow<()> {
        walk_while_mut(self, w)
    }
    fn fold_for_in_loop(&mut self, f: &mut ForInLoop) -> ControlFlow<()> {
        walk_for_in_mut(self, f)
    }
    fn fold_tag_call(&mut self, tc: &mut TagCall) -> ControlFlow<()> {
        walk_tag_call_mut(self, tc)
    }
    fn fold_format_string(&mut self, fs: &mut FormatString) -> ControlFlow<()> {
        walk_format_string_mut(self, fs)
    }
    fn fold_format_part(&mut self, p: &mut FormatPart) -> ControlFlow<()> {
        walk_format_part_mut(self, p)
    }
    fn fold_range(&mut self, r: &mut Range) -> ControlFlow<()> {
        walk_range_mut(self, r)
    }
    fn fold_return(&mut self, r: &mut Return) -> ControlFlow<()> {
        walk_return_mut(self, r)
    }
    fn fold_asm_expr(&mut self, a: &mut AsmExpr) -> ControlFlow<()> {
        walk_asm_mut(self, a)
    }
}

pub fn walk_file_ast_mut(v: &mut impl Folder, ast: &mut FileAst) -> ControlFlow<()> {
    for bind in ast.defs.values_mut() {
        v.fold_bind(bind)?;
    }
    for (expr, _) in ast.exprs.iter_mut() {
        v.fold_expr(expr)?;
    }
    Continue(())
}

pub fn walk_bind_mut(v: &mut impl Folder, bind: &mut Bind) -> ControlFlow<()> {
    v.fold_bind_value(bind.value_mut())
}

pub fn walk_bind_value_mut(v: &mut impl Folder, val: &mut BindValue) -> ControlFlow<()> {
    match val {
        BindValue::Expr(e) => v.fold_expr(e),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                v.fold_expr(e)?;
            }
            v.fold_return(ret)
        }
        BindValue::Extern => Continue(()),
    }
}

pub fn walk_expr_mut(v: &mut impl Folder, expr: &mut Expr) -> ControlFlow<()> {
    match expr {
        Expr::FnCall(c) => v.fold_fn_call(c),
        Expr::Binary(b) => v.fold_binary(b),
        Expr::Bind(b) => v.fold_bind(b),
        Expr::When(w) => v.fold_when_expr(w),
        Expr::If(ifx) => v.fold_if_expr(ifx),
        Expr::Loop(l) => v.fold_loop(l),
        Expr::TagCall(tc) => v.fold_tag_call(tc),
        Expr::FormatString(fs) => v.fold_format_string(fs),
        Expr::Range(r) => v.fold_range(r),
        Expr::Asm(a) => v.fold_asm_expr(a),
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                v.fold_expr(e)?;
            }
            Continue(())
        }
        Expr::TupleAlloc { init, .. } => v.fold_expr(init),
        Expr::TupleGet { base, .. } => v.fold_expr(base),
        Expr::TupleSet { base, value, .. } => {
            v.fold_expr(base)?;
            v.fold_expr(value)
        }
        Expr::BufGet { buf, index } => {
            v.fold_expr(buf)?;
            v.fold_expr(index)
        }
        Expr::BufSet { buf, index, value } => {
            v.fold_expr(buf)?;
            v.fold_expr(index)?;
            v.fold_expr(value)
        }
        Expr::Cast { expr: e, .. } => v.fold_expr(e),
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e)
        | Expr::MutArg(e) | Expr::OwnArg(e) => v.fold_expr(e),
        Expr::Lit(_)
        | Expr::SelfRef
        | Expr::AnonymousTag(..)
        | Expr::TypeNominal(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. } => Continue(()),
    }
}

pub fn walk_fn_call_mut(v: &mut impl Folder, call: &mut FnCall) -> ControlFlow<()> {
    if let Some(args) = &mut call.args {
        for arg in args {
            v.fold_expr(arg)?;
        }
    }
    Continue(())
}

pub fn walk_binary_mut(v: &mut impl Folder, bin: &mut Binary) -> ControlFlow<()> {
    v.fold_expr(&mut bin.lhs)?;
    v.fold_expr(&mut bin.rhs)
}

pub fn walk_when_mut(v: &mut impl Folder, when: &mut WhenExpr) -> ControlFlow<()> {
    if let Some(subject) = &mut when.subject {
        v.fold_expr(subject)?;
    }
    for arm in &mut when.arms {
        v.fold_when_arm(arm)?;
    }
    Continue(())
}

pub fn walk_when_arm_mut(v: &mut impl Folder, arm: &mut WhenArm) -> ControlFlow<()> {
    match arm {
        WhenArm::Cond {
            condition, body, ..
        } => {
            v.fold_expr(condition)?;
            v.fold_expr(body)
        }
        WhenArm::Is { body, .. } => v.fold_expr(body),
        WhenArm::Else(body, _) => v.fold_expr(body),
    }
}

pub fn walk_if_mut(v: &mut impl Folder, ifx: &mut IfExpr) -> ControlFlow<()> {
    v.fold_if_condition(&mut ifx.condition)?;
    for e in &mut ifx.body {
        v.fold_expr(e)?;
    }
    v.fold_return(&mut ifx.ret)
}

pub fn walk_if_condition_mut(v: &mut impl Folder, c: &mut IfCondition) -> ControlFlow<()> {
    match c {
        IfCondition::Bool(e) => v.fold_expr(e),
        IfCondition::Pattern { subject, .. } => v.fold_expr(subject),
    }
}

pub fn walk_loop_mut(v: &mut impl Folder, l: &mut Loop) -> ControlFlow<()> {
    match l {
        Loop::While(w) => v.fold_while_loop(w),
        Loop::ForIn(f) => v.fold_for_in_loop(f),
    }
}

pub fn walk_while_mut(v: &mut impl Folder, w: &mut WhileLoop) -> ControlFlow<()> {
    v.fold_expr(&mut w.cond)?;
    for e in &mut w.exprs {
        v.fold_expr(e)?;
    }
    Continue(())
}

pub fn walk_for_in_mut(v: &mut impl Folder, f: &mut ForInLoop) -> ControlFlow<()> {
    v.fold_expr(&mut f.pat)?;
    v.fold_expr(&mut f.iter)?;
    for e in &mut f.exprs {
        v.fold_expr(e)?;
    }
    Continue(())
}

pub fn walk_tag_call_mut(v: &mut impl Folder, tc: &mut TagCall) -> ControlFlow<()> {
    for arg in &mut tc.args {
        v.fold_expr(arg)?;
    }
    Continue(())
}

pub fn walk_format_string_mut(v: &mut impl Folder, fs: &mut FormatString) -> ControlFlow<()> {
    for part in &mut fs.parts {
        v.fold_format_part(part)?;
    }
    Continue(())
}

pub fn walk_format_part_mut(v: &mut impl Folder, p: &mut FormatPart) -> ControlFlow<()> {
    match p {
        FormatPart::Expr(e, _) => v.fold_expr(e),
        FormatPart::Text(_) => Continue(()),
    }
}

pub fn walk_range_mut(v: &mut impl Folder, r: &mut Range) -> ControlFlow<()> {
    v.fold_expr(&mut r.start)?;
    v.fold_expr(&mut r.end)
}

pub fn walk_return_mut(v: &mut impl Folder, r: &mut Return) -> ControlFlow<()> {
    if let Some(e) = &mut r.value {
        v.fold_expr(e)?;
    }
    Continue(())
}

pub fn walk_asm_mut(v: &mut impl Folder, a: &mut AsmExpr) -> ControlFlow<()> {
    for c in &mut a.constraints {
        v.fold_expr(c)?;
    }
    for o in &mut a.operands {
        v.fold_expr(o)?;
    }
    Continue(())
}
