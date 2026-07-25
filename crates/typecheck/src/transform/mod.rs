//! 3-stage transformation pipeline: `FileAst → TypedFileAst`.
//!
//! Stages:
//! 1. **Declare** — Resolve tag/bind declarations, build file-level index, detect declaration flaws.
//! 2. **Lower** — Lower parse expressions to typed expressions, attach type flaws.
//! 3. **Flow** — Compute flow contexts, attach flow flaws (ownership, bounds).

use crate::analysis::validate_consumption::stage_validate_consumption;

use ast::FileAst;
use ast::prelude::*;

use crate::typed::{DefId, FileId, TagId, TypedCallableSignature, TypedFileAst, VariantMapEntry};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod bounds_check;
mod declare;
mod effects;
mod flow;
mod inline_comptime_calls;
mod lower_exprs;
mod lower_kind;
mod lower_tag;
mod lower_ty;
mod lower_util;
mod lower_when;

pub(crate) use declare::collect_bind_value_self_ref_spans;
pub use declare::stage_declare;
pub use flow::{RefTracker, stage_flow};
pub use lower_exprs::stage_lower;

/// Cross-file type environment for the transformation pipeline.
///
/// Carries resolved type information from other files so that expressions in
/// this file can resolve cross-file references.
pub struct TransformCtx {
    /// Tag types from other files: tag name → resolved Ty.
    pub cross_file_tag_types: HashMap<TagId, crate::ty::Ty>,
    pub cross_file_tag_params: HashMap<Intern<String>, Parameters>,
    /// Function return types from other files: def name → return Ty.
    pub cross_file_fn_return_types: HashMap<DefId, crate::ty::Ty>,
    pub cross_file_callable_signatures: HashMap<DefId, TypedCallableSignature>,
    /// Variant map entries from other files.
    pub cross_file_variant_map: HashMap<Intern<String>, Vec<VariantMapEntry>>,
    /// Merged package AST used to evaluate compile-time trait bodies.
    pub compile_time_eval_ast: Arc<FileAst>,
    /// IDE package pipeline: skip comptime inlining and heavy Reflectable synthesis.
    pub ide_package: bool,
}

impl TransformCtx {
    pub fn new() -> Self {
        Self {
            cross_file_tag_types: HashMap::new(),
            cross_file_tag_params: HashMap::new(),
            cross_file_fn_return_types: HashMap::new(),
            cross_file_callable_signatures: HashMap::new(),
            cross_file_variant_map: HashMap::new(),
            compile_time_eval_ast: Arc::new(FileAst::empty_for_tests()),
            ide_package: false,
        }
    }

    pub fn with_package_compile_time(compile_time_eval_ast: FileAst) -> Self {
        Self::with_package_compile_time_arc(Arc::new(compile_time_eval_ast))
    }

    pub fn with_package_compile_time_arc(compile_time_eval_ast: Arc<FileAst>) -> Self {
        Self {
            compile_time_eval_ast,
            ..Self::new()
        }
    }
}

impl Default for TransformCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformCtx {
    /// This collects tag types, function return types, and variant maps from all files
    /// so that a new file being transformed can resolve references to symbols in these files.
    pub fn from_typed_asts(asts: &[&TypedFileAst]) -> Self {
        let mut ctx = Self::new();
        // Each `ast.variant_map` carries both its own variants and the cross-file
        // entries spliced in during `stage_declare`, so we deduplicate by routing
        // every variant entry through the tag that owns it. Without this each
        // additional file would re-extend the variant Vec with prior files' entries.
        for ast in asts {
            for (tag_id, tag) in &ast.tags {
                ctx.cross_file_tag_types
                    .insert(*tag_id, tag.resolved_ty.clone());
                if let Some(params) = &tag.params {
                    ctx.cross_file_tag_params.insert(tag_id.0, params.clone());
                }
            }
            for (def_id, bind) in &ast.defs {
                ctx.cross_file_fn_return_types
                    .insert(*def_id, bind.return_type.clone());
                ctx.cross_file_callable_signatures
                    .insert(*def_id, TypedCallableSignature::from(bind));
            }
        }

        ctx.cross_file_variant_map = crate::collect_package_variant_map(asts);
        ctx
    }
}

/// Stage 1 only: resolve declarations and build the file-level index.
pub fn transform_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    stage_declare(file_ast, file_id, ctx)
}

