#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
//! AST type definitions for the Gin compiler — parse-level types only.
//!
//! Typed AST types live in `typed_ast`, analysis/typecheck logic lives in `typecheck`.

pub mod expr;
pub mod warnings;
pub use expr::{
    AsmExpr, AttributeItem, BinOp, Binary, Bind, BindAttributes, BindValue, BundleExportImport,
    ClobberSpec, Complexity, ComplexityExpr, Expr, FnCall, ForInLoop, FormatPart, FormatString,
    IfCondition, IfExpr, Import, ImportSource, Literal, LocalBundleImport, LocalMemberImport, Loop,
    LoopEnum, ModuleImport, OperandKind, OperandSpec, Range, Return, TagCall, Typed, WhenArm,
    WhenExpr, WhileLoop,
};

mod file_ast;
pub use file_ast::{
    DefMap, FileAst, MergeConflict, MethodMap, Symbol, SymbolAlias, SymbolKind, SymbolTable, TagMap,
};

mod import_alias;
pub use import_alias::apply_symbol_aliases;

mod module_qualify;
pub use module_qualify::qualify_module_defs;

pub mod ty_state;
pub use ty_state::TyState;

pub mod parameter;
pub use parameter::{
    GroupParam, ParamConvention, ParamInfo, ParamSlot, ParameterKind, Parameters,
    format_type_surface,
};

pub mod type_decl;
pub use type_decl::is_capitalized_type_name;

pub mod const_value;
pub use const_value::{Bound, ConstValue, TypeConstraint};

pub mod folder;

pub mod type_expr;
pub use type_expr::{InRangeBounds, TypeExpr, fmt_in_range_bounds};

pub mod source;
pub use source::{
    byte_offset_to_line_col, byte_offset_to_position, compute_line_starts, get_char_at_position,
    is_identifier_char, is_in_comment, position_to_byte_offset, symbol_at_byte_offset,
    word_at_byte_offset, word_byte_range,
};

pub mod path;
pub use path::ModPath;

pub mod variant;
pub use variant::Variant;

pub mod doc_comment;
pub use doc_comment::DocComment;

pub mod declare;
pub use declare::{Declare, DeclareAttributes, DeclareValue, ProvidedTrait};

pub mod pattern;
pub use pattern::{
    expr_union_variant_at_byte, for_loop_pattern_names, for_loop_single_binding,
    is_pattern_wildcard, list_elem_ty, literal_value_from_expr, pattern_binding_types,
    pattern_param_type_at_slot, pattern_variant_literal_at_byte, type_expr_denotes_variant_name,
    type_surface_mangle_name, union_variant_reference_at_byte,
};

pub mod span;
pub use span::{HasSpanId, Span, SpanId, SpanTable, Spanned, SubSpan};

pub mod blanket_impl;
pub use blanket_impl::BlanketImpl;

pub mod ty;
pub use ty::{
    Ty, UnionVariant, VariantLookupResult, VariantMap, VariantMapEntry, resolve_type_expr_from_map,
};

pub mod trait_bound;
pub use trait_bound::TraitBound;

pub mod hover_format;
pub use hover_format::{HoverDoc, HoverSection, SECTION_SEP};

mod impl_block;
pub use impl_block::ImplBlock;

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
    pub use internment::Intern;
}
