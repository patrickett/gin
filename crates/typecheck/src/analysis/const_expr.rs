//! Compile-time expression evaluation — fold expressions to [`ConstValue`]s.
//!
//! Handles literal folding, tag constructors, binary ops, bind references,
//! `AsmBuilder` method chains, and compile-time bind validation.

use std::collections::{HashMap, HashSet};

use crate::solver::{ConstraintEnv, ProveResult, predicate_expr_to_predicate};
use diagnostic::Diagnostic;
use internment::Intern;

use crate::analysis::pattern::{collect_pattern_bindings, pattern_matches};
use crate::prepare_target::eval_type_static_member;
use ast::declare::DeclareValue;
use ast::expr::{Expr, FnCall, FormatPart, Literal, Typed};
use ast::span::{HasSpanId, SpanId};
use ast::{Bind, BindValue, ConstExpr, FileAst, LoopEnum, Parameters, Return, WhenArm};
use ast::{ConstValue, HashFloat};

const MAX_COMPILE_TIME_DEPTH: usize = 512;

/// Public entry for compile-time expression evaluation.
pub fn eval_compile_time_expr_public(
    expr: &Expr,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
) -> Option<ConstValue> {
    eval_compile_time_expr(expr, const_binds, ast)
}

/// Evaluate with an explicit environment (type-variable substitutions, etc.).
pub fn eval_compile_time_expr_with_env(
    expr: &Expr,
    env: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
) -> Option<ConstValue> {
    eval_compile_time_expr_depth(expr, env, ast, 0, &mut HashSet::new())
}

/// Evaluate a compile-time bind call with its arguments.
pub fn eval_compile_time_bind_call(
    bind: &Bind,
    args: &[ConstValue],
    outer_env: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
    depth: usize,
    call_stack: &mut HashSet<String>,
) -> Option<ConstValue> {
    let call_key = format!("{}({args:?})", bind.name.as_str());
    if !call_stack.insert(call_key.clone()) {
        return None;
    }

    let mut env = outer_env.clone();
    if let Some(params) = &bind.params {
        let param_names: Vec<_> = params.keys().copied().collect();
        for (name, cv) in param_names.into_iter().zip(args.iter().cloned()) {
            env.insert(name, Some(cv));
        }
    }
    let result = match &bind.value {
        BindValue::Expr(expr) => {
            eval_compile_time_expr_depth(&expr.value, &env, ast, depth, call_stack)
        }
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                let _ = eval_compile_time_expr_depth(&e.value, &env, ast, depth, call_stack);
            }
            ret.value
                .as_ref()
                .and_then(|r| eval_compile_time_expr_depth(&r.value, &env, ast, depth, call_stack))
        }
        BindValue::Extern | BindValue::Unassigned => None,
    };

    call_stack.remove(&call_key);
    result
}

/// Fold `:=` binds to compile-time constants.
///
/// Evaluates the expression tree of each `:=` bind and stores the result
/// in the bind's `const_value`. This handles the `AsmBuilder` method chain
/// (`AsmBuilder::new → .input/.inout → .build`) as well as basic literal and
/// tag-constructor expressions.
pub fn fold_compile_time_binds(ast: &mut FileAst) {
    // Build a map of currently-known constant values (iterative folding)
    let mut const_binds: HashMap<Intern<String>, Option<ConstValue>> =
        ast.defs.keys().map(|name| (*name, None)).collect();

    // Iteratively fold until no new values are discovered
    let mut changed = true;
    while changed {
        changed = false;
        for bind in ast.defs.values() {
            if !bind.is_compile_time {
                continue;
            }
            if const_binds
                .get(&bind.name)
                .and_then(|o| o.as_ref())
                .is_some()
            {
                continue; // already folded
            }
            let expr = match &bind.value {
                BindValue::Expr(e) => e,
                _ => continue,
            };
            if let Some(cv) = eval_compile_time_expr(&expr.value, &const_binds, ast) {
                const_binds.insert(bind.name, Some(cv));
                changed = true;
            }
        }
    }

    // Write folded constants back to the AST
    for bind in ast.defs.values_mut() {
        if !bind.is_compile_time {
            continue;
        }
        if let Some(Some(cv)) = const_binds.get(&bind.name)
            && let BindValue::Expr(expr) = &mut bind.value
        {
            expr.const_value = Some(cv.clone());
        }
    }
}

