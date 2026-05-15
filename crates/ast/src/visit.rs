use std::ops::ControlFlow;

use crate::{
    AsmExpr, Binary, Bind, BindValue, Expr, FileAst, FnCall, ForInLoop, FormatPart, FormatString,
    IfCondition, IfExpr, Loop, Range, Return, TagCall, WhenArm, WhenExpr, WhileLoop,
};

use ControlFlow::Continue;

pub trait Visitor: Sized {
    fn visit_file_ast(&mut self, ast: &FileAst) -> ControlFlow<()> {
        walk_file_ast(self, ast)
    }
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        walk_bind(self, bind)
    }
    fn visit_bind_value(&mut self, val: &BindValue) -> ControlFlow<()> {
        walk_bind_value(self, val)
    }
    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        walk_expr(self, expr)
    }
    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        walk_fn_call(self, call)
    }
    fn visit_binary(&mut self, bin: &Binary) -> ControlFlow<()> {
        walk_binary(self, bin)
    }
    fn visit_when_expr(&mut self, when: &WhenExpr) -> ControlFlow<()> {
        walk_when(self, when)
    }
    fn visit_when_arm(&mut self, arm: &WhenArm) -> ControlFlow<()> {
        walk_when_arm(self, arm)
    }
    fn visit_if_expr(&mut self, ifx: &IfExpr) -> ControlFlow<()> {
        walk_if(self, ifx)
    }
    fn visit_if_condition(&mut self, c: &IfCondition) -> ControlFlow<()> {
        walk_if_condition(self, c)
    }
    fn visit_loop(&mut self, l: &Loop) -> ControlFlow<()> {
        walk_loop(self, l)
    }
    fn visit_while_loop(&mut self, w: &WhileLoop) -> ControlFlow<()> {
        walk_while(self, w)
    }
    fn visit_for_in_loop(&mut self, f: &ForInLoop) -> ControlFlow<()> {
        walk_for_in(self, f)
    }
    fn visit_tag_call(&mut self, tc: &TagCall) -> ControlFlow<()> {
        walk_tag_call(self, tc)
    }
    fn visit_format_string(&mut self, fs: &FormatString) -> ControlFlow<()> {
        walk_format_string(self, fs)
    }
    fn visit_format_part(&mut self, p: &FormatPart) -> ControlFlow<()> {
        walk_format_part(self, p)
    }
    fn visit_range(&mut self, r: &Range) -> ControlFlow<()> {
        walk_range(self, r)
    }
    fn visit_return(&mut self, r: &Return) -> ControlFlow<()> {
        walk_return(self, r)
    }
    fn visit_asm_expr(&mut self, a: &AsmExpr) -> ControlFlow<()> {
        walk_asm(self, a)
    }
}

pub fn walk_file_ast(v: &mut impl Visitor, ast: &FileAst) -> ControlFlow<()> {
    for bind in ast.defs().values() {
        v.visit_bind(bind)?;
    }
    for (expr, _) in ast.top_level_exprs() {
        v.visit_expr(expr)?;
    }
    Continue(())
}

pub fn walk_bind(v: &mut impl Visitor, bind: &Bind) -> ControlFlow<()> {
    v.visit_bind_value(bind.value())
}

pub fn walk_bind_value(v: &mut impl Visitor, val: &BindValue) -> ControlFlow<()> {
    match val {
        BindValue::Expr(e) => v.visit_expr(e),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                v.visit_expr(e)?;
            }
            v.visit_return(ret)
        }
        BindValue::Extern => Continue(()),
    }
}

pub fn walk_expr(v: &mut impl Visitor, expr: &Expr) -> ControlFlow<()> {
    match expr {
        Expr::FnCall(c) => v.visit_fn_call(c),
        Expr::Binary(b) => v.visit_binary(b),
        Expr::Bind(b) => v.visit_bind(b),
        Expr::When(w) => v.visit_when_expr(w),
        Expr::If(ifx) => v.visit_if_expr(ifx),
        Expr::Loop(l) => v.visit_loop(l),
        Expr::TagCall(tc) => v.visit_tag_call(tc),
        Expr::FormatString(fs) => v.visit_format_string(fs),
        Expr::Range(r) => v.visit_range(r),
        Expr::Asm(a) => v.visit_asm_expr(a),
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                v.visit_expr(e)?;
            }
            Continue(())
        }
        Expr::TupleAlloc { init, .. } => v.visit_expr(init),
        Expr::TupleGet { base, .. } => v.visit_expr(base),
        Expr::TupleSet { base, value, .. } => {
            v.visit_expr(base)?;
            v.visit_expr(value)
        }
        Expr::BufGet { buf, index } => {
            v.visit_expr(buf)?;
            v.visit_expr(index)
        }
        Expr::BufSet { buf, index, value } => {
            v.visit_expr(buf)?;
            v.visit_expr(index)?;
            v.visit_expr(value)
        }
        Expr::Cast { expr: e, .. } => v.visit_expr(e),
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => v.visit_expr(e),
        Expr::Lit(_)
        | Expr::SelfRef(_)
        | Expr::AnonymousTag(..)
        | Expr::TypeNominal(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. } => Continue(()),
    }
}

