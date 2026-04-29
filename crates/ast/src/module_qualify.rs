//! Prefix top-level definitions from a dependency module file so they can be
//! addressed as `module.symbol` (e.g. `io.print`) without clashing across packages.
//!
use crate::expr::AsmExpr;
use crate::expr::{BindValue, Expr, FormatPart, IfCondition, IfExpr, Loop, WhenArm};
use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::span::Spanned;
use crate::{Bind, DefMap, FileAst};
use internment::Intern;
use std::collections::HashSet;
use std::mem;

/// Prefix every top-level def in `ast` with `module_qual.` (e.g. `io.print`), rewrite
/// same-file references, and keep [`Bind::name`] in sync with the def map key.
pub fn qualify_module_defs(mut ast: FileAst, module_qual: &str) -> FileAst {
    let module_qual = module_qual.trim_matches('.');
    if module_qual.is_empty() {
        return ast;
    }

    let qual_parts: Vec<Intern<String>> = module_qual
        .split('.')
        .filter(|s| !s.is_empty())
        .map(Intern::<String>::from_ref)
        .collect();
    if qual_parts.is_empty() {
        return ast;
    }

    let old_names: HashSet<Intern<String>> = ast.defs.keys().copied().collect();

    let mut new_defs = DefMap::new();
    let old_defs = mem::take(&mut ast.defs);
    for (name, mut bind) in old_defs {
        let new_name = Intern::<String>::new(format!("{module_qual}.{}", name.as_str()));
        rewrite_bind_tree(&mut bind, &old_names, &qual_parts);
        let bind = bind.remap_module_symbol(new_name);
        new_defs.insert(new_name, bind);
    }
    ast.defs = new_defs;

    let mut new_private = HashSet::new();
    for name in ast.private_defs.drain() {
        new_private.insert(Intern::<String>::new(format!(
            "{module_qual}.{}",
            name.as_str()
        )));
    }
    ast.private_defs = new_private;

    ast
}

fn rewrite_bind_tree(
    bind: &mut Bind,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    match bind.value_mut() {
        BindValue::Expr(e) => rewrite_spanned_expr(e, old_names, qual_parts),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                rewrite_spanned_expr(e, old_names, qual_parts);
            }
            if let Some(e) = ret.0.as_mut() {
                rewrite_spanned_expr(e, old_names, qual_parts);
            }
        }
        BindValue::Extern => {}
    }
}

fn rewrite_spanned_expr(
    sp: &mut Spanned<Expr>,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    rewrite_expr(&mut sp.0, old_names, qual_parts);
}