/// Stages 2–3 on an AST produced by [`transform_declare`].
///
/// Merges cross-file defs from `ctx` into `typed.fn_return_types` before lowering so
/// forward references across files resolve regardless of compilation order.
pub fn transform_finish(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    lower_file(typed, file_ast, ctx);
    effects::stage_infer_effects(typed, &ctx.cross_file_callable_signatures);
    finish_lowered_file(typed, file_ast, ctx);
}

fn lower_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    for (def_id, return_ty) in &ctx.cross_file_fn_return_types {
        typed
            .fn_return_types
            .entry(*def_id)
            .or_insert_with(|| return_ty.clone());
    }
    stage_lower(typed, file_ast, ctx);
}

fn finish_lowered_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    stage_validate_consumption(typed);
    let trait_registry = crate::compile_time_trait::CompileTimeTraitRegistry::from_parse_ast(
        file_ast,
        Arc::clone(&ctx.compile_time_eval_ast),
    );
    stage_flow(typed, &trait_registry, &ctx.cross_file_callable_signatures);
    if !ctx.ide_package {
        inline_comptime_calls::stage_inline_comptime_calls(typed, file_ast, ctx);
    }
}

/// Options for [`transform_package`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageTransformOptions {
    /// When true, omit merged eval AST and comptime inlining (LSP / diagnostics).
    pub ide_package: bool,
}

impl PackageTransformOptions {
    pub const IDE: Self = Self { ide_package: true };
    pub const FULL: Self = Self { ide_package: false };
}

fn merge_declared_file_into_ctx(ctx: &mut TransformCtx, typed: &TypedFileAst, file_ast: &FileAst) {
    for (tag_id, tag) in &typed.tags {
        ctx.cross_file_tag_types
            .insert(*tag_id, tag.resolved_ty.clone());
    }
    for (def_id, bind) in &typed.defs {
        ctx.cross_file_fn_return_types
            .insert(*def_id, bind.return_type.clone());
        ctx.cross_file_callable_signatures
            .insert(*def_id, TypedCallableSignature::from(bind));
    }
    // `typed.variant_map` contains both this file's own variants *and* the cross-file
    // entries spliced in during `stage_declare`. Re-inserting the cross-file entries
    // each iteration caused exponential blow-up (each file doubled the previous file's
    // variant entries), so only merge variants owned by tags declared in this file.
    let own_tags: HashSet<Intern<String>> = file_ast.tags.keys().copied().collect();
    for (variant_name, entries) in &typed.variant_map {
        let own: Vec<_> = entries
            .iter()
            .filter(|(union_name, _, _)| own_tags.contains(union_name))
            .cloned()
            .collect();
        if own.is_empty() {
            continue;
        }
        ctx.cross_file_variant_map
            .entry(*variant_name)
            .or_default()
            .extend(own);
    }
}

/// Map `use dep.(folder.Symbol)` aliases onto types from already-declared package files.
fn enrich_ctx_from_symbol_aliases(file_ast: &FileAst, ctx: &mut TransformCtx) {
    for alias in &file_ast.symbol_aliases {
        let tag_name = alias
            .target
            .segments
            .last()
            .copied()
            .unwrap_or(alias.target.root);
        if let Some(ty) = ctx.cross_file_tag_types.get(&TagId(tag_name)) {
            ctx.cross_file_tag_types
                .insert(TagId(alias.alias), ty.clone());
        }
    }
}

