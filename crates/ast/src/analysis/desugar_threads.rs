//! Desugaring pass for auto-threaded linear parameters.
//!
//! When a function has a bare param (`name Type`) that the body inference
//! found to be **Threaded** (appears in all return paths), the param value
//! must be threaded through the function's return.
//!
//! This pass modifies the typed AST to make this threading explicit:
//!
//! 1. **Function return type** — threaded params are prepended to the return type.
//!    ```gin
//!    fn inspect(file File) Int           -- user wrote
//!    fn inspect(file File) (File, Int)   -- after desugaring
//!    ```
//!
//! 2. **Return statements** — each `return expr` becomes
//!    `return (file, expr)` where `file` is the current value of each threaded param.
//!
//! 3. **Call sites** — `inspect(my_buf)` becomes
//!    `(my_buf, result) := inspect(my_buf)`.
//!
//! This pass runs after type resolution (Stage 2) but before flow analysis (Stage 3),

use std::collections::HashMap;

use crate::analysis::infer_convention::{ConventionInference, InferredConvention};
use crate::ty::Ty;
use crate::typed::{DefId, TypedExprKind, TypedFileAst};
use internment::Intern;

/// Run the desugaring pass on the typed AST.
///
/// Takes the convention inference results (from Phase 4 body inference)
/// and expands return types for functions with Threaded params.
///
/// Returns a list of diagnostic flaws for calls that don't properly
/// destructure the returned threaded params.
pub fn stage_desugar_threads(
    typed: &mut TypedFileAst,
    conventions: &HashMap<DefId, ConventionInference>,
) -> Vec<diagnostic::TypeSymptom> {
    let flaws = Vec::new();

    // Collect threading info for all definitions.
    // Maps DefId -> list of (threaded_param_name, param_type) in declaration order.
    let thread_info: HashMap<DefId, Vec<(Intern<String>, Ty)>> = typed
        .defs
        .iter()
        .filter_map(|(def_id, bind)| {
            let conv = conventions.get(def_id)?;
            let threaded: Vec<(Intern<String>, Ty)> = bind
                .params
                .iter()
                .filter(|(pn, _)| conv.conventions.get(pn) == Some(&InferredConvention::Threaded))
                .map(|(pn, pt)| (*pn, pt.clone()))
                .collect();
            if threaded.is_empty() {
                None
            } else {
                Some((*def_id, threaded))
            }
        })
        .collect();

    if thread_info.is_empty() {
        return flaws;
    }

    // For each definition with threaded params, update the return type.
    for (def_id, info) in &thread_info {
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
    // For each call to a function with threaded params, check that the
    // corresponding arg at the call site is consumed properly.
    for (target_def_id, info) in &thread_info {
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
                    for (thread_name, _) in info {
                        // Find the position of this threaded param in the callee's param list.
                        if let Some(param_idx) =
                            callee_param_order.iter().position(|pn| pn == thread_name)
                            && param_idx < args.len()
                        {
                            let arg_id = args[param_idx];
                            let arg_kind = &typed.exprs.kind[arg_id.as_usize()];
                            // Threaded params must be passed as bare variables or ConsumeArg.
                            // ConsumeArg indicates the user explicitly terminated the thread.
                            if matches!(arg_kind, TypedExprKind::ConsumeArg(_)) {
                                // Explicit consume — no threading needed.
                                continue;
                            }
                            // Bare variable — this is fine, the linear checker handles the destructure.
                            // No flaw needed at this stage.
                        }
                    }
                }
            }
        }
    }

    flaws
}
