pub mod analysis;

pub mod completions;

#[cfg(test)]
extern crate self as typecheck;

pub mod resolved_program;
pub mod ty;
pub mod typed;
pub use completions::{
    CompletionCandidate, CompletionKind, dot_completions_for_ty, fn_call_at, signature_for_fn,
};
pub use resolved_program::ResolvedProgram;
pub use typed::{
    AlternativeEvidence, AppliedEffectTarget, Availability, BindBody, DefId, EffectTarget,
    EvidenceProposition, ExprId, FileId, FunctionEffects, GroupId, HoverResult, HoverTarget,
    MathematicalComparison, Overlap, PackageSemanticIndex, PlaceId, PlaceProjection,
    PlaceVersionComponent, PlaceVersionId, PlaceVersionOrigin, ReferenceTargetGroup,
    ResolvedCompoundOperator, ResultEvidence, TagId, TargetIndex, TypedBind,
    TypedCallableSignature, TypedCondition, TypedExpr, TypedExprKind, TypedFileAst, TypedGroup,
    TypedIfExpr, TypedLoop, TypedLoopKind, TypedPlace, TypedPlaceVersion, TypedTag, TypedWhenArm,
    TypedWhenExpr, VariantId, VariantLookupResult, VariantMap, VariantMapEntry,
    collect_package_variant_map, format_ty_for_hover,
};

pub mod staging;

pub mod compile_time_trait;
pub use compile_time_trait::{
    CompileTimeTraitRegistry, REFLECTABLE_SHAPE_FIELD, REFLECTABLE_TRAIT, RESERVED_TRAITS,
    synthesize_reflectable_trait, trait_field_for_ty, trait_field_for_type_name,
};

pub mod reflect;
pub use reflect::{
    const_value_for_provided_trait_field, const_value_to_expr, const_value_to_typed_expr,
};

pub mod equality;
pub mod integer_literal;
pub mod intrinsic;
pub mod intrinsic_fold;
pub mod layout;
pub mod operator;
pub mod representation;
pub mod type_registry;
pub use intrinsic_fold::inject_compiler_intrinsics;
pub use type_registry::{FixedArrayFormer, TypeDeclarationSemantics, TypeRegistry};

pub mod prepare;
pub use prepare::{prepare_package_asts, prepare_parse_ast};

pub mod prepare_target;
pub use prepare_target::{
    apply_entry_target_merge, infer_when_declare_subject_ty, materialize_default_binds,
    materialize_type_static_access, materialize_when_declare_subjects_from_package,
    validate_when_declare_exhaustiveness,
};

pub mod normal_expr;
pub use normal_expr::Normalize;

pub mod subst;
pub use subst::DepSubst;

pub mod solver;
pub use solver::{ConstraintEnv, Predicate, ProveResult, predicate_expr_to_predicate};

pub mod range_bounds;
pub use range_bounds::InclusiveBounds;

pub mod transform;
