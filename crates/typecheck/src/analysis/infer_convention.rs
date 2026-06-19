//! Body inference for the auto-thread model.
//!
//! Every bare (Inferred) parameter is automatically threaded through
//! the return type. Only `eat` (Consume) parameters are consumed.
//! The return type is expanded in `desugar_threads.rs`.

use std::collections::HashMap;

use ast::expr::Bind;
use ast::parameter::ParamConvention;
use internment::Intern;

/// The inferred convention for a single parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InferredConvention {
    /// Parameter is auto-threaded through the return.
    Threaded,
    /// Parameter is consumed (`eat`) — not returned.
    Consumed,
    /// Parameter is a reference (`ref` or `mut`) — borrowed, not consumed or threaded.
    Ref { mutable: bool },
}

/// Result of body inference for a function.
#[derive(Debug, Clone, Default)]
pub struct ConventionInference {
    pub conventions: HashMap<Intern<String>, InferredConvention>,
}

/// Run body inference for a single bind.
///
/// In the auto-thread model, this is trivial:
/// - Bare params → Threaded
/// - `eat` params → Consumed
pub fn infer_param_conventions(bind: &Bind) -> ConventionInference {
    let mut result = ConventionInference::default();

    let Some(params) = &bind.params else {
        return result;
    };

    for param_name in params.keys() {
        let explicit = bind
            .param_conventions
            .get(param_name)
            .copied()
            .unwrap_or(ParamConvention::Inferred);

        let convention = match explicit {
            ParamConvention::Eat => InferredConvention::Consumed,
            ParamConvention::Ref(mutable) => {
                // Ref params are neither consumed nor threaded.
                // They're borrowed — the caller retains ownership.
                InferredConvention::Ref { mutable }
            }
            ParamConvention::Inferred => InferredConvention::Threaded,
        };
        result.conventions.insert(*param_name, convention);
    }

    result
}