fn rewrite_expr(
    expr: &mut Expr,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    match expr {
        Expr::FnCall(call) => {
            maybe_rewrite_fn_path(&mut call.path, old_names, qual_parts);
            if let Some(args) = call.args.as_mut() {
                for a in args {
                    rewrite_spanned_expr(a, old_names, qual_parts);
                }
            }
        }
        Expr::Binary(bin) => {
            rewrite_spanned_expr(&mut bin.lhs, old_names, qual_parts);
            rewrite_spanned_expr(&mut bin.rhs, old_names, qual_parts);
        }
        Expr::Bind(bind) => {
            rewrite_bind_tree(bind, old_names, qual_parts);
        }
        Expr::When(w) => {
            if let Some(s) = w.subject.as_mut() {
                rewrite_spanned_expr(s, old_names, qual_parts);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        rewrite_spanned_expr(condition, old_names, qual_parts);
                        rewrite_spanned_expr(body, old_names, qual_parts);
                    }
                    WhenArm::Is { pattern, body } => {
                        rewrite_spanned_expr(pattern, old_names, qual_parts);
                        rewrite_spanned_expr(body, old_names, qual_parts);
                    }
                    WhenArm::Else(body) => {
                        rewrite_spanned_expr(body, old_names, qual_parts);
                    }
                }
            }
        }
        Expr::If(ifex) => rewrite_if(ifex, old_names, qual_parts),
        Expr::Loop(loop_ex) => match loop_ex {
            Loop::While(w) => {
                rewrite_spanned_expr(&mut w.cond, old_names, qual_parts);
                for e in &mut w.exprs {
                    rewrite_spanned_expr(e, old_names, qual_parts);
                }
            }
            Loop::ForIn(f) => {
                rewrite_spanned_expr(&mut f.pat, old_names, qual_parts);
                rewrite_spanned_expr(&mut f.iter, old_names, qual_parts);
                for e in &mut f.exprs {
                    rewrite_spanned_expr(e, old_names, qual_parts);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &mut fs.parts {
                if let FormatPart::Expr(e) = part {
                    rewrite_spanned_expr(e.as_mut(), old_names, qual_parts);
                }
            }
        }
        Expr::Lit(_) | Expr::AnonymousTag(_, _) | Expr::SelfRef(_) => {}
        Expr::TagCall(tc) => {
            for a in &mut tc.args {
                rewrite_spanned_expr(a, old_names, qual_parts);
            }
        }
        Expr::TypeNominal(_, _) => {}
        Expr::TypeQualified(path) => {
            maybe_rewrite_fn_path(path, old_names, qual_parts);
        }
        Expr::TypeGeneric { params, .. } => {
            for (_, pk) in params.iter_mut() {
                match pk {
                    ParameterKind::Default(e) => rewrite_spanned_expr(e, old_names, qual_parts),
                    ParameterKind::Tagged(sp) => {
                        rewrite_spanned_expr(sp.as_mut(), old_names, qual_parts)
                    }
                    ParameterKind::Generic => {}
                }
            }
        }
        Expr::Range(r) => {
            rewrite_spanned_expr(&mut r.start, old_names, qual_parts);
            rewrite_spanned_expr(&mut r.end, old_names, qual_parts);
        }
        Expr::TupleAlloc { init, .. } => rewrite_spanned_expr(init, old_names, qual_parts),
        Expr::TupleGet { base, .. } => rewrite_spanned_expr(base, old_names, qual_parts),
        Expr::TupleSet { base, value, .. } => {
            rewrite_spanned_expr(base, old_names, qual_parts);
            rewrite_spanned_expr(value, old_names, qual_parts);
        }
        Expr::Cast { expr, .. } => rewrite_spanned_expr(expr, old_names, qual_parts),
        Expr::BufGet { buf, index } => {
            rewrite_spanned_expr(buf, old_names, qual_parts);
            rewrite_spanned_expr(index, old_names, qual_parts);
        }
        Expr::BufSet { buf, index, value } => {
            rewrite_spanned_expr(buf, old_names, qual_parts);
            rewrite_spanned_expr(index, old_names, qual_parts);
            rewrite_spanned_expr(value, old_names, qual_parts);
        }
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            rewrite_spanned_expr(e, old_names, qual_parts);
        }
        Expr::Asm(AsmExpr { operands, .. }) => {
            for o in operands {
                rewrite_spanned_expr(o, old_names, qual_parts);
            }
        }
        Expr::TupleLit(elems) => {
            for e in elems {
                rewrite_spanned_expr(e, old_names, qual_parts);
            }
        }
    }
}

fn rewrite_if(
    ifex: &mut IfExpr,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    match &mut ifex.condition {
        IfCondition::Bool(e) => rewrite_spanned_expr(e, old_names, qual_parts),
        IfCondition::Pattern { subject, pattern } => {
            rewrite_spanned_expr(subject, old_names, qual_parts);
            rewrite_spanned_expr(pattern, old_names, qual_parts);
        }
    }
    for e in &mut ifex.body {
        rewrite_spanned_expr(e, old_names, qual_parts);
    }
    if let Some(e) = ifex.ret.0.as_mut() {
        rewrite_spanned_expr(e, old_names, qual_parts);
    }
}

fn maybe_rewrite_fn_path(
    path: &mut ModPath,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    if !path.segments.is_empty() {
        return;
    }
    if !old_names.contains(&path.root) {
        return;
    }
    let span = path.span;
    let old_root = path.root;
    let mut segments: Vec<Intern<String>> = qual_parts[1..].to_vec();
    segments.push(old_root);
    path.root = qual_parts[0];
    path.segments = segments;
    path.span = span;
}
