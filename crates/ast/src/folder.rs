use std::ops::ControlFlow;

use crate::{
    AsmExpr, Binary, Bind, BindValue, Expr, FileAst, FnCall, ForInLoop, FormatPart, FormatString,
    IfExpr, Loop, Range, Return, SpanId, TagCall, WhenArm, WhenExpr, WhileLoop,
};

use ControlFlow::Continue;

macro_rules! walk_expr_body {
    ($v:expr, $expr:expr) => {
        match $expr {
            Expr::FnCall(c) => $v.visit_fn_call(c),
            Expr::Binary(b) => $v.visit_binary(b),
            Expr::Bind(b) => $v.visit_bind(b),
            Expr::When(w) => $v.visit_when_expr(w),
            Expr::If(ifx) => $v.visit_if_expr(ifx),
            Expr::Loop(l) => $v.visit_loop(l),
            Expr::TagCall(tc) => $v.visit_tag_call(tc),
            Expr::FormatString(fs) => $v.visit_format_string(fs),
            Expr::Range(r) => $v.visit_range(r),
            Expr::Asm(a) => $v.visit_asm_expr(a),
            Expr::RecordLit(fields) => {
                for (_, e) in fields {
                    $v.visit_expr(e)?;
                }
                Continue(())
            }
            Expr::TupleLit(elems) | Expr::List(elems) => {
                for e in elems {
                    $v.visit_expr(e)?;
                }
                Continue(())
            }
            Expr::TupleAlloc { init, size } => {
                $v.visit_expr(init)?;
                $v.visit_expr(size)
            }
            Expr::TupleGet { base, .. } | Expr::RecordGet { base, .. } => $v.visit_expr(base),
            Expr::TupleSet { base, value, .. } | Expr::RecordSet { base, value, .. } => {
                $v.visit_expr(base)?;
                $v.visit_expr(value)
            }
            Expr::Destructure { value, .. } => $v.visit_expr(value),
            Expr::BufGet { buf, index } => {
                $v.visit_expr(buf)?;
                $v.visit_expr(index)
            }
            Expr::BufSet { buf, index, value } => {
                $v.visit_expr(buf)?;
                $v.visit_expr(index)?;
                $v.visit_expr(value)
            }
            Expr::Cast { expr: e, .. } => $v.visit_expr(e),
            Expr::TakePtr(e)
            | Expr::Ref { inner: e, .. }
            | Expr::ConsumeArg(e)
            | Expr::Eat(e)
            | Expr::Deref(e)
            | Expr::Negate(e) => $v.visit_expr(e),
            Expr::Lit(_)
            | Expr::SelfRef
            | Expr::AnonymousTag(..) => Continue(()),
        }
    };
}

macro_rules! walk_expr_child_nodes {
    ($expr:expr) => {{
        match $expr {
            Expr::FnCall(call) => {
                if let Some(args) = borrow!(call.args) {
                    for arg in args { child!(arg); }
                }
            }
            Expr::Binary(binary) => { child!(binary.lhs); child!(binary.rhs); }
            Expr::Bind(bind) => match borrow!(bind.value) {
                BindValue::Expr(expr) => child!(expr),
                BindValue::Body { exprs, ret } => {
                    for expr in exprs { child!(expr); }
                    if let Some(expr) = borrow!(ret.value) { child!(expr); }
                }
                BindValue::Extern | BindValue::Unassigned => {}
            },
            Expr::When(when) => {
                if let Some(subject) = borrow!(when.subject) { child!(subject); }
                for arm in borrow!(when.arms) {
                    match arm {
                        WhenArm::Cond { condition, body, .. } => {
                            child!(condition);
                            child!(body);
                        }
                        WhenArm::Is { body, .. } | WhenArm::Else(body, _) => child!(body),
                    }
                }
            }
            Expr::If(if_expr) => {
                child!(if_expr.subject);
                for expr in borrow!(if_expr.body) { child!(expr); }
                if let Some(expr) = borrow!(if_expr.ret.value) { child!(expr); }
            }
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(while_loop) => {
                    child!(while_loop.cond);
                    for expr in borrow!(while_loop.exprs) { child!(expr); }
                }
                Loop::ForIn(for_loop) => {
                    child!(for_loop.pat);
                    child!(for_loop.iter);
                    for expr in borrow!(for_loop.exprs) { child!(expr); }
                }
            },
            Expr::TagCall(call) => {
                for arg in borrow!(call.args) { child!(arg); }
            }
            Expr::FormatString(format) => {
                for part in borrow!(format.parts) {
                    if let FormatPart::Expr(expr, _) = part { child!(expr); }
                }
            }
            Expr::Range(range) => { child!(range.start); child!(range.end); }
            Expr::Asm(asm) => {
                if let Some(spec) = borrow!(asm.spec_expr) { child!(spec); }
                for operand in borrow!(asm.operand_values) { child!(operand); }
            }
            Expr::RecordLit(fields) => {
                for (_, expr) in fields { child!(expr); }
            }
            Expr::TupleLit(exprs) | Expr::List(exprs) => {
                for expr in exprs { child!(expr); }
            }
            Expr::TupleAlloc { init, size } => { child!(init); child!(size); }
            Expr::TupleGet { base, .. }
            | Expr::RecordGet { base, .. }
            | Expr::Cast { expr: base, .. }
            | Expr::TakePtr(base)
            | Expr::Ref { inner: base, .. }
            | Expr::ConsumeArg(base)
            | Expr::Eat(base)
            | Expr::Deref(base)
            | Expr::Negate(base)
            | Expr::Destructure { value: base, .. } => child!(base),
            Expr::TupleSet { base, value, .. } | Expr::RecordSet { base, value, .. } => {
                child!(base);
                child!(value);
            }
            Expr::BufGet { buf, index } => { child!(buf); child!(index); }
            Expr::BufSet { buf, index, value } => {
                child!(buf);
                child!(index);
                child!(value);
            }
            Expr::Lit(_)
            | Expr::SelfRef
            | Expr::AnonymousTag(_) => {}
        }
        Continue(())
    }};
}

