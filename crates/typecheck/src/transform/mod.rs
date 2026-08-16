//! 3-stage transformation pipeline: `FileAst → TypedFileAst`.
//!
//! Stages:
//! 1. **Declare** — Resolve tag/bind declarations, build file-level index, detect declaration flaws.
//! 2. **Lower** — Lower parse expressions to typed expressions, attach type flaws.
//! 3. **Flow** — Compute flow contexts, attach flow flaws (ownership, bounds).

use crate::analysis::validate_consumption::stage_validate_consumption;
use crate::staging;

use ast::FileAst;
use ast::prelude::*;
use flask::CompileTarget;

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
mod package_transform;
mod place;

pub(crate) use declare::collect_bind_value_self_ref_spans;
pub use declare::stage_declare;
pub use flow::{RefTracker, stage_flow};
pub use lower_exprs::stage_lower;
pub use package_transform::{
    PackageTransformArtifacts, transform_package_with_shared_context,
    transform_package_with_shared_context_and_package,
};

/// Cross-file type environment for the transformation pipeline.
///
/// Carries resolved type information from other files so that expressions in
/// this file can resolve cross-file references.
pub struct TransformCtx {
    pub package_instance: ast::ty::PackageInstanceKey,
    pub type_registry: crate::TypeRegistry,
    /// Tag types from other files: tag name → resolved Ty.
    pub cross_file_tag_types: HashMap<TagId, crate::ty::Ty>,
    pub cross_file_tag_params: HashMap<Intern<String>, Parameters>,
    /// Function return types from other files: def name → return Ty.
    pub cross_file_fn_return_types: HashMap<DefId, crate::ty::Ty>,
    pub cross_file_callable_signatures: HashMap<DefId, TypedCallableSignature>,
    pub integer_literal_defaults: Vec<crate::ty::Ty>,
    pub raw_pointer_defaults: Vec<crate::ty::Ty>,
    pub fixed_array_defaults: Vec<crate::ty::Ty>,
    /// Variant map entries from other files.
    pub cross_file_variant_map: HashMap<Intern<String>, Vec<VariantMapEntry>>,
    /// Merged package AST used to evaluate compile-time trait bodies.
    pub compile_time_eval_ast: Arc<FileAst>,
    /// IDE package pipeline: skip comptime inlining and heavy Reflectable synthesis.
    pub ide_package: bool,
    pub compile_target: CompileTarget,
}