/// Validate comptime-classified function bodies only reference compile-time-safe values.
pub fn validate_compile_time_binds(ast: &FileAst) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let span_table = &ast.span_table;

    // Collect all top-level bind names.
    // A name is compile-time-safe if:
    //   1. It was bound with `:=`, or
    //   2. It's a non-function top-level definition (no params) —
    //      these are global constants like `X0`, `SvcTemplate`, etc.
    let compile_time_names: HashMap<Intern<String>, bool> = ast
        .defs
        .iter()
        .map(|(name, bind)| {
            let is_const = bind.is_compile_time || (bind.params.is_none() && !bind.is_method());
            (*name, is_const)
        })
        .collect();

    // Walk comptime-classified top-level bind bodies only.
    for bind in ast.defs.values() {
        if !bind.is_compile_time {
            continue;
        }
        let mut runtime_names = param_names(bind.params.as_ref());

        match &bind.value {
            BindValue::Expr(expr) => {
                validate_typed_expr(
                    expr,
                    &mut runtime_names,
                    &compile_time_names,
                    span_table,
                    &mut diagnostics,
                );
            }
            BindValue::Body { exprs, ret } => {
                validate_body_exprs(
                    exprs,
                    ret,
                    &mut runtime_names,
                    &compile_time_names,
                    span_table,
                    &mut diagnostics,
                );
            }
            BindValue::Extern | BindValue::Unassigned => {}
        }
    }

    diagnostics
}

/// Warnings for `const_bind_after_declare` parsed at parse time.
pub fn check_const_bind_after_declare(ast: &FileAst) -> Vec<Diagnostic> {
    ast.parse_warnings.clone()
}

/// Check field refinements on tag constructions (e.g. `Index(3, 5)` with `value and < n`).
pub fn check_construction_refinements(ast: &FileAst) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let const_binds: HashMap<Intern<String>, Option<ConstValue>> = ast
        .defs
        .iter()
        .map(|(name, bind)| {
            let cv = match &bind.value {
                BindValue::Expr(expr) => expr.const_value.clone(),
                _ => None,
            };
            (*name, cv)
        })
        .collect();
    for bind in ast.defs.values() {
        if !bind.is_compile_time {
            continue;
        }
        match &bind.value {
            BindValue::Expr(expr) => {
                walk_tag_refinements(&expr.value, &const_binds, ast, &mut diagnostics);
            }
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    walk_tag_refinements(&expr.value, &const_binds, ast, &mut diagnostics);
                }
                if let Some(te) = ret.value.as_ref() {
                    walk_tag_refinements(&te.value, &const_binds, ast, &mut diagnostics);
                }
            }
            BindValue::Extern | BindValue::Unassigned => {}
        }
    }
    diagnostics
}