pub fn transform_package(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
) -> Vec<TypedFileAst> {
    let mut accumulated_ctx = TransformCtx {
        compile_time_eval_ast: Arc::clone(&package_ctx.compile_time_eval_ast),
        ide_package: opts.ide_package,
        ..TransformCtx::new()
    };

    let mut declared: Vec<TypedFileAst> = Vec::with_capacity(file_asts.len());
    for (file_ast, file_id) in file_asts {
        enrich_ctx_from_symbol_aliases(file_ast, &mut accumulated_ctx);
        let typed = transform_declare(file_ast, *file_id, &accumulated_ctx);
        merge_declared_file_into_ctx(&mut accumulated_ctx, &typed, file_ast);
        declared.push(typed);
    }

    // Per-file declare already runs a tag fixpoint with `accumulated_ctx`. A second
    // package-wide fixpoint re-resolved every tag and blew memory/time on gin_core;
    // cross-file declare order + merged variant index covers IDE needs.

    let declared_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let mut full_ctx = TransformCtx::from_typed_asts(&declared_refs);
    full_ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    full_ctx.ide_package = opts.ide_package;
    for (def_id, signature) in &package_ctx.cross_file_callable_signatures {
        full_ctx
            .cross_file_callable_signatures
            .entry(*def_id)
            .or_insert_with(|| signature.clone());
    }

    for (typed, (file_ast, _)) in declared.iter_mut().zip(file_asts.iter()) {
        lower_file(typed, file_ast, &full_ctx);
    }

    effects::stage_infer_package_effects(
        &mut declared,
        &package_ctx.cross_file_callable_signatures,
    );

    let converged_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let mut converged_ctx = TransformCtx::from_typed_asts(&converged_refs);
    converged_ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    converged_ctx.ide_package = opts.ide_package;
    for (def_id, signature) in &package_ctx.cross_file_callable_signatures {
        converged_ctx
            .cross_file_callable_signatures
            .entry(*def_id)
            .or_insert_with(|| signature.clone());
    }

    for (typed, (file_ast, _)) in declared.iter_mut().zip(file_asts.iter()) {
        finish_lowered_file(typed, file_ast, &converged_ctx);
    }

    declared
}

/// Attach cross-file tag/variant index for hover on a single transformed file.
///
/// Package transforms must not call this per file (see [`collect_package_variant_map`]).
fn splice_cross_file_for_single_file(typed: &mut TypedFileAst, ctx: &TransformCtx) {
    for (tag_id, ty) in &ctx.cross_file_tag_types {
        typed.tag_types.entry(*tag_id).or_insert_with(|| ty.clone());
    }
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        typed
            .variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }
}

/// This is the main entry point for external callers (ginc, ginlsp, database).
/// It wraps parsing output into a `FileAst`, assigns a `FileId`, and runs
/// the 3-stage pipeline.
pub fn transform_file(file_ast: FileAst, file_id: FileId) -> TypedFileAst {
    let ctx = TransformCtx::new();
    transform(&file_ast, file_id, &ctx)
}

/// Use this when transforming files that may reference symbols from other files.
/// The `ctx` should be built from already-transformed files via `TransformCtx::from_typed_asts`.
pub fn transform_file_with_ctx(
    file_ast: &FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
) -> TypedFileAst {
    transform(file_ast, file_id, ctx)
}

pub fn transform(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = transform_declare(file_ast, file_id, ctx);
    // Propagate parse-level diagnostics from the AST.
    typed.parse_warnings = file_ast.parse_warnings.clone();
    transform_finish(&mut typed, file_ast, ctx);
    splice_cross_file_for_single_file(&mut typed, ctx);
    typed
}

#[cfg(test)]
mod dependent_local_tests {
    use super::*;
    use crate::ty::Ty;
    use crate::typed::{BindBody, ExprId, TypedExprKind};
    use ast::{BinderOwner, ConstExpr, DependentArgId, TyArg};
    use internment::Intern;
    use parser::cursor::TokenCursor;

