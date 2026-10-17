pub mod analysis;
pub mod assembly;
mod domain;
mod emission;
mod tag_declarations;

pub mod array_promotion;
pub mod completions;
pub mod constant_object;
pub use constant_object::{
    ConstObject, ConstObjectError, ConstObjectId, ConstObjectStore, ConstPath, ConstProjection,
    ConstRelocation,
};

#[cfg(test)]
extern crate self as typecheck;

#[cfg(test)]
pub(crate) mod test_support;

pub mod resolved_program;
pub mod ty;
pub mod typed;
pub use completions::{
    CompletionCandidate, CompletionKind, dot_completions_for_ty, fn_call_at, signature_for_fn,
};
pub use resolved_program::ResolvedProgram;
pub use typed::{
    AlternativeEvidence, AppliedEffectTarget, Availability, BindBody, CallableResult, ConstFacts,
    DefId, EffectTarget, EvidenceProposition, ExprFacts, ExprId, FileId, FunctionEffects, GroupId,
    HoverResult, HoverTarget, MathematicalComparison, Overlap, OwnershipFacts,
    PackageSemanticIndex, PlaceId, PlaceProjection, PlaceVersionComponent, PlaceVersionId,
    PlaceVersionOrigin, ReferenceTargetGroup, ResolvedCompoundOperator, ResultEvidence,
    SemanticFacts, SequenceTraversal, TagId, TargetIndex, TypedBind, TypedCallableSignature,
    TypedCondition, TypedExpr, TypedExprKind, TypedFileAst, TypedGroup, TypedIfExpr, TypedLoop,
    TypedLoopKind, TypedPlace, TypedPlaceVersion, TypedTag, TypedWhenArm, TypedWhenExpr,
    ValueEvidence, VariantId, VariantLookupResult, VariantMap, VariantMapEntry,
    collect_package_variant_map, format_ty_for_hover,
};

pub mod staging;

pub mod const_value;
pub use const_value::{const_value_to_expr, const_value_to_typed_expr};

pub mod equality;
pub mod eval;
pub use eval::{
    Address, CompileTimeHost, EvalCondition, EvalFailure, EvalFrame, EvalHeap, EvalHost, EvalLocal,
    EvalOutcome, EvalResult, EvalState, Evaluator, Fuel,
};
mod anonymous_arithmetic;
pub mod integer_literal;
pub mod intrinsic;
pub mod intrinsic_fold;
pub mod layout;
pub mod operator;
pub mod representation;
pub mod type_registry;
pub use intrinsic_fold::inject_compiler_intrinsics;
pub use type_registry::{
    SequenceLiteralAcceptance, SequenceLiteralMaterialization, TypeDeclarationSemantics,
    TypeRegistry,
};

pub mod prepare;
pub use prepare::{prepare_package_asts, prepare_parse_ast};

pub mod prepare_target;
pub use prepare_target::{
    materialize_type_static_access, materialize_when_declare_subjects_from_package,
    validate_when_declare_exhaustiveness,
};

pub mod normal_expr;
pub use normal_expr::Normalize;

pub mod subst;
pub use subst::DepSubst;

pub mod solver;
pub use solver::{ConstraintEnv, Predicate, ProveResult, predicate_expr_to_predicate};

pub mod transform;

pub use analysis::expression_returns;

pub mod discriminant;
