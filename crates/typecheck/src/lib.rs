pub mod analysis;

pub mod completions;

pub mod ty;
pub mod typed;
pub use completions::{
    CompletionCandidate, CompletionKind, dot_completions_for_ty, fn_call_at, signature_for_fn,
};
pub use typed::{
    BindBody, DefId, ExprId, FileId, HoverResult, HoverTarget, PackageSemanticIndex,
    ResolvedImport, TagId, TypedBind, TypedExpr, TypedExprKind, TypedFileAst, TypedIfExpr,
    TypedLoop, TypedLoopKind, TypedTag, TypedWhenArm, TypedWhenExpr, VariantId,
    VariantLookupResult, VariantMap, VariantMapEntry, collect_package_variant_map,
    format_ty_for_hover,
};

pub mod comptime_classify;
pub use comptime_classify::{BindComptimeExt, ComptimeClass, FileAstComptimeExt};

pub mod compile_time_trait;
pub use compile_time_trait::{
    CompileTimeTraitRegistry, REFLECTABLE_SHAPE_FIELD, REFLECTABLE_TRAIT, RESERVED_TRAITS,
    synthesize_reflectable_trait, trait_field_for_ty, trait_field_for_type_name,
};

pub mod reflect;
pub use reflect::{
    const_value_for_provided_trait_field, const_value_to_expr, const_value_to_typed_expr,
    reflect_ty_to_const_value,
};

pub mod asm_intrinsics;
pub use asm_intrinsics::inject_asm_exprs;

pub mod intrinsic_fold;
pub use intrinsic_fold::inject_compiler_intrinsics;

pub mod prepare;
pub use prepare::{prepare_file_ast, prepare_package_asts};

pub mod prepare_target;
pub use prepare_target::{
    apply_entry_target_merge, const_binds_from_prepared_ast, infer_when_declare_subject_ty,
    materialize_default_binds, materialize_type_static_access,
    materialize_when_declare_subjects_from_package, validate_when_declare_exhaustiveness,
};

pub mod const_expr;
pub use const_expr::Normalize;

pub mod subst;
pub use subst::DepSubst;

pub mod solver;
pub use solver::{ConstraintEnv, Predicate, ProveResult, predicate_expr_to_predicate};

pub mod range_bounds;
pub use range_bounds::InclusiveBounds;

pub mod transform;
