#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
//! AST type definitions for the Gin compiler.

pub mod expr;
pub use expr::FormatPart;
pub use expr::*;

mod file_ast;
pub use file_ast::*;

mod import_alias;
pub use import_alias::apply_symbol_aliases;

mod module_qualify;
pub use module_qualify::qualify_module_defs;

mod ty_state;
pub use ty_state::*;

mod parameter;
pub use parameter::*;

mod analysis;
pub use analysis::Analysis;
pub use analysis::check::pipeline::analyze_file;

/// Re-exported for external crates that reference `ast::flow::*`.
pub mod flow {
    pub use crate::analysis::{
        ConstValue, FlowAnalysis, FlowContext, ImpossibleCheck, IndexOutOfBounds, TypeConstraint,
    };
}
pub use analysis::{
    Bound, ConstValue, FlowAnalysis, FlowAnalyzer, FlowContext, ImpossibleCheck, IndexOutOfBounds,
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, TypeConstraint, VariantLookupResult,
    VariantMap, VariantMapEntry, is_type_surface, mangled_fn_call_name, resolve_name_from_files,
    resolve_parameter_kind_with_subst, resolve_type_expr_from_map, resolve_type_expr_with_subst,
    substitute_in_ty, typevars_from_receiver,
};

pub mod completions;
pub use completions::{
    CompletionCandidate, CompletionKind, SignatureInfo, completions_for_ast, fn_call_at,
    format_params, signature_for_fn,
};

pub mod hover;

pub mod ty;

pub mod folder;
pub mod visit;

pub mod type_expr;
pub use type_expr::*;

pub mod source;
pub use source::{
    byte_offset_to_position, get_char_at_position, is_identifier_char, is_in_comment,
    position_to_byte_offset, word_at_byte_offset, word_byte_range,
};

mod path;
pub use path::*;

mod variant;
pub use variant::*;

mod doc_comment;
pub use doc_comment::*;

mod declare;
pub use declare::*;

mod pattern;
pub use pattern::*;

pub mod span;
pub use span::*;

mod impl_block;
pub use impl_block::*;

pub mod signature;
pub use signature::*;

pub mod prelude {
    pub use crate::declare::*;
    pub use crate::doc_comment::*;
    pub use crate::expr::*;
    pub use crate::file_ast::*;
    pub use crate::folder::*;
    pub use crate::impl_block::*;
    pub use crate::parameter::*;
    pub use crate::path::*;
    pub use crate::pattern::*;
    pub use crate::span::*;
    pub use crate::type_expr::*;
    pub use crate::variant::*;
    pub use crate::visit::*;

    pub use internment::Intern;
}
