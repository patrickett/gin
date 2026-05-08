#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
mod ty;
pub use ty::{Ty, ty_alignment, ty_byte_size_static, ty_union_discriminant_size};

mod env;
pub use env::{FnParamInfo, TyEnv};

mod infer;
pub use infer::{LocalTypes, TyInfer, TyInferEnv};

mod resolve;

pub mod flow;
pub mod flow_analyzer;

mod check;

mod salsa_update;

mod analysis;
pub use analysis::analyze_file_with_ty_env;

pub mod hover;
pub use hover::{
    dot_type_at, find_definition_span, find_import_definition_span, find_references, hover_at,
    is_variant_at,
};

pub mod completions;
pub use completions::{
    CompletionCandidate, CompletionKind, SignatureInfo, completions_for_ast, fn_call_at,
    format_params, signature_for_fn,
};

pub mod source;
pub use source::{
    byte_offset_to_position, get_char_at_position, is_identifier_char, is_in_comment,
    position_to_byte_offset, word_at_byte_offset, word_byte_range,
};
