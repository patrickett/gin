//! AST type definitions for the Gin compiler — parse-level types only.
//!
//! Typed AST types live alongside analysis/typecheck logic in `typecheck`.

pub mod expr;
pub mod warnings;
pub use expr::{
    AttributeItem, BinOp, Binary, Bind, BindAttributes, BindOperator, BindValue,
    BundleExportImport, Complexity, ComplexityExpr, Condition, Expr, FnCall, ForInLoop, FormatPart,
    FormatString, IfExpr, Import, ImportSource, Literal, LocalBundleImport, LocalMemberImport,
    Loop, LoopEnum, ModuleImport, OperatorRole, Range, Return, TagCall, Typed, WhenArm, WhenExpr,
    WhileLoop,
};

mod file_ast;
pub use file_ast::{
    DefMap, FileAst, MergeConflict, MethodMap, Symbol, SymbolAlias, SymbolKind, SymbolTable, TagMap,
};

mod import_alias;

mod module_qualify;

pub mod ty_state;
pub use ty_state::TyState;

pub mod group_path;
pub use group_path::GroupPath;

pub mod parameter;
pub use parameter::{
    GroupParam, ParamConvention, ParamInfo, ParamSlot, Parameter, ParameterKind, Parameters,
};

pub mod type_decl;
pub use type_decl::TypeNameExt;

pub mod normal_expr;
pub use normal_expr::{
    BinderId, BinderOwner, DependentArgId, NormalExpr, TargetQueryKind, TypeReference,
};

pub mod integer;

pub mod const_value;
pub use const_value::{ConstValue, TypeConstraint};

pub mod folder;

pub mod hash_float;
pub use hash_float::HashFloat;

pub mod type_expr;
pub use type_expr::{InRangeBounds, TypeExpr};

pub mod source;
pub use source::{CharExt, LineStartsExt, SourceExt};

pub mod path;
pub use path::ModPath;

pub mod variant;
pub use variant::Variant;

pub mod doc_comment;
pub use doc_comment::DocComment;

pub mod declare;
pub use declare::{
    Declare, DeclareAttributes, DeclareValue, DefaultTypeFormerRole, FixedArrayContract,
    HasFunction, HasFunctionKind, HasMember, HasMemberBody, HasProperty, ProvidedTrait,
};

pub mod pattern;
pub use pattern::{Pattern, PatternNameExt};

pub mod span;
pub use span::{HasSpanId, Span, SpanId, SpanTable, Spanned, SubSpan};

pub mod ty;
pub use ty::{
    ParamKind, PredicateExpr, ProofProposition, ProofRelation, ProofTerm, ResultAlternative,
    ResultFamilyOwner, Ty, TyArg, UnionVariant, VariantLookupResult, VariantMap, VariantMapEntry,
};

pub mod range_bounds;
pub use range_bounds::InclusiveBounds;

pub mod hover_format;
pub use hover_format::{HoverDoc, HoverSection, SECTION_SEP};

pub mod prelude {
    pub use crate::declare::*;
    pub use crate::doc_comment::*;
    pub use crate::expr::*;
    pub use crate::file_ast::*;
    pub use crate::folder::*;

    pub use crate::parameter::*;
    pub use crate::path::*;
    pub use crate::pattern::*;
    pub use crate::source::{CharExt, LineStartsExt, SourceExt};
    pub use crate::span::*;
    pub use crate::type_expr::*;
    pub use crate::variant::*;
    pub use internment::Intern;
}