pub fn walk_fn_call(v: &mut impl Visitor, call: &FnCall) -> ControlFlow<()> {
    if let Some(args) = &call.args {
        for arg in args {
            v.visit_expr(arg)?;
        }
    }
    Continue(())
}

pub fn walk_binary(v: &mut impl Visitor, bin: &Binary) -> ControlFlow<()> {
    v.visit_expr(&bin.lhs)?;
    v.visit_expr(&bin.rhs)
}

pub fn walk_when(v: &mut impl Visitor, when: &WhenExpr) -> ControlFlow<()> {
    if let Some(subject) = &when.subject {
        v.visit_expr(subject)?;
    }
    for arm in &when.arms {
        v.visit_when_arm(arm)?;
    }
    Continue(())
}

pub fn walk_when_arm(v: &mut impl Visitor, arm: &WhenArm) -> ControlFlow<()> {
    match arm {
        WhenArm::Cond {
            condition, body, ..
        } => {
            v.visit_expr(condition)?;
            v.visit_expr(body)
        }
        WhenArm::Is { body, .. } => {
            // pattern is a TypeExpr, visited directly by type-related passes
            v.visit_expr(body)
        }
        WhenArm::Else(body, _) => v.visit_expr(body),
    }
}

pub fn walk_if(v: &mut impl Visitor, ifx: &IfExpr) -> ControlFlow<()> {
    v.visit_if_condition(&ifx.condition)?;
    for e in &ifx.body {
        v.visit_expr(e)?;
    }
    v.visit_return(&ifx.ret)
}

pub fn walk_if_condition(v: &mut impl Visitor, c: &IfCondition) -> ControlFlow<()> {
    match c {
        IfCondition::Bool(e) => v.visit_expr(e),
        IfCondition::Pattern { subject, .. } => {
            // pattern is a TypeExpr, visited directly by type-related passes
            v.visit_expr(subject)
        }
    }
}

pub fn walk_loop(v: &mut impl Visitor, l: &Loop) -> ControlFlow<()> {
    match l {
        Loop::While(w) => v.visit_while_loop(w),
        Loop::ForIn(f) => v.visit_for_in_loop(f),
    }
}

pub fn walk_while(v: &mut impl Visitor, w: &WhileLoop) -> ControlFlow<()> {
    v.visit_expr(&w.cond)?;
    for e in &w.exprs {
        v.visit_expr(e)?;
    }
    Continue(())
}

pub fn walk_for_in(v: &mut impl Visitor, f: &ForInLoop) -> ControlFlow<()> {
    v.visit_expr(&f.pat)?;
    v.visit_expr(&f.iter)?;
    for e in &f.exprs {
        v.visit_expr(e)?;
    }
    Continue(())
}

pub fn walk_tag_call(v: &mut impl Visitor, tc: &TagCall) -> ControlFlow<()> {
    for arg in &tc.args {
        v.visit_expr(arg)?;
    }
    Continue(())
}

pub fn walk_format_string(v: &mut impl Visitor, fs: &FormatString) -> ControlFlow<()> {
    for part in &fs.parts {
        v.visit_format_part(part)?;
    }
    Continue(())
}

pub fn walk_format_part(v: &mut impl Visitor, p: &FormatPart) -> ControlFlow<()> {
    match p {
        FormatPart::Expr(e, _) => v.visit_expr(e),
        FormatPart::Text(_) => Continue(()),
    }
}

pub fn walk_range(v: &mut impl Visitor, r: &Range) -> ControlFlow<()> {
    v.visit_expr(&r.start)?;
    v.visit_expr(&r.end)
}

pub fn walk_return(v: &mut impl Visitor, r: &Return) -> ControlFlow<()> {
    if let Some(e) = &r.value {
        v.visit_expr(e)?;
    }
    Continue(())
}

pub fn walk_asm(v: &mut impl Visitor, a: &AsmExpr) -> ControlFlow<()> {
    for c in &a.constraints {
        v.visit_expr(c)?;
    }
    for o in &a.operands {
        v.visit_expr(o)?;
    }
    Continue(())
}