    fn body_exprs(typed: &TypedFileAst, name: &str) -> Vec<ExprId> {
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref(name)))
            .expect("definition");
        match &bind.body {
            BindBody::Body { exprs, ret } => {
                exprs.iter().copied().chain(ret.iter().copied()).collect()
            }
            BindBody::Expr(expr) => vec![*expr],
            BindBody::Extern => Vec::new(),
        }
    }

    fn local_bind(typed: &TypedFileAst, name: &str) -> ExprId {
        body_exprs(typed, name)
            .into_iter()
            .find(|id| matches!(typed.exprs.kind[id.as_usize()], TypedExprKind::Bind { .. }))
            .expect("local bind")
    }

    fn inferred(ty: &Ty, index: usize) -> DependentArgId {
        let params = match ty {
            Ty::Record {
                resolved_params, ..
            }
            | Ty::Union {
                resolved_params, ..
            } => resolved_params.as_ref().expect("resolved params"),
            _ => panic!("dependent nominal type"),
        };
        match &params[index].1 {
            TyArg::Const(ConstExpr::Inferred(id)) => *id,
            arg => panic!("inferred argument, got {arg:?}"),
        }
    }

    fn source(body: &str) -> String {
        format!("List(x, length Int: ?, capacity Int: ?, state Int: ?) has value x\n{body}")
    }

    #[test]
    fn fresh_local_uses_expression_binder_without_informative_initializer() {
        let ast = TokenCursor::parse_source(&source(
            "fresh(expression x) List(Int):\n    value List(Int): expression\nreturn value\n",
        ));
        let typed = transform(&ast, FileId(11), &TransformCtx::new());
        let bind_id = local_bind(&typed, "fresh");
        let witness = inferred(&typed.exprs.ty[bind_id.as_usize()], 1);

        assert_eq!(witness.binder.file, 11);
        assert_eq!(witness.binder.owner, BinderOwner::Expression(bind_id.0));
    }

    #[test]
    fn local_projection_preserves_complete_initializer_arguments() {
        let ast = TokenCursor::parse_source(&source(
            "project(expression List(Int, 3, 8, s)) List(Int):\n    value List(Int): expression\nreturn value\n",
        ));
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind_id = local_bind(&typed, "project");
        let local_ty = &typed.exprs.ty[bind_id.as_usize()];
        let input_ty = &typed.defs[&DefId(Intern::from_ref("project"))].params[0].1;

        assert_eq!(local_ty, input_ty);
    }

    #[test]
    fn repeated_local_references_use_the_stored_resolved_type() {
        let ast = TokenCursor::parse_source(&source(
            "stable(expression x) List(Int):\n    value List(Int): expression\n    value\nreturn value\n",
        ));
        let typed = transform(&ast, FileId(4), &TransformCtx::new());
        let exprs = body_exprs(&typed, "stable");
        let bind_id = local_bind(&typed, "stable");
        let expected = typed.exprs.ty[bind_id.as_usize()].clone();

        for id in exprs.into_iter().filter(|id| {
            matches!(
                &typed.exprs.kind[id.as_usize()],
                TypedExprKind::FnCall { target, args, .. }
                    if target.0.as_str() == "value" && args.as_ref().is_none_or(Vec::is_empty)
            )
        }) {
            assert_eq!(typed.exprs.ty[id.as_usize()], expected);
        }
    }

    #[test]
    fn local_witnesses_are_distinct_by_bind_and_file() {
        let source = source(
            "locals(left x, right x) List(Int):\n    first List(Int): left\n    second List(Int): right\nreturn first\n",
        );
        let ast = TokenCursor::parse_source(&source);
        let first_file = transform(&ast, FileId(7), &TransformCtx::new());
        let second_file = transform(&ast, FileId(8), &TransformCtx::new());
        let local_ids: Vec<_> = body_exprs(&first_file, "locals")
            .into_iter()
            .filter(|id| {
                matches!(
                    first_file.exprs.kind[id.as_usize()],
                    TypedExprKind::Bind { .. }
                )
            })
            .collect();
        let first = inferred(&first_file.exprs.ty[local_ids[0].as_usize()], 1);
        let second = inferred(&first_file.exprs.ty[local_ids[1].as_usize()], 1);
        let other_file_id = local_bind(&second_file, "locals");
        let other_file = inferred(&second_file.exprs.ty[other_file_id.as_usize()], 1);

        assert_ne!(first, second);
        assert_ne!(first, other_file);
    }
}

#[cfg(test)]
mod target_group_tests {
    use super::*;
    use crate::typed::{
        BindBody, EffectTarget, ExprId, GroupId, ReferenceTargetGroup, TypedExprKind,
    };
    use internment::Intern;
    use parser::cursor::TokenCursor;
    use std::collections::HashSet;

