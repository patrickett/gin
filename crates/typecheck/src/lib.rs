pub mod analysis;

pub mod completions;

pub mod ty;
pub mod typed;
pub use completions::{
    CompletionCandidate, CompletionKind, dot_completions_for_ty, fn_call_at, signature_for_fn,
};
pub use typed::*;

pub mod comptime_classify;
pub use comptime_classify::*;

pub mod compile_time_trait;
pub use compile_time_trait::*;

pub mod reflect;
pub use reflect::*;

pub mod asm_intrinsics;
pub use asm_intrinsics::*;

pub mod intrinsic_fold;
pub use intrinsic_fold::*;

pub mod prepare;
pub use prepare::*;

pub mod prepare_target;
pub use prepare_target::*;

pub mod range_bounds;
pub use range_bounds::*;

pub mod transform;