pub fn walk_typed_expr_children<'a>(
    expr: &'a Expr,
    visit: &mut impl FnMut(&'a crate::expr::Typed<Expr>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    macro_rules! borrow { ($value:expr) => { &$value }; }
    macro_rules! child { ($child:expr) => { visit(&*$child)? }; }
    walk_expr_child_nodes!(expr)
}

pub fn walk_typed_expr_children_mut(
    expr: &mut Expr,
    visit: &mut impl FnMut(&mut crate::expr::Typed<Expr>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    macro_rules! borrow { ($value:expr) => { &mut $value }; }
    macro_rules! child { ($child:expr) => { visit(&mut *$child)? }; }
    walk_expr_child_nodes!(expr)
}

pub fn walk_expr_children<'a>(
    expr: &'a Expr,
    visit: &mut impl FnMut(SpanId, &'a Expr) -> ControlFlow<()>,
) -> ControlFlow<()> {
    walk_typed_expr_children(expr, &mut |child| visit(child.span_id, &child.value))
}

pub fn walk_expr_children_mut(
    expr: &mut Expr,
    visit: &mut impl FnMut(SpanId, &mut Expr) -> ControlFlow<()>,
) -> ControlFlow<()> {
    walk_typed_expr_children_mut(expr, &mut |child| visit(child.span_id, &mut child.value))
}

// Most walk bodies are textually identical for Visitor and Folder (auto-deref
// handles child references). Only two pairs differ:
//
//   walk_file_ast  — values() vs values_mut(), iter() vs iter_mut()
//   walk_bind      — value()  vs value_mut()

macro_rules! walk_fn {
    // Same body for both Visitor and Folder.
    // `$v` / `$p` bound from call-site so hygiene aligns.
    (@s $name:ident, $fname:ident, $v:ident, $p:ident, $T:ty, $body:block) => {
        pub fn $name($v: &mut impl Visitor, $p: &$T) -> ControlFlow<()> $body
        pub fn $fname($v: &mut impl Folder, $p: &mut $T) -> ControlFlow<()> $body
    };
    // Different bodies.
    (@d $name:ident, $fname:ident, $v:ident, $p:ident, $T:ty, $a:block, $b:block) => {
        pub fn $name($v: &mut impl Visitor, $p: &$T) -> ControlFlow<()> $a
        pub fn $fname($v: &mut impl Folder, $p: &mut $T) -> ControlFlow<()> $b
    };
}

walk_fn!(@d walk_file_ast, walk_file_ast_mut, v, ast, FileAst, {
    for bind in ast.defs.values()    { v.visit_bind(bind)?; }
    for (expr, _) in ast.exprs.iter() { v.visit_expr(expr)?; }
    Continue(())
}, {
    for bind in ast.defs.values_mut()     { v.visit_bind(bind)?; }
    for (expr, _) in ast.exprs.iter_mut() { v.visit_expr(expr)?; }
    Continue(())
});

walk_fn!(@d walk_bind, walk_bind_mut, v, bind, Bind,
    { v.visit_bind_value(&bind.value) },
    { v.visit_bind_value(bind.value_mut()) }
);

walk_fn!(@s walk_bind_value, walk_bind_value_mut, v, val, BindValue, {
    match val {
        BindValue::Expr(e) => v.visit_expr(e),
        BindValue::Body { exprs, ret } => {
            for e in exprs { v.visit_expr(e)?; }
            v.visit_return(ret)
        }
        BindValue::Extern | BindValue::Unassigned => Continue(()),
    }
});

walk_fn!(@s walk_expr, walk_expr_mut, v, expr, Expr, { walk_expr_body!(v, expr) });

walk_fn!(@d walk_fn_call, walk_fn_call_mut, v, call, FnCall, {
    if let Some(args) = &call.args { for arg in args { v.visit_expr(arg)?; } }
    Continue(())
}, {
    if let Some(args) = &mut call.args { for arg in args { v.visit_expr(arg)?; } }
    Continue(())
});

walk_fn!(@d walk_binary, walk_binary_mut, v, bin, Binary,
    { v.visit_expr(&bin.lhs)?; v.visit_expr(&bin.rhs) },
    { v.visit_expr(&mut bin.lhs)?; v.visit_expr(&mut bin.rhs) }
);

walk_fn!(@d walk_when, walk_when_mut, v, when, WhenExpr, {
    if let Some(subject) = &when.subject { v.visit_expr(subject)?; }
    for arm in &when.arms { v.visit_when_arm(arm)?; }
    Continue(())
}, {
    if let Some(subject) = &mut when.subject { v.visit_expr(subject)?; }
    for arm in &mut when.arms { v.visit_when_arm(arm)?; }
    Continue(())
});

walk_fn!(@s walk_when_arm, walk_when_arm_mut, v, arm, WhenArm, {
    match arm {
        WhenArm::Cond { condition, body, .. } => {
            v.visit_expr(condition)?;
            v.visit_expr(body)
        }
        WhenArm::Is { body, .. } => v.visit_expr(body),
        WhenArm::Else(body, _) => v.visit_expr(body),
    }
});

walk_fn!(@d walk_if, walk_if_mut, v, ifx, IfExpr, {
    v.visit_expr(&ifx.subject)?;
    for e in &ifx.body { v.visit_expr(e)?; }
    v.visit_return(&ifx.ret)
}, {
    v.visit_expr(&mut ifx.subject)?;
    for e in &mut ifx.body { v.visit_expr(e)?; }
    v.visit_return(&mut ifx.ret)
});

walk_fn!(@s walk_loop, walk_loop_mut, v, l, Loop, {
    match l {
        Loop::While(w) => v.visit_while_loop(w),
        Loop::ForIn(f) => v.visit_for_in_loop(f),
    }
});

walk_fn!(@d walk_while, walk_while_mut, v, w, WhileLoop, {
    v.visit_expr(&w.cond)?;
    for e in &w.exprs { v.visit_expr(e)?; }
    Continue(())
}, {
    v.visit_expr(&mut w.cond)?;
    for e in &mut w.exprs { v.visit_expr(e)?; }
    Continue(())
});

walk_fn!(@d walk_for_in, walk_for_in_mut, v, f, ForInLoop, {
    v.visit_expr(&f.pat)?;
    v.visit_expr(&f.iter)?;
    for e in &f.exprs { v.visit_expr(e)?; }
    Continue(())
}, {
    v.visit_expr(&mut f.pat)?;
    v.visit_expr(&mut f.iter)?;
    for e in &mut f.exprs { v.visit_expr(e)?; }
    Continue(())
});

walk_fn!(@d walk_tag_call, walk_tag_call_mut, v, tc, TagCall,
    { for arg in &tc.args { v.visit_expr(arg)?; } Continue(()) },
    { for arg in &mut tc.args { v.visit_expr(arg)?; } Continue(()) }
);

walk_fn!(@d walk_format_string, walk_format_string_mut, v, fs, FormatString,
    { for part in &fs.parts { v.visit_format_part(part)?; } Continue(()) },
    { for part in &mut fs.parts { v.visit_format_part(part)?; } Continue(()) }
);

walk_fn!(@s walk_format_part, walk_format_part_mut, v, p, FormatPart, {
    match p {
        FormatPart::Expr(e, _) => v.visit_expr(e),
        FormatPart::Text(_) => Continue(()),
    }
});

walk_fn!(@d walk_range, walk_range_mut, v, r, Range,
    { v.visit_expr(&r.start)?; v.visit_expr(&r.end) },
    { v.visit_expr(&mut r.start)?; v.visit_expr(&mut r.end) }
);

walk_fn!(@d walk_return, walk_return_mut, v, r, Return,
    { if let Some(e) = &r.value { v.visit_expr(e)?; } Continue(()) },
    { if let Some(e) = &mut r.value { v.visit_expr(e)?; } Continue(()) }
);

walk_fn!(@d walk_asm, walk_asm_mut, v, a, AsmExpr, {
    if let Some(spec) = &a.spec_expr { v.visit_expr(spec)?; }
    for o in &a.operand_values { v.visit_expr(o)?; }
    Continue(())
}, {
    if let Some(spec) = &mut a.spec_expr { v.visit_expr(spec)?; }
    for o in &mut a.operand_values { v.visit_expr(o)?; }
    Continue(())
});

macro_rules! v {
    () => {
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
    };
}

macro_rules! f {
    () => {
        fn visit_file_ast(&mut self, ast: &mut FileAst) -> ControlFlow<()> {
            walk_file_ast_mut(self, ast)
        }
        fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
            walk_bind_mut(self, bind)
        }
        fn visit_bind_value(&mut self, val: &mut BindValue) -> ControlFlow<()> {
            walk_bind_value_mut(self, val)
        }
        fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
            walk_expr_mut(self, expr)
        }
        fn visit_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
            walk_fn_call_mut(self, call)
        }
        fn visit_binary(&mut self, bin: &mut Binary) -> ControlFlow<()> {
            walk_binary_mut(self, bin)
        }
        fn visit_when_expr(&mut self, when: &mut WhenExpr) -> ControlFlow<()> {
            walk_when_mut(self, when)
        }
        fn visit_when_arm(&mut self, arm: &mut WhenArm) -> ControlFlow<()> {
            walk_when_arm_mut(self, arm)
        }
        fn visit_if_expr(&mut self, ifx: &mut IfExpr) -> ControlFlow<()> {
            walk_if_mut(self, ifx)
        }
        fn visit_loop(&mut self, l: &mut Loop) -> ControlFlow<()> {
            walk_loop_mut(self, l)
        }
        fn visit_while_loop(&mut self, w: &mut WhileLoop) -> ControlFlow<()> {
            walk_while_mut(self, w)
        }
        fn visit_for_in_loop(&mut self, f: &mut ForInLoop) -> ControlFlow<()> {
            walk_for_in_mut(self, f)
        }
        fn visit_tag_call(&mut self, tc: &mut TagCall) -> ControlFlow<()> {
            walk_tag_call_mut(self, tc)
        }
        fn visit_format_string(&mut self, fs: &mut FormatString) -> ControlFlow<()> {
            walk_format_string_mut(self, fs)
        }
        fn visit_format_part(&mut self, p: &mut FormatPart) -> ControlFlow<()> {
            walk_format_part_mut(self, p)
        }
        fn visit_range(&mut self, r: &mut Range) -> ControlFlow<()> {
            walk_range_mut(self, r)
        }
        fn visit_return(&mut self, r: &mut Return) -> ControlFlow<()> {
            walk_return_mut(self, r)
        }
        fn visit_asm_expr(&mut self, a: &mut AsmExpr) -> ControlFlow<()> {
            walk_asm_mut(self, a)
        }
    };
}

pub trait Visitor: Sized {
    v!();
}

/// Mutable folder — same shape as [`Visitor`] but with `&mut` references.
pub trait Folder: Sized {
    f!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{Literal, Typed};
    use crate::span::SpanId;

    #[test]
    fn tuple_allocation_visits_initializer_and_size() {
        struct Literals(Vec<u128>);

        impl Visitor for Literals {
            fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
                if let Expr::Lit(Literal::Int(value)) = expr {
                    self.0.push(*value);
                }
                walk_expr(self, expr)
            }
        }

        let expr = Expr::TupleAlloc {
            init: Box::new(Typed::infer(Expr::Lit(Literal::Int(1)), SpanId::INVALID)),
            size: Box::new(Typed::infer(Expr::Lit(Literal::Int(2)), SpanId::INVALID)),
        };
        let mut literals = Literals(Vec::new());

        let _ = literals.visit_expr(&expr);

        assert_eq!(literals.0, vec![1, 2]);
    }
}