impl TransformCtx {
    pub fn new() -> Self {
        Self {
            package_instance: ast::ty::PackageInstanceKey::workspace("anonymous", "0"),
            type_registry: crate::TypeRegistry::default(),
            cross_file_tag_types: HashMap::new(),
            cross_file_tag_params: HashMap::new(),
            cross_file_fn_return_types: HashMap::new(),
            cross_file_callable_signatures: HashMap::new(),
            integer_literal_defaults: Vec::new(),
            raw_pointer_defaults: Vec::new(),
            fixed_array_defaults: Vec::new(),
            cross_file_variant_map: HashMap::new(),
            compile_time_eval_ast: Arc::new(FileAst::empty_for_tests()),
            ide_package: false,
            compile_target: CompileTarget::Library,
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

    pub fn with_package_instance(mut self, package: ast::ty::PackageInstanceKey) -> Self {
        self.package_instance = package;
        self
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
            ctx.type_registry.merge_from(&ast.type_registry);
            for (tag_id, tag) in &ast.tags {
                ctx.cross_file_tag_types
                    .insert(*tag_id, tag.resolved_ty.clone());
                if let Some(params) = &tag.params {
                    ctx.cross_file_tag_params.insert(tag_id.0, params.clone());
                }
                if tag.attributes.default_literal == Some(ast::ty::LiteralKind::Integer) {
                    ctx.integer_literal_defaults.push(tag.resolved_ty.clone());
                }
                if tag.attributes.default_type_former
                    == Some(ast::DefaultTypeFormerRole::RawPointer)
                {
                    ctx.raw_pointer_defaults.push(tag.resolved_ty.clone());
                }
                if tag.attributes.default_type_former
                    == Some(ast::DefaultTypeFormerRole::FixedArray)
                {
                    ctx.fixed_array_defaults.push(tag.resolved_ty.clone());
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

/// Stages 2–3 on an AST produced by [`stage_declare`].
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
    place::stage_freeze_place_contracts(typed);
}

fn finish_lowered_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    stage_validate_consumption(typed);
    staging::analyze_availability(typed, &ctx.cross_file_callable_signatures);
    staging::prepare_resolved_expressions(typed);
    crate::analysis::stage_rematerialize_owned_arguments(
        typed,
        &ctx.cross_file_callable_signatures,
    );
    stage_flow(typed, &ctx.cross_file_callable_signatures);
    if !ctx.ide_package {
        inline_comptime_calls::stage_inline_comptime_calls(typed, file_ast, ctx);
    }
}

/// Options for [`transform_package`].
#[derive(Clone, Debug)]
pub struct PackageTransformOptions {
    /// When true, omit merged eval AST and comptime inlining (LSP / diagnostics).
    pub ide_package: bool,
    pub compile_target: CompileTarget,
}

impl PackageTransformOptions {
    pub const IDE: Self = Self {
        ide_package: true,
        compile_target: CompileTarget::Library,
    };
    pub const FULL: Self = Self {
        ide_package: false,
        compile_target: CompileTarget::Library,
    };

    pub fn with_compile_target(mut self, compile_target: CompileTarget) -> Self {
        self.compile_target = compile_target;
        self
    }
}

impl Default for PackageTransformOptions {
    fn default() -> Self {
        Self::FULL
    }
}

fn merge_declared_file_into_ctx(ctx: &mut TransformCtx, typed: &TypedFileAst, file_ast: &FileAst) {
    ctx.type_registry.merge_from(&typed.type_registry);
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

pub fn transform_package(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
) -> Vec<TypedFileAst> {
    let prepared_file_asts: Vec<FileAst> = file_asts
        .iter()
        .map(|(file_ast, _)| file_ast.clone())
        .collect();
    let mut accumulated_ctx = TransformCtx {
        package_instance: package_ctx.package_instance.clone(),
        type_registry: package_ctx.type_registry.clone(),
        compile_time_eval_ast: Arc::clone(&package_ctx.compile_time_eval_ast),
        ide_package: opts.ide_package,
        compile_target: opts.compile_target.clone(),
        ..TransformCtx::new()
    };

    let mut declared: Vec<TypedFileAst> = Vec::with_capacity(file_asts.len());
    for ((_, file_id), prepared_file_ast) in file_asts.iter().zip(prepared_file_asts.iter()) {
        let typed = stage_declare(prepared_file_ast, *file_id, &accumulated_ctx);
        merge_declared_file_into_ctx(&mut accumulated_ctx, &typed, prepared_file_ast);
        declared.push(typed);
    }

    // Per-file declare already runs a tag fixpoint with `accumulated_ctx`. A second
    // package-wide fixpoint re-resolved every tag and blew memory/time on gin_core;
    // cross-file declare order + merged variant index covers IDE needs.

    let declared_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let mut full_ctx = TransformCtx::from_typed_asts(&declared_refs);
    full_ctx.package_instance = package_ctx.package_instance.clone();
    full_ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    full_ctx.ide_package = opts.ide_package;
    full_ctx.compile_target = opts.compile_target.clone();
    for (def_id, signature) in &package_ctx.cross_file_callable_signatures {
        full_ctx
            .cross_file_callable_signatures
            .entry(*def_id)
            .or_insert_with(|| signature.clone());
    }

    for (typed, prepared_file_ast) in declared.iter_mut().zip(prepared_file_asts.iter()) {
        typed.type_registry = full_ctx.type_registry.clone();
        lower_file(typed, prepared_file_ast, &full_ctx);
    }

    effects::stage_infer_package_effects(
        &mut declared,
        &package_ctx.cross_file_callable_signatures,
    );

    let converged_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let mut converged_ctx = TransformCtx::from_typed_asts(&converged_refs);
    converged_ctx.package_instance = package_ctx.package_instance.clone();
    converged_ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    converged_ctx.ide_package = opts.ide_package;
    converged_ctx.compile_target = opts.compile_target;
    for (def_id, signature) in &package_ctx.cross_file_callable_signatures {
        converged_ctx
            .cross_file_callable_signatures
            .entry(*def_id)
            .or_insert_with(|| signature.clone());
    }

    for (typed, prepared_file_ast) in declared.iter_mut().zip(prepared_file_asts.iter()) {
        typed.type_registry = converged_ctx.type_registry.clone();
        finish_lowered_file(typed, prepared_file_ast, &converged_ctx);
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

pub fn transform(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let prepared = file_ast.clone();
    let mut typed = stage_declare(&prepared, file_id, ctx);
    // Propagate parse-level diagnostics from the AST.
    typed.parse_warnings = prepared.parse_warnings.clone();
    transform_finish(&mut typed, &prepared, ctx);
    splice_cross_file_for_single_file(&mut typed, ctx);
    typed
}

#[cfg(test)]
#[path = "../../tests/transform_mod_dependent_local_tests.rs"]
mod dependent_local_tests;

#[cfg(test)]
#[path = "../../tests/transform_mod_target_group_tests.rs"]
mod target_group_tests;