    fn return_expr(typed: &TypedFileAst, name: &str) -> ExprId {
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref(name)))
            .expect("definition");
        match &bind.body {
            BindBody::Expr(expr) => *expr,
            BindBody::Body {
                ret: Some(expr), ..
            } => *expr,
            _ => panic!("definition has no return expression"),
        }
    }

    #[test]
    fn reference_returning_call_substitutes_actual_target_group() {
        let ast = TokenCursor::parse_source(
            "first(ref values List(x)) ref x: values\ncaller(ref input List(x)) ref x: first(ref input)\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let call = return_expr(&typed, "caller");

        assert!(matches!(
            typed.exprs.kind[call.as_usize()],
            TypedExprKind::FnCall { .. }
        ));
        assert_eq!(
            typed.exprs.target_group[call.as_usize()],
            Some(ReferenceTargetGroup::Param(GroupId(0)))
        );
    }

    #[test]
    fn cross_file_call_substitutes_actual_target_group() {
        let callee_ast = TokenCursor::parse_source("first(ref values List(x)) ref x: values\n");
        let callee = transform(&callee_ast, FileId(0), &TransformCtx::new());
        let ctx = TransformCtx::from_typed_asts(&[&callee]);
        let caller_ast =
            TokenCursor::parse_source("caller(ref input List(x)) ref x: first(ref input)\n");
        let caller = transform(&caller_ast, FileId(1), &ctx);
        let call = return_expr(&caller, "caller");

        assert_eq!(
            caller.exprs.target_group[call.as_usize()],
            Some(ReferenceTargetGroup::Param(GroupId(0)))
        );
        assert!(
            !caller
                .all_flaws()
                .iter()
                .any(|(_, flaw)| { flaw.code.slug().starts_with("type-reference-return-") })
        );
    }

    #[test]
    fn shared_callee_group_rejects_conflicting_actual_targets() {
        let ast = TokenCursor::parse_source(
            "choose(ref{r} left x, ref{r} right x) ref{r} x: left\ncaller(ref{a} left x, ref{b} right x) ref{a} x: choose(ref left, ref right)\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| { flaw.code.slug() == "type-conflicting-target-group-arguments" })
        );
    }

    #[test]
    fn reference_return_rejects_wrong_parameter_group() {
        let ast =
            TokenCursor::parse_source("wrong(ref{a} left x, ref{b} right x) ref{a} x: right\n");
        let typed = transform(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed.all_flaws().iter().any(|(_, flaw)| {
                flaw.code.slug() == "type-reference-return-wrong-target-group"
            })
        );
    }

    #[test]
    fn field_write_effect_is_precise_and_does_not_invalidate_group() {
        let ast =
            TokenCursor::parse_source("Cell has value Int\nset(mut cell Cell): cell.value: 1\n");
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref("set")))
            .expect("definition");

        assert_eq!(
            bind.effects.writes,
            HashSet::from([EffectTarget::Field {
                base: Box::new(EffectTarget::Group(GroupId(0))),
                index: 0,
            }])
        );
        assert!(bind.effects.invalidates.is_empty());
    }

    #[test]
    fn distinct_item_writes_keep_exact_indices() {
        let ast = TokenCursor::parse_source(
            "write(mut items x):\n    items.(0): 1\n    items.(1): 2\nreturn\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref("write")))
            .expect("definition");
        let base = EffectTarget::Group(GroupId(0));

        assert_eq!(
            bind.effects.writes,
            HashSet::from([
                EffectTarget::ItemRegion {
                    base: Box::new(base.clone()),
                    index: crate::TargetIndex::Symbolic(ast::ConstExpr::Value(
                        ast::ConstValue::Int(0),
                    )),
                },
                EffectTarget::ItemRegion {
                    base: Box::new(base),
                    index: crate::TargetIndex::Symbolic(ast::ConstExpr::Value(
                        ast::ConstValue::Int(1),
                    )),
                },
            ])
        );
    }

    #[test]
    fn fixed_array_set_writes_exact_item_without_invalidation() {
        let ast = TokenCursor::parse_source(
            "set(mut array x, index Int, value Int): array.(index): value\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref("set")))
            .expect("definition");

        assert_eq!(
            bind.effects.writes,
            HashSet::from([EffectTarget::ItemRegion {
                base: Box::new(EffectTarget::Group(GroupId(0))),
                index: crate::TargetIndex::Symbolic(ast::ConstExpr::Var(
                    Intern::from_ref("index",)
                )),
            }])
        );
        assert!(bind.effects.invalidates.is_empty());
    }

    #[test]
    fn array_signature_preserves_symbolic_length() {
        let ast = TokenCursor::parse_source(
            "get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref("get")))
            .expect("definition");

        assert!(matches!(
            &bind.params[0].1,
            crate::ty::Ty::Ref { inner, mutable: false }
                if matches!(inner.as_ref(), crate::ty::Ty::Array { size: ast::ConstExpr::Var(name), .. } if name.as_str() == "n")
        ));
    }

    #[test]
    fn fixed_array_get_returns_indexed_target_in_array_group() {
        let ast = TokenCursor::parse_source("get(ref array x, index Int) ref Int: array.(index)\n");
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let returned = return_expr(&typed, "get");

        assert_eq!(
            typed.exprs.target_group[returned.as_usize()],
            Some(ReferenceTargetGroup::ItemRegion {
                base: Box::new(ReferenceTargetGroup::Param(GroupId(0))),
                index: crate::TargetIndex::Symbolic(ast::ConstExpr::Var(
                    Intern::from_ref("index",)
                )),
            })
        );
    }

    #[test]
    fn terminal_mut_operation_propagates_invalidation_to_caller() {
        let ast = TokenCursor::parse_source(
            "Cell has value Int\ndestroy(mut cell Cell): eat cell\ncaller(mut cell Cell) ref Cell:\n    saved: ref cell\n    destroy(mut cell)\nreturn saved\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let destroy = typed
            .defs
            .get(&DefId(Intern::from_ref("destroy")))
            .expect("definition");

        assert_eq!(
            destroy.effects.invalidates,
            HashSet::from([EffectTarget::Descendants(Box::new(EffectTarget::Group(
                GroupId(0),
            )))])
        );
        let codes: Vec<_> = typed
            .all_flaws()
            .iter()
            .map(|(_, flaw)| flaw.code.slug().to_string())
            .collect();
        assert!(
            codes
                .iter()
                .any(|code| code == "type-use-of-invalidated-reference"),
            "{codes:?}"
        );
    }

    #[test]
    fn package_effects_converge_across_cross_file_recursion() {
        let first =
            TokenCursor::parse_source("Cell has value Int\na(mut cell Cell): b(mut cell)\n");
        let second =
            TokenCursor::parse_source("b(mut cell Cell):\n    a(mut cell)\nreturn eat cell\n");
        let typed = transform_package(
            &[(first, FileId(0)), (second, FileId(1))],
            &TransformCtx::new(),
            PackageTransformOptions::IDE,
        );
        let a = typed[0]
            .defs
            .get(&DefId(Intern::from_ref("a")))
            .expect("definition");
        let b = typed[1]
            .defs
            .get(&DefId(Intern::from_ref("b")))
            .expect("definition");

        let group = EffectTarget::Group(GroupId(0));
        let descendants = EffectTarget::Descendants(Box::new(group.clone()));
        assert_eq!(a.effects.invalidates, HashSet::from([descendants.clone()]));
        assert_eq!(a.effects.consumes, HashSet::from([group.clone()]));
        assert_eq!(b.effects.invalidates, HashSet::from([descendants]));
        assert_eq!(b.effects.consumes, HashSet::from([group]));
    }

    #[test]
    fn cross_file_callable_signature_preserves_effects() {
        let ast =
            TokenCursor::parse_source("Cell has value Int\ndestroy(mut cell Cell): eat cell\n");
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let ctx = TransformCtx::from_typed_asts(&[&typed]);
        let signature = ctx
            .cross_file_callable_signatures
            .get(&DefId(Intern::from_ref("destroy")))
            .expect("signature");

        assert_eq!(
            signature.effects.invalidates,
            HashSet::from([EffectTarget::Descendants(Box::new(EffectTarget::Group(
                GroupId(0),
            )))])
        );
        assert_eq!(
            signature.effects.consumes,
            HashSet::from([EffectTarget::Group(GroupId(0))])
        );
    }

    #[test]
    fn local_replacement_preserves_reference_validity() {
        let ast = TokenCursor::parse_source(
            "keep() ref Int:\n    local: 1\n    saved: ref local\n    local: 2\nreturn saved\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());

        assert!(
            !typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| { flaw.code.slug() == "type-use-of-invalidated-reference" })
        );
    }

    #[test]
    fn ownership_move_invalidates_references_to_source() {
        let ast = TokenCursor::parse_source(
            "Cell has value Int\nsink(cell Cell): eat cell\nmove_ref(cell Cell) ref Cell:\n    saved: ref cell\n    sink(cell)\nreturn saved\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());

        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| { flaw.code.slug() == "type-use-of-invalidated-reference" })
        );
    }

    #[test]
    fn reference_return_rejects_local_target() {
        let ast = TokenCursor::parse_source(
            "wrong(ref input Int) ref Int:\n    local: 1\nreturn ref local\n",
        );
        let typed = transform(&ast, FileId(0), &TransformCtx::new());

        let codes: Vec<_> = typed
            .all_flaws()
            .iter()
            .map(|(_, flaw)| flaw.code.slug().to_string())
            .collect();
        assert!(
            codes
                .iter()
                .any(|code| code == "type-reference-return-local-target"),
            "{codes:?}; target={:?}",
            typed.exprs.target_group[return_expr(&typed, "wrong").as_usize()]
        );
    }
}