fn walk_tag_refinements(
    expr: &Expr,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::TagCall(call) => {
            let Some(decl) = ast.tags.get(&call.name) else {
                return;
            };
            let DeclareValue::Interface(members) = &decl.value else {
                return;
            };
            let span_table = &ast.span_table;

            for (i, member) in members.iter().enumerate() {
                let Some(refinement) = &member.refinement else {
                    continue;
                };
                let Some(arg) = call.args.get(i) else {
                    continue;
                };
                let Some(cv) = eval_compile_time_expr(&arg.value, const_binds, ast) else {
                    continue;
                };
                let field_value = ConstExpr::Value(cv);
                let predicate =
                    predicate_expr_to_predicate(refinement, field_value, &HashMap::new());
                match ConstraintEnv::default().prove(&predicate, &HashMap::new()) {
                    ProveResult::Proven => {}
                    ProveResult::Disproven => {
                        diagnostics.push(
                            Diagnostic::new(
                                "dep-refinement-failed",
                                format!("refinement `{:?}` is false", refinement),
                            )
                            .at_span_id(arg.span_id(), span_table),
                        );
                    }
                    ProveResult::Unknown => {
                        diagnostics.push(
                            Diagnostic::new(
                                "dep-refinement-unproven",
                                format!("cannot prove `{:?}` in this context", refinement),
                            )
                            .at_span_id(arg.span_id(), span_table),
                        );
                    }
                }
            }
        }
        Expr::Binary(bin) => {
            walk_tag_refinements(&bin.lhs.value, const_binds, ast, diagnostics);
            walk_tag_refinements(&bin.rhs.value, const_binds, ast, diagnostics);
        }
        Expr::Bind(b) => {
            let span_table = &ast.span_table;
            if let Some(Some(cv)) = const_binds.get(&b.name)
                && let ConstValue::Tag { name, args, .. } = cv
                && let Some(decl) = ast.tags.get(name)
                && let DeclareValue::Interface(members) = &decl.value
            {
                for (i, member) in members.iter().enumerate() {
                    let Some(refinement) = &member.refinement else {
                        continue;
                    };
                    let Some(arg_cv) = args.get(i) else { continue };
                    let field_value = ConstExpr::Value(arg_cv.clone());
                    let predicate =
                        predicate_expr_to_predicate(refinement, field_value, &HashMap::new());
                    match ConstraintEnv::default().prove(&predicate, &HashMap::new()) {
                        ProveResult::Proven => {}
                        ProveResult::Disproven => {
                            diagnostics.push(
                                Diagnostic::new(
                                    "dep-refinement-failed",
                                    format!("refinement `{:?}` is false", refinement),
                                )
                                .at_span_id(b.name_span, span_table),
                            );
                        }
                        ProveResult::Unknown => {
                            diagnostics.push(
                                Diagnostic::new(
                                    "dep-refinement-unproven",
                                    format!("cannot prove `{:?}` in this context", refinement),
                                )
                                .at_span_id(b.name_span, span_table),
                            );
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Try to evaluate an expression tree to a compile-time constant.
///
/// Handles literals, pure tag constructors, binary ops, bind references,
/// and the `AsmBuilder` method chain (
/// `AsmBuilder::new → .input/.inout/.output → .clobber/.clobber_memory → .build`).
fn eval_compile_time_expr(
    expr: &Expr,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
) -> Option<ConstValue> {
    eval_compile_time_expr_depth(expr, const_binds, ast, 0, &mut HashSet::new())
}

fn eval_compile_time_expr_depth(
    expr: &Expr,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
    depth: usize,
    call_stack: &mut HashSet<String>,
) -> Option<ConstValue> {
    if depth > MAX_COMPILE_TIME_DEPTH {
        return None;
    }
    match expr {
        Expr::Lit(lit) => match lit {
            Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
            Literal::String(s) => Some(ConstValue::String(s.clone())),
            Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
        },
        Expr::AnonymousTag(name) => Some(ConstValue::Tag {
            name: *name,
            qual_path: None,
            args: Vec::new().into(),
        }),
        Expr::RecordLit(fields) => {
            // Record literal: `(arch: 'x86_64', vendor: 'unknown')` → ConstValue::Record
            let pairs: Vec<(Intern<String>, ConstValue)> = fields
                .iter()
                .filter_map(|(name, expr)| {
                    let cv = eval_compile_time_expr_depth(
                        &expr.value,
                        const_binds,
                        ast,
                        depth,
                        call_stack,
                    )?;
                    Some((*name, cv))
                })
                .collect();
            if pairs.len() == fields.len() {
                Some(ConstValue::Record {
                    fields: pairs.into(),
                })
            } else {
                None
            }
        }
        Expr::TagCall(call) => {
            let args: Vec<ConstValue> = call
                .args
                .iter()
                .filter_map(|a| {
                    eval_compile_time_expr_depth(&a.value, const_binds, ast, depth, call_stack)
                })
                .collect();
            if args.len() != call.args.len() {
                return None;
            }
            // `qual_path` is the prefix of the path, NOT including the last
            // segment which is already captured in `name`. Skipping the last
            // segment prevents double-appending in `to_hover_string`.
            let qual_path = call.qual_path.as_ref().map(|p| {
                let use_segments =
                    if p.segments.last().map(|s| s.as_str()) == Some(call.name.as_str()) {
                        &p.segments[..p.segments.len().saturating_sub(1)]
                    } else {
                        &p.segments[..]
                    };
                let mut s = p.root.as_str().to_string();
                for seg in use_segments {
                    s.push('.');
                    s.push_str(seg.as_str());
                }
                s
            });
            if let Some(decl) = ast.tags.get(&call.name)
                && let DeclareValue::Interface(members) = &decl.value
                && !members.is_empty()
            {
                // Interface tags can't be constructed with positional args.
                Some(ConstValue::Tag {
                    name: call.name,
                    qual_path,
                    args: args.into(),
                })
            } else {
                Some(ConstValue::Tag {
                    name: call.name,
                    qual_path,
                    args: args.into(),
                })
            }
        }
        Expr::RecordSet { .. } | Expr::Destructure { .. } => None,
        Expr::RecordGet { base, field } => {
            let base_cv = if let Expr::FnCall(call) = &base.value {
                if call.args.is_none() && call.path.value.segments.is_empty() {
                    const_binds
                        .get(&call.path.value.root)
                        .and_then(|cv| cv.clone())?
                } else if call.args.is_none() && !call.path.value.segments.is_empty() {
                    let mut parts = vec![call.path.value.root.as_str()];
                    parts.extend(call.path.value.segments.iter().map(|s| s.as_str()));
                    let fq_name = Intern::new(parts.join("."));
                    const_binds
                        .get(&fq_name)
                        .and_then(|cv| cv.clone())
                        .or_else(|| {
                            eval_compile_time_expr_depth(
                                &base.value,
                                const_binds,
                                ast,
                                depth,
                                call_stack,
                            )
                        })?
                } else {
                    eval_compile_time_expr_depth(&base.value, const_binds, ast, depth, call_stack)?
                }
            } else {
                eval_compile_time_expr_depth(&base.value, const_binds, ast, depth, call_stack)?
            };
            match base_cv {
                ConstValue::Record { fields } => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, v)| v.clone()),
                ConstValue::Tag { name, args, .. } => {
                    let idx = ast
                        .tags
                        .get(&name)
                        .and_then(|decl| decl.params.as_ref())
                        .and_then(|params| params.keys().position(|param_name| param_name == field))
                        .or_else(|| match name.as_str() {
                            "Target" => ["arch", "vendor", "os"]
                                .iter()
                                .position(|field_name| *field_name == field.as_str()),
                            _ => None,
                        })?;
                    args.get(idx).cloned()
                }
                _ => None,
            }
        }
        Expr::Bind(b) if b.params.is_none() => {
            if let Some(Some(cv)) = const_binds.get(&b.name) {
                return Some(cv.clone());
            }
            if let BindValue::Expr(expr) = &b.value
                && let Some(cv) =
                    eval_compile_time_expr_depth(&expr.value, const_binds, ast, depth, call_stack)
            {
                return Some(cv);
            }
            None
        }

        Expr::TupleLit(elems) | Expr::List(elems) => {
            let items: Vec<ConstValue> = elems
                .iter()
                .filter_map(|e| {
                    eval_compile_time_expr_depth(&e.value, const_binds, ast, depth, call_stack)
                })
                .collect();
            if items.len() == elems.len() {
                Some(ConstValue::List(items.into()))
            } else {
                None
            }
        }
        Expr::Binary(bin) => {
            let lhs =
                eval_compile_time_expr_depth(&bin.lhs.value, const_binds, ast, depth, call_stack)?;
            let rhs =
                eval_compile_time_expr_depth(&bin.rhs.value, const_binds, ast, depth, call_stack)?;
            lhs.eval_binop(&bin.op, &rhs)
        }
        Expr::When(when) => {
            let subject = when.subject.as_ref().and_then(|s| {
                eval_compile_time_expr_depth(&s.value, const_binds, ast, depth, call_stack)
            })?;
            eval_matching_when_arm(&when.arms, &subject, const_binds, ast, depth, call_stack)
        }
        Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
            const_binds
                .get(&call.path.value.root)
                .and_then(|cv| cv.clone())
        }
        Expr::FnCall(call) if call.args.is_none() && !call.path.value.segments.is_empty() => {
            let mut parts = vec![call.path.value.root.as_str()];
            parts.extend(call.path.value.segments.iter().map(|s| s.as_str()));
            let fq_name = Intern::new(parts.join("."));
            if let Some(cv) = const_binds.get(&fq_name).and_then(|cv| cv.clone()) {
                return Some(cv);
            }
            if call.path.value.segments.len() != 1 {
                return None;
            }
            let base = call.path.value.root;
            let field = call.path.value.segments[0];
            if let Some(cv) = eval_type_static_member(base, field, ast, const_binds) {
                return Some(cv);
            }
            if let Some(Some(cv)) = const_binds.get(&base)
                && let ConstValue::Record { fields } = cv
            {
                return fields
                    .iter()
                    .find(|(n, _)| n == &field)
                    .map(|(_, v)| v.clone());
            }
            None
        }
        Expr::FnCall(call) if call.path.value.segments.is_empty() => {
            let name = call.path.value.root;
            let args: Vec<ConstValue> = match &call.args {
                Some(a) => a
                    .iter()
                    .filter_map(|a| {
                        eval_compile_time_expr_depth(&a.value, const_binds, ast, depth, call_stack)
                    })
                    .collect(),
                None => Vec::new(),
            };
            if call.args.is_some() && args.len() != call.args.as_ref().unwrap().len() {
                return None;
            }
            if let Some(Some(cv)) = const_binds.get(&name) {
                return Some(cv.clone());
            }
            if let Some(cv) = eval_compile_time_helper(name.as_str(), &args) {
                return Some(cv);
            }
            let bind = ast.defs.get(&name)?;
            if !bind.is_compile_time {
                return None;
            }
            eval_compile_time_bind_call(bind, &args, const_binds, ast, depth + 1, call_stack)
        }
        Expr::FnCall(call) => {
            eval_compile_time_fn_call_asm(call, const_binds, ast, depth, call_stack)
        }
        _ => None,
    }
}

/// Evaluate named helpers (`add`, `max_size`, `gt`, `lt`, `ge`, `le`) at compile time.
fn eval_compile_time_helper(name: &str, args: &[ConstValue]) -> Option<ConstValue> {
    match (name, args) {
        // Size helpers
        ("add", [a, b]) => match (a.as_const_size_int(), b.as_const_size_int()) {
            (Some(x), Some(y)) => Some(ConstValue::const_size_tag(x + y)),
            _ => Some(ConstValue::dynamic_size_tag()),
        },
        ("max_size", [a, b]) => match (a.as_const_size_int(), b.as_const_size_int()) {
            (Some(x), Some(y)) => Some(ConstValue::const_size_tag(if x >= y { x } else { y })),
            _ => Some(ConstValue::dynamic_size_tag()),
        },
        // Comparison helpers — operate on plain Int values and return Bool tags
        ("gt", [a, b]) => compare_int_values(a, b, |x, y| x > y),
        ("lt", [a, b]) => compare_int_values(a, b, |x, y| x < y),
        ("ge", [a, b]) => compare_int_values(a, b, |x, y| x >= y),
        ("le", [a, b]) => compare_int_values(a, b, |x, y| x <= y),
        _ => None,
    }
}

/// Compare two `ConstValue::Int` values with the given predicate.
fn compare_int_values(
    a: &ConstValue,
    b: &ConstValue,
    cmp: fn(i128, i128) -> bool,
) -> Option<ConstValue> {
    match (a, b) {
        (ConstValue::Int(x), ConstValue::Int(y)) => Some(ConstValue::Tag {
            name: Intern::from_ref(if cmp(*x, *y) { "True" } else { "False" }),
            qual_path: None,
            args: vec![].into(),
        }),
        _ => None,
    }
}

/// Evaluate an `FnCall` as part of the `AsmBuilder` method chain.
fn eval_compile_time_fn_call_asm(
    call: &FnCall,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
    depth: usize,
    call_stack: &mut HashSet<String>,
) -> Option<ConstValue> {
    let args: Vec<ConstValue> = match &call.args {
        Some(a) => a
            .iter()
            .filter_map(|a| {
                eval_compile_time_expr_depth(&a.value, const_binds, ast, depth, call_stack)
            })
            .collect(),
        None => Vec::new(),
    };
    if call.args.is_some() && args.len() != call.args.as_ref().unwrap().len() {
        return None;
    }

    let path = &call.path.value;
    let (method_name, is_qualified) = if !path.segments.is_empty() {
        (path.segments[0], path.root.as_str() == "AsmBuilder")
    } else if !path.root.as_str().is_empty() {
        (path.root, false)
    } else {
        return None;
    };

    if !is_qualified
        && method_name.as_str() != "new"
        && !matches!(
            method_name.as_str(),
            "input" | "output" | "inout" | "lateout" | "clobber" | "clobber_memory" | "build"
        )
    {
        return None;
    }

    crate::asm_intrinsics::try_fold_asm_builder(method_name.as_str(), &args)
}

/// Evaluate a when expression against a known subject value by finding the
/// matching arm and evaluating its body.
fn eval_matching_when_arm(
    arms: &[WhenArm],
    subject: &ConstValue,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
    ast: &FileAst,
    depth: usize,
    call_stack: &mut HashSet<String>,
) -> Option<ConstValue> {
    for arm in arms {
        match arm {
            WhenArm::Is { pattern, body, .. } => {
                if pattern_matches(&pattern.value, subject) {
                    let mut arm_env = const_binds.clone();
                    let mut flat = HashMap::new();
                    collect_pattern_bindings(&pattern.value, subject, &mut flat);
                    for (k, v) in flat {
                        arm_env.insert(k, Some(v));
                    }
                    return eval_compile_time_expr_depth(
                        &body.value,
                        &arm_env,
                        ast,
                        depth,
                        call_stack,
                    );
                }
            }
            WhenArm::Else(body, _) => {
                return eval_compile_time_expr_depth(
                    &body.value,
                    const_binds,
                    ast,
                    depth,
                    call_stack,
                );
            }
            WhenArm::Cond {
                condition, body, ..
            } => {
                let cond_cv = eval_compile_time_expr_depth(
                    &condition.value,
                    const_binds,
                    ast,
                    depth,
                    call_stack,
                )?;
                match &cond_cv {
                    // Condition is True — evaluate and return the body
                    ConstValue::Tag { name, .. } if name.as_str() == "True" => {
                        return eval_compile_time_expr_depth(
                            &body.value,
                            const_binds,
                            ast,
                            depth,
                            call_stack,
                        );
                    }
                    // Condition is False — continue to next arm
                    ConstValue::Tag { name, .. } if name.as_str() == "False" => {}
                    // Unknown condition value — cannot determine at compile time
                    _ => return None,
                }
            }
        }
    }
    None
}

/// Collect parameter names from a bind's optional parameter list.
fn param_names(params: Option<&Parameters>) -> HashSet<Intern<String>> {
    match params {
        Some(params) => params.keys().copied().collect(),
        None => HashSet::new(),
    }
}

/// Check that an expression tree references only compile-time-safe names.
fn is_expr_compile_time_safe(
    expr: &Expr,
    runtime_names: &HashSet<Intern<String>>,
    compile_time_names: &HashMap<Intern<String>, bool>,
) -> bool {
    match expr {
        Expr::Lit(_) => true,
        Expr::TagCall(call) => call
            .args
            .iter()
            .all(|a| is_expr_compile_time_safe(&a.value, runtime_names, compile_time_names)),
        Expr::Bind(b) => {
            if runtime_names.contains(&b.name) {
                return false;
            }
            compile_time_names.get(&b.name).copied().unwrap_or(false)
        }
        Expr::FnCall(call) => {
            let args_safe = call
                .args
                .as_ref()
                .map(|args| {
                    args.iter().all(|a| {
                        is_expr_compile_time_safe(&a.value, runtime_names, compile_time_names)
                    })
                })
                .unwrap_or(true);
            if !args_safe {
                return false;
            }
            if call.path.value.segments.is_empty() {
                compile_time_names.contains_key(&call.path.value.root)
            } else {
                true
            }
        }
        Expr::Binary(bin) => {
            is_expr_compile_time_safe(&bin.lhs.value, runtime_names, compile_time_names)
                && is_expr_compile_time_safe(&bin.rhs.value, runtime_names, compile_time_names)
        }
        Expr::RecordLit(fields) => fields
            .iter()
            .all(|(_, e)| is_expr_compile_time_safe(&e.value, runtime_names, compile_time_names)),
        Expr::TupleLit(elems) => elems
            .iter()
            .all(|e| is_expr_compile_time_safe(&e.value, runtime_names, compile_time_names)),
        Expr::List(elems) => elems
            .iter()
            .all(|e| is_expr_compile_time_safe(&e.value, runtime_names, compile_time_names)),
        Expr::Destructure { value, .. } => {
            is_expr_compile_time_safe(&value.value, runtime_names, compile_time_names)
        }
        Expr::RecordSet { base, value, .. } => {
            is_expr_compile_time_safe(&base.value, runtime_names, compile_time_names)
                && is_expr_compile_time_safe(&value.value, runtime_names, compile_time_names)
        }
        Expr::RecordGet { base, .. } => {
            is_expr_compile_time_safe(&base.value, runtime_names, compile_time_names)
        }
        _ => false,
    }
}

/// Walk a list of body expressions, tracking runtime bindings and validating
/// any `:=` binds encountered.
fn validate_body_exprs(
    exprs: &[Typed<Expr>],
    ret: &Return,
    runtime_names: &mut HashSet<Intern<String>>,
    compile_time_names: &HashMap<Intern<String>, bool>,
    span_table: &ast::span::SpanTable,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for expr in exprs {
        validate_typed_expr(
            expr,
            runtime_names,
            compile_time_names,
            span_table,
            diagnostics,
        );
    }
    if let Some(ret_expr) = &ret.value {
        validate_typed_expr(
            ret_expr.as_ref(),
            runtime_names,
            compile_time_names,
            span_table,
            diagnostics,
        );
    }
}

/// Validate a single typed expression, descending into binds and nested
/// control flow.
fn validate_typed_expr(
    expr: &Typed<Expr>,
    runtime_names: &mut HashSet<Intern<String>>,
    compile_time_names: &HashMap<Intern<String>, bool>,
    span_table: &ast::span::SpanTable,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &expr.value {
        Expr::Bind(b) => {
            if b.params.is_none() {
                match &b.value {
                    BindValue::Expr(inner) => {
                        if b.is_compile_time {
                            if !is_expr_compile_time_safe(
                                &inner.value,
                                runtime_names,
                                compile_time_names,
                            ) {
                                let var_name = find_first_runtime_ref(&inner.value, runtime_names);
                                if let Some(var) = var_name {
                                    diagnostics.push(
                                        Diagnostic::new(
                                            "compile-time-runtime-var",
                                            format!(
                                                "runtime variable `{}` used in compile-time bind `{}`",
                                                var,
                                                b.name.as_str()
                                            ),
                                        )
                                        .with_arg("bind_name", b.name.as_str().to_string())
                                        .with_arg("var_name", var)
                                        .at_span(span_table.get(expr.span_id)),
                                    );
                                } else {
                                    diagnostics.push(
                                        Diagnostic::new(
                                            "compile-time-runtime-call",
                                            format!(
                                                "runtime call in compile-time bind `{}`",
                                                b.name.as_str()
                                            ),
                                        )
                                        .with_arg("bind_name", b.name.as_str().to_string())
                                        .at_span(span_table.get(expr.span_id)),
                                    );
                                }
                            }
                        } else {
                            runtime_names.insert(b.name);
                        }
                    }
                    BindValue::Body { exprs, ret } => {
                        let mut inner_runtime = runtime_names.clone();
                        if b.is_compile_time {
                            validate_body_exprs(
                                exprs,
                                ret,
                                &mut inner_runtime,
                                compile_time_names,
                                span_table,
                                diagnostics,
                            );
                        } else {
                            validate_body_exprs(
                                exprs,
                                ret,
                                &mut inner_runtime,
                                compile_time_names,
                                span_table,
                                diagnostics,
                            );
                            runtime_names.insert(b.name);
                        }
                    }
                    BindValue::Extern | BindValue::Unassigned => {}
                }
            }
        }
        Expr::If(ifx) => {
            validate_typed_expr(
                &ifx.subject,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            let mut body_runtime = runtime_names.clone();
            validate_body_exprs(
                &ifx.body,
                &ifx.ret,
                &mut body_runtime,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::Loop(loop_enum) => match loop_enum {
            LoopEnum::While(w) => {
                validate_typed_expr(
                    &w.cond,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
                let mut body_runtime = runtime_names.clone();
                let no_ret = Return {
                    value: None,
                    span_id: SpanId::INVALID,
                };
                validate_body_exprs(
                    &w.exprs,
                    &no_ret,
                    &mut body_runtime,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
            LoopEnum::ForIn(f) => {
                validate_typed_expr(
                    &f.iter,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
                let mut body_runtime = runtime_names.clone();
                for name in pattern_names_in_expr(&f.pat.value) {
                    body_runtime.insert(name);
                }
                let no_ret = Return {
                    value: None,
                    span_id: SpanId::INVALID,
                };
                validate_body_exprs(
                    &f.exprs,
                    &no_ret,
                    &mut body_runtime,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
        },
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    validate_typed_expr(
                        arg,
                        runtime_names,
                        compile_time_names,
                        span_table,
                        diagnostics,
                    );
                }
            }
        }
        Expr::Binary(bin) => {
            validate_typed_expr(
                &bin.lhs,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                &bin.rhs,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                validate_typed_expr(
                    arg,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for elem in elems {
                validate_typed_expr(
                    elem,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
        }
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let FormatPart::Expr(e, _) = part {
                    validate_typed_expr(
                        e,
                        runtime_names,
                        compile_time_names,
                        span_table,
                        diagnostics,
                    );
                }
            }
        }
        Expr::Range(r) => {
            validate_typed_expr(
                &r.start,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                &r.end,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::Asm(a) => {
            if let Some(spec) = &a.spec_expr {
                validate_typed_expr(
                    spec,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
            for op in &a.operand_values {
                validate_typed_expr(
                    op,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
        }
        Expr::Negate(e)
        | Expr::Cast { expr: e, .. }
        | Expr::TakePtr(e)
        | Expr::Ref { inner: e, .. }
        | Expr::ConsumeArg(e)
        | Expr::Eat(e)
        | Expr::Deref(e) => {
            validate_typed_expr(
                e,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::TupleAlloc { init, size } => {
            validate_typed_expr(
                init,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                size,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::TupleGet { base, .. } | Expr::RecordGet { base, .. } => {
            validate_typed_expr(
                base,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::Destructure { value, .. } => {
            validate_typed_expr(
                value,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::RecordSet { base, value, .. } | Expr::TupleSet { base, value, .. } => {
            validate_typed_expr(
                base,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                value,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::BufGet { buf, index } => {
            validate_typed_expr(
                buf,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                index,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            validate_typed_expr(
                buf,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                index,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
            validate_typed_expr(
                value,
                runtime_names,
                compile_time_names,
                span_table,
                diagnostics,
            );
        }
        Expr::When(w) => {
            if let Some(subject) = &w.subject {
                validate_typed_expr(
                    subject,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        validate_typed_expr(
                            condition,
                            runtime_names,
                            compile_time_names,
                            span_table,
                            diagnostics,
                        );
                        validate_typed_expr(
                            body,
                            runtime_names,
                            compile_time_names,
                            span_table,
                            diagnostics,
                        );
                    }
                    WhenArm::Is { body, .. } => {
                        validate_typed_expr(
                            body,
                            runtime_names,
                            compile_time_names,
                            span_table,
                            diagnostics,
                        );
                    }
                    WhenArm::Else(body, _) => {
                        validate_typed_expr(
                            body,
                            runtime_names,
                            compile_time_names,
                            span_table,
                            diagnostics,
                        );
                    }
                }
            }
        }
        Expr::RecordLit(fields) => {
            for (_, e) in fields {
                validate_typed_expr(
                    e,
                    runtime_names,
                    compile_time_names,
                    span_table,
                    diagnostics,
                );
            }
        }
        Expr::Lit(_)
        | Expr::SelfRef
        | Expr::AnonymousTag(..)
        | Expr::TypeNominal(..)
        | Expr::TypeInRange(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. }
        | Expr::TypeRef { .. } => {}
    }
}

/// Extract variable names from an expression used as a for-loop pattern.
fn pattern_names_in_expr(pat: &Expr) -> Vec<Intern<String>> {
    match pat {
        Expr::Bind(b) if b.params.is_none() => vec![b.name],
        Expr::TupleLit(elems) => elems
            .iter()
            .flat_map(|e| pattern_names_in_expr(&e.value))
            .collect(),
        _ => Vec::new(),
    }
}

/// Find the first runtime name referenced in an expression (for error messages).
fn find_first_runtime_ref(expr: &Expr, runtime_names: &HashSet<Intern<String>>) -> Option<String> {
    match expr {
        Expr::Bind(b) => {
            if runtime_names.contains(&b.name) {
                Some(b.name.as_str().to_string())
            } else {
                None
            }
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    if let Some(name) = find_first_runtime_ref(&arg.value, runtime_names) {
                        return Some(name);
                    }
                }
            }
            None
        }
        Expr::Binary(bin) => find_first_runtime_ref(&bin.lhs.value, runtime_names)
            .or_else(|| find_first_runtime_ref(&bin.rhs.value, runtime_names)),
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                if let Some(name) = find_first_runtime_ref(&arg.value, runtime_names) {
                    return Some(name);
                }
            }
            None
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for elem in elems {
                if let Some(name) = find_first_runtime_ref(&elem.value, runtime_names) {
                    return Some(name);
                }
            }
            None
        }
        Expr::Destructure { value, .. } => find_first_runtime_ref(&value.value, runtime_names),
        Expr::RecordSet { base, value, .. } => find_first_runtime_ref(&base.value, runtime_names)
            .or_else(|| find_first_runtime_ref(&value.value, runtime_names)),
        Expr::RecordGet { base, .. } => find_first_runtime_ref(&base.value, runtime_names),
        _ => None,
    }
}
