//! Desugaring pass for `^`-borrowed parameters.
//!
//! When a function has `^param` (borrow convention), the parameter value
//! must be threaded through the function's return — the function borrows
//! the value and gives it back on return.
//!
//! This pass modifies the typed AST to make this threading explicit:
//!
//! 1. **Function return type** — borrowed params are prepended to the return type.
//!    `fn foo(^buf Buffer(T), n Int) Int` becomes
//!    `fn foo(^buf Buffer(T), n Int) (Buffer(T), Int)`
//!
//! 2. **Return statements** — each `return expr` becomes
//!    `return (buf, expr)` where `buf` is the current value of each borrowed param.
//!
//! 3. **Call sites** — `foo(^my_buf, 42)` becomes
//!    `(my_buf, result) := foo(my_buf, 42)`.
//!
//! This pass runs after type resolution (Stage 2) but before flow analysis (Stage 3).

use internment::Intern;

use crate::ty::Ty;
use crate::typed::{DefId, TypedExprKind, TypedFileAst};

/// Run the desugaring pass on the typed AST.
///
/// Returns a list of diagnostic flaws for calls that don't properly
/// destructure the returned borrowed params.
pub fn stage_desugar_borrows(typed: &mut TypedFileAst) -> Vec<diagnostic::TypeSymptom> {
    let mut flaws = Vec::new();

    // Collect borrow info for all definitions.
    // Maps DefId -> list of (borrowed_param_name, param_type).
    let borrow_info: std::collections::HashMap<DefId, Vec<(Intern<String>, Ty)>> = typed
        .defs
        .iter()
        .filter_map(|(def_id, bind)| {
            if bind.borrow_params.is_empty() {
                return None;
            }
            let info: Vec<_> = bind
                .borrow_params
                .iter()
                .filter_map(|name| {
                    bind.params
                        .iter()
                        .find(|(pn, _)| pn == name)
                        .map(|(pn, pt)| (*pn, pt.clone()))
                })
                .collect();
            if info.is_empty() {
                None
            } else {
                Some((*def_id, info))
            }
        })
        .collect();

    if borrow_info.is_empty() {
        return flaws;
    }

    // For each definition with borrowed params, update the return type.
    for (def_id, info) in &borrow_info {
        if let Some(bind) = typed.defs.get_mut(def_id) {
            let mut ret_fields: Vec<Ty> = info.iter().map(|(_, ty)| ty.clone()).collect();
            match bind.return_type.clone() {
                Ty::Unit => {}
                Ty::Tuple(fields) => ret_fields.extend(fields),
                existing => ret_fields.push(existing),
            }
            bind.return_type = if ret_fields.len() == 1 {
                ret_fields.into_iter().next().unwrap()
            } else {
                Ty::Tuple(ret_fields)
            };
        }
    }

    // Walk all expressions to verify call sites.
    // For each call to a function with borrow params, check that the
    // corresponding arg at the call site uses TakeRef.
    for (target_def_id, info) in &borrow_info {
        let callee_param_order: Vec<Intern<String>> = typed
            .defs
            .get(target_def_id)
            .map(|bind| bind.params.iter().map(|(pn, _)| *pn).collect())
            .unwrap_or_default();

        // Scan all expressions for FnCalls to this target.
        let expr_count = typed.exprs.kind.len();
        for idx in 0..expr_count {
            let kind = &typed.exprs.kind[idx];
            if let TypedExprKind::FnCall { target, args } = kind {
                if target != target_def_id {
                    continue;
                }
                if let Some(args) = args {
                    for (borrow_name, _) in info {
                        // Find the position of this borrow param in the callee's param list.
                        if let Some(param_idx) =
                            callee_param_order.iter().position(|pn| pn == borrow_name)
                        {
                            if param_idx < args.len() {
                                let arg_id = args[param_idx];
                                let arg_kind = &typed.exprs.kind[arg_id.as_usize()];
                                if !matches!(arg_kind, TypedExprKind::TakeRef(_)) {
                                    flaws.push(diagnostic::TypeSymptom::BorrowNotReturned {
                                        name: borrow_name.as_str().to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    flaws
}
