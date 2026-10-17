//! 3-stage transformation pipeline: `FileAst → TypedFileAst`.
//!
//! Stages:
//! 1. **Declare** — Resolve tag/bind declarations, build file-level index, detect declaration flaws.
//! 2. **Lower** — Lower parse expressions to typed expressions, attach type flaws.
//! 3. **Flow** — Compute flow contexts, attach flow flaws (ownership, bounds).

pub(crate) mod semantic_reads;

use crate::analysis::validate_consumption::stage_validate_consumption;
use crate::staging;
use crate::tag_declarations::TagDeclarations;
use diagnostic::code::GenericCode;
use diagnostic::{Diagnostic, DiagnosticCode};

use ast::FileAst;
use ast::prelude::*;
use flask::CompileTarget;
use internment::Intern;

use crate::typed::{
    DefId, FileId, TagId, TypedCallableSignature, TypedFileAst, VariantMap, VariantMapEntry,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod bindings;
mod bounds_check;
mod call_arguments;
mod callable_signatures;
mod check_session;
mod constant_queries;
pub use check_session::{BodyReuse, PackageCheckSession, PackageCheckStats};
mod declaration_catalog;
mod declare;
mod effects;
pub(crate) mod finish_reads;
pub(crate) mod references;
pub use effects::{EffectWork, infer_package_full};
mod flow;
pub(crate) mod implementation_reads;
mod inline_comptime_calls;
mod local_call_effects;
mod lower_exprs;
mod lower_kind;
mod lower_tag;
mod lower_ty;
mod lower_util;
mod lower_when;
mod member_access;
mod overloads;
mod package_transform;
mod parameters;
pub(crate) mod place;
mod postconditions;
mod proof_walk;
mod refinement_terms;
mod refinement_types;
mod return_types;
mod scalar_mutations;
mod sequence;
mod specialize_calls;
mod storage_contracts;
mod symbolic_values;
mod terminal;
mod trait_methods;
mod type_arguments;
mod when_coverage;
pub(crate) mod when_values;

pub(crate) use declare::collect_bind_value_self_ref_spans;
pub use declare::stage_declare;
pub use flow::{RefTracker, stage_flow};
pub use package_transform::{
    PackageTransformArtifacts, transform_package_with_shared_context,
    transform_package_with_shared_context_and_package,
    transform_package_with_shared_context_and_session,
};

pub(crate) fn flaw(
    span_table: &SpanTable,
    span: SpanId,
    code: DiagnosticCode,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic::new(code, message).at_span_id(span, span_table)
}

pub(crate) fn collect_result_exit_alternatives(
    expr: &Expr,
    tag_decls: TagDeclarations<'_>,
    alternatives: &mut Vec<Intern<String>>,
) {
    let label = match expr {
        Expr::AnonymousTag(label) => Some(*label),
        Expr::TagCall(call) if call.qual_path.is_none() && tag_decls.get(&call.name).is_none() => {
            Some(call.name)
        }
        _ => None,
    };
    if let Some(label) = label {
        if !alternatives.contains(&label) {
            alternatives.push(label);
        }
        return;
    }
    match expr {
        Expr::Return(ret) => {
            for exit in &ret.values {
                collect_result_exit_alternatives(&exit.value, tag_decls, alternatives);
            }
        }
        Expr::Block(items) => {
            for (index, item) in items.iter().enumerate() {
                if index + 1 == items.len()
                    || matches!(
                        item.value,
                        Expr::Return(_) | Expr::Block(_) | Expr::When(_) | Expr::Loop(_)
                    )
                {
                    collect_result_exit_alternatives(&item.value, tag_decls, alternatives);
                }
            }
        }
        Expr::If(if_expr) => {
            for statement in &if_expr.body {
                if matches!(statement.value, Expr::If(_) | Expr::Loop(_)) {
                    collect_result_exit_alternatives(&statement.value, tag_decls, alternatives);
                }
            }
            for exit in &if_expr.ret.values {
                collect_result_exit_alternatives(&exit.value, tag_decls, alternatives);
            }
        }
        Expr::Loop(body) => {
            let statements = match body {
                ast::Loop::While(body) => &body.exprs,
                ast::Loop::ForIn(body) => &body.exprs,
            };
            for statement in statements {
                if matches!(statement.value, Expr::If(_) | Expr::Loop(_)) {
                    collect_result_exit_alternatives(&statement.value, tag_decls, alternatives);
                }
            }
        }
        Expr::When(when_expr) => {
            for arm in &when_expr.arms {
                let body = match arm {
                    ast::WhenArm::Cond { body, .. }
                    | ast::WhenArm::Is { body, .. }
                    | ast::WhenArm::Else(body, _) => body,
                };
                collect_result_exit_alternatives(&body.value, tag_decls, alternatives);
            }
        }
        _ => {}
    }
}

/// Cross-file type environment for the transformation pipeline.
///
/// Carries resolved type information from other files so that expressions in
/// this file can resolve cross-file references.
#[derive(Clone, PartialEq)]
pub struct TransformCtx {
    pub package_instance: ast::ty::PackageInstanceKey,
    pub type_registry: crate::TypeRegistry,
    /// Tag types from other files: tag name → resolved Ty.
    pub cross_file_tag_types: HashMap<TagId, crate::ty::Ty>,
    pub cross_file_tag_params: HashMap<Intern<String>, Parameters>,
    /// Function return types from other files: def name → return Ty.
    pub cross_file_fn_return_types: HashMap<DefId, crate::ty::Ty>,
    pub cross_file_callable_signatures: HashMap<DefId, TypedCallableSignature>,
    pub cross_file_terminal_exits: HashSet<ast::ty::DeclarationKey>,
    pub overload_groups: ast::OverloadGroups,
    /// Variant map entries from other files.
    pub cross_file_variant_map: HashMap<Intern<String>, Vec<VariantMapEntry>>,
    /// Merged package AST used to evaluate compile-time trait bodies.
    pub compile_time_eval_ast: Arc<FileAst>,
    /// IDE package pipeline: skip comptime inlining and other expensive synthesis.
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
            cross_file_terminal_exits: HashSet::new(),
            overload_groups: ast::OverloadGroups::new(),
            cross_file_variant_map: HashMap::new(),
            compile_time_eval_ast: Arc::new(FileAst::empty_for_tests()),
            ide_package: false,
            compile_target: CompileTarget::Library,
        }
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
        let _profile = compiler_profile::span(compiler_profile::Phase::Context);
        let mut ctx = Self::new();
        for ast in asts {
            merge_typed_file_into_ctx(&mut ctx, ast);
            for (tag_id, tag) in &*ast.tags {
                if let Some(params) = &tag.params {
                    ctx.cross_file_tag_params.insert(tag_id.0, params.clone());
                }
            }
        }

        // Each `ast.variant_map` carries both its own variants and the cross-file
        // entries spliced in during `stage_declare`, so we deduplicate by routing
        // every variant entry through the tag that owns it. Without this each
        // additional file would re-extend the variant Vec with prior files' entries.
        ctx.cross_file_variant_map = crate::collect_package_variant_map(asts);
        ctx
    }
}

fn merge_typed_file_into_ctx(ctx: &mut TransformCtx, typed: &TypedFileAst) {
    ctx.cross_file_terminal_exits
        .extend(typed.terminal_exit_declarations());
    ctx.overload_groups.extend(
        typed
            .overload_groups
            .iter()
            .map(|(name, ids)| (*name, ids.clone())),
    );
    ctx.type_registry.merge_from(&typed.type_registry);
    for (tag_id, tag) in &*typed.tags {
        ctx.cross_file_tag_types
            .insert(*tag_id, tag.resolved_ty.clone());
    }
    for (def_id, bind) in &typed.defs {
        ctx.cross_file_fn_return_types
            .insert(*def_id, bind.return_type.clone());
        ctx.cross_file_callable_signatures
            .insert(*def_id, TypedCallableSignature::from(bind));
    }
}

/// Stages 2–3 on an AST produced by [`stage_declare`].
///
/// Merges cross-file defs from `ctx` into `typed.fn_return_types` before lowering so
/// forward references across files resolve regardless of compilation order.
pub fn transform_finish(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    lower_file(typed, file_ast, ctx);
    crate::assembly::populate_target_constants(typed, &ctx.compile_target);
    crate::assembly::validate_assembly_expressions(typed, &ctx.compile_target);
    references::infer_rounds(
        std::slice::from_mut(typed),
        &ctx.cross_file_callable_signatures,
        false,
    );
    effects::infer_package_full(
        std::slice::from_mut(typed),
        &ctx.cross_file_callable_signatures,
    );
    finish_lowered_file(typed, file_ast, ctx);
}

fn lower_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    let constants = constant_queries::ConstantQueries::new(Arc::clone(&typed.eval_ast));
    lower_file_with_queries(typed, file_ast, ctx, &constants, None);
}

fn lower_file_with_queries(
    typed: &mut TypedFileAst,
    file_ast: &FileAst,
    ctx: &TransformCtx,
    constants: &constant_queries::ConstantQueries,
    reads: Option<&implementation_reads::ImplementationReadRecorder>,
) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Lowering);
    for (def_id, return_ty) in &ctx.cross_file_fn_return_types {
        typed
            .fn_return_types
            .entry(*def_id)
            .or_insert_with(|| return_ty.clone());
    }
    lower_exprs::stage_lower_with_queries(typed, file_ast, ctx, constants, reads);
    lower_exprs::materialize_owned_sequence_literals(typed);
    place::stage_freeze_place_contracts(typed);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FinishStage {
    Lowered,
    Prepared,
}

impl FinishStage {
    fn initialize(self, typed: &mut TypedFileAst, context: &TransformCtx) {
        if matches!(self, Self::Lowered) {
            typed.type_registry = context.type_registry.clone();
        }
    }

    fn run(self, typed: &mut TypedFileAst, source: &FileAst, context: &TransformCtx) {
        match self {
            Self::Lowered => finish_lowered_file(typed, source, context),
            Self::Prepared => finish_prepared_file(typed, source, context),
        }
    }
}

fn finish_lowered_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    prepare_finishing_file(typed, file_ast, ctx);
    finish_prepared_file(typed, file_ast, ctx);
}

fn prepare_finishing_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Finishing);
    refinement_types::validate(typed, file_ast, &ctx.cross_file_callable_signatures);
    declare::validate_declaration_representations(typed, file_ast, ctx);
    typed.validate_terminal_declarations(file_ast, ctx);
    stage_validate_consumption(typed);
    staging::analyze_availability(typed, &ctx.cross_file_callable_signatures);
    crate::analysis::stage_rematerialize_owned_arguments(
        typed,
        &ctx.cross_file_callable_signatures,
    );
    staging::prepare_resolved_expressions(typed);
}

fn finish_prepared_file(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Finishing);
    stage_flow(typed, &ctx.cross_file_callable_signatures);
    effects::infer_package_full(
        std::slice::from_mut(typed),
        &ctx.cross_file_callable_signatures,
    );
    staging::analyze_availability(typed, &ctx.cross_file_callable_signatures);
    if !ctx.ide_package {
        inline_comptime_calls::stage_inline_comptime_calls(typed, file_ast, ctx);
    }
    crate::constant_object::collect_closed_constants(typed);
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
    merge_typed_file_into_ctx(ctx, typed);
    // `typed.variant_map` contains both this file's own variants *and* the cross-file
    // entries spliced in during `stage_declare`. Re-inserting the cross-file entries
    // each iteration caused exponential blow-up (each file doubled the previous file's
    // variant entries), so only merge variants owned by tags declared in this file.
    let own_tags: HashSet<Intern<String>> = file_ast.tags.keys().copied().collect();
    for (variant_name, entries) in &*typed.variant_map {
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

fn build_package_context(
    package_ctx: &TransformCtx,
    opts: &PackageTransformOptions,
    asts: &[&TypedFileAst],
    compile_target: CompileTarget,
) -> TransformCtx {
    let mut ctx = TransformCtx::from_typed_asts(asts);
    ctx.cross_file_terminal_exits
        .extend(package_ctx.cross_file_terminal_exits.iter().cloned());
    ctx.package_instance = package_ctx.package_instance.clone();
    ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    ctx.ide_package = opts.ide_package;
    ctx.overload_groups
        .extend(package_ctx.overload_groups.clone());
    ctx.compile_target = compile_target;
    for (def_id, signature) in &package_ctx.cross_file_callable_signatures {
        ctx.cross_file_callable_signatures
            .entry(*def_id)
            .or_insert_with(|| signature.clone());
    }
    ctx
}

pub fn transform_package(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
) -> Vec<TypedFileAst> {
    transform_package_impl(file_asts, package_ctx, opts, None)
}

fn transform_package_impl(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
    mut session: Option<&mut PackageCheckSession>,
) -> Vec<TypedFileAst> {
    if !opts.ide_package {
        return transform_package_eager(file_asts, package_ctx, opts, session);
    }
    let _profile = compiler_profile::span(compiler_profile::Phase::FrontEnd);
    let mut files = Cow::Borrowed(file_asts);
    for _ in 0..32 {
        let (mut typed, context) =
            lower_package_once(&files, package_ctx, opts.clone(), session.as_deref_mut());
        let generic = typed.iter().any(|file| {
            file.defs
                .values()
                .any(|bind| bind.requires_specialization())
        });
        if !generic {
            finish_package_once(
                &mut typed,
                &files,
                &context,
                session.as_deref_mut(),
                FinishStage::Lowered,
                &package_ctx.cross_file_callable_signatures,
            );
            return typed;
        }
        prepare_package_finishing(&mut typed, &files, &context, session.as_deref_mut());
        let (asts, expansion) = specialize_package_pending(&files, &typed, session.as_deref_mut());
        if !expansion.expanded {
            finish_package_once(
                &mut typed,
                &files,
                &context,
                session.as_deref_mut(),
                FinishStage::Prepared,
                &package_ctx.cross_file_callable_signatures,
            );
            expansion.publish(&mut typed);
            return typed;
        }
        if let Some(session) = session.as_deref_mut() {
            session.skip_finishing();
        }
        for ((file, _), ast) in files.to_mut().iter_mut().zip(asts) {
            *file = ast;
        }
    }
    let mut typed = transform_package_once(&files, package_ctx, opts, session);
    // type-generic-specialization-limit
    typed[0].parse_warnings.push(Diagnostic::new(
        GenericCode::SpecializationLimit,
        "generic specialization did not converge",
    ));
    typed
}

fn prepare_package_finishing(
    typed: &mut [TypedFileAst],
    files: &[(FileAst, FileId)],
    context: &TransformCtx,
    session: Option<&mut PackageCheckSession>,
) {
    let start = std::time::Instant::now();
    if let Some(session) = session {
        session.prepare_finishing(typed, files, context);
        session.stats_mut().finishing += start.elapsed();
    } else {
        for (typed, (source, _)) in typed.iter_mut().zip(files.iter()) {
            typed.type_registry = context.type_registry.clone();
            prepare_finishing_file(typed, source, context);
        }
    }
}

fn specialize_package_pending(
    files: &[(FileAst, FileId)],
    typed: &[TypedFileAst],
    mut session: Option<&mut PackageCheckSession>,
) -> (Vec<FileAst>, specialize_calls::Expansion) {
    let mut asts = {
        let _clone = compiler_profile::span(compiler_profile::Phase::AstClone);
        files.iter().map(|(ast, _)| ast.clone()).collect::<Vec<_>>()
    };
    let _specialization = compiler_profile::span(compiler_profile::Phase::Specialization);
    let start = std::time::Instant::now();
    let expansion = match session.as_deref_mut() {
        Some(session) => session.specialize_pending(&mut asts, typed),
        None => specialize_calls::expand_pending(&mut asts, typed, None).0,
    };
    if let Some(session) = session {
        session.stats_mut().specialization += start.elapsed();
    }
    (asts, expansion)
}

fn transform_package_eager(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
    mut session: Option<&mut PackageCheckSession>,
) -> Vec<TypedFileAst> {
    let _profile = compiler_profile::span(compiler_profile::Phase::FrontEnd);
    let mut files = Cow::Borrowed(file_asts);
    for _ in 0..32 {
        let mut typed =
            transform_package_once(&files, package_ctx, opts.clone(), session.as_deref_mut());
        if !typed.iter().any(|file| {
            file.defs
                .values()
                .any(|bind| bind.requires_specialization())
        }) {
            return typed;
        }
        let _specialization = compiler_profile::span(compiler_profile::Phase::Specialization);
        let mut asts = {
            let _clone = compiler_profile::span(compiler_profile::Phase::AstClone);
            files.iter().map(|(ast, _)| ast.clone()).collect::<Vec<_>>()
        };
        let start = std::time::Instant::now();
        let expanded = match session.as_deref_mut() {
            Some(session) => session.specialize(&mut asts, &mut typed),
            None => specialize_calls::expand(&mut asts, &mut typed),
        };
        if let Some(session) = session.as_deref_mut() {
            session.stats_mut().specialization += start.elapsed();
        }
        if !expanded {
            return typed;
        }
        for ((file, _), ast) in files.to_mut().iter_mut().zip(asts) {
            *file = ast;
        }
    }
    let mut typed = transform_package_once(&files, package_ctx, opts, session);
    // type-generic-specialization-limit
    typed[0].parse_warnings.push(Diagnostic::new(
        GenericCode::SpecializationLimit,
        "generic specialization did not converge",
    ));
    typed
}

fn transform_package_once(
    files: &[(FileAst, FileId)],
    context: &TransformCtx,
    options: PackageTransformOptions,
    mut session: Option<&mut PackageCheckSession>,
) -> Vec<TypedFileAst> {
    let (mut typed, converged) =
        lower_package_once(files, context, options, session.as_deref_mut());
    finish_package_once(
        &mut typed,
        files,
        &converged,
        session,
        FinishStage::Lowered,
        &context.cross_file_callable_signatures,
    );
    typed
}

fn lower_package_once(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
    mut session: Option<&mut PackageCheckSession>,
) -> (Vec<TypedFileAst>, TransformCtx) {
    let _profile = compiler_profile::span(compiler_profile::Phase::PackagePass);
    let declarations_profile = compiler_profile::span(compiler_profile::Phase::Declarations);
    let start = std::time::Instant::now();
    let prepared_file_asts: Vec<&FileAst> =
        file_asts.iter().map(|(file_ast, _)| file_ast).collect();
    let mut accumulated_ctx = TransformCtx {
        package_instance: package_ctx.package_instance.clone(),
        type_registry: package_ctx.type_registry.clone(),
        cross_file_terminal_exits: package_ctx.cross_file_terminal_exits.clone(),
        compile_time_eval_ast: Arc::clone(&package_ctx.compile_time_eval_ast),
        ide_package: opts.ide_package,
        compile_target: opts.compile_target.clone(),
        ..TransformCtx::new()
    };

    let catalog = declaration_catalog::DeclarationCatalog::collect(file_asts);
    catalog.seed_identities(&mut accumulated_ctx);
    let aliases = catalog.resolve_named_aliases();
    aliases.publish(&mut accumulated_ctx);
    let completed_aliases = aliases.completed_types();
    let declaration_environment = session
        .as_deref_mut()
        .and_then(|session| session.declaration_environment(&accumulated_ctx));

    let mut declarations = Vec::with_capacity(file_asts.len());
    for ((_, file_id), prepared_file_ast) in file_asts.iter().zip(prepared_file_asts.iter()) {
        let mut types = if let Some(session) = session.as_deref_mut() {
            session.declare_types(
                prepared_file_ast,
                *file_id,
                &accumulated_ctx,
                &completed_aliases,
                declaration_environment.as_ref(),
            )
        } else {
            declare::declare_types_with_aliases(
                prepared_file_ast,
                *file_id,
                &accumulated_ctx,
                &completed_aliases,
            )
        };
        catalog.publish_diagnostics(&aliases, &mut types.typed);
        merge_declared_file_into_ctx(&mut accumulated_ctx, &types.typed, prepared_file_ast);
        aliases.publish(&mut accumulated_ctx);
        declarations.push(types);
    }
    if let Some(session) = session.as_deref_mut() {
        session.stats_mut().type_declarations += start.elapsed();
    }
    drop(declarations_profile);
    let signature_start = std::time::Instant::now();
    let mut declared = match session.as_deref_mut() {
        Some(session) => session.resolve_signatures(
            declarations,
            &accumulated_ctx,
            declaration_environment.as_ref(),
        ),
        None => declarations
            .into_iter()
            .map(|types| declare::complete_signatures(types, &accumulated_ctx))
            .collect(),
    };
    if let Some(session) = session.as_deref_mut() {
        session.stats_mut().signatures += signature_start.elapsed();
    }

    let declared_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let full_ctx = build_package_context(
        package_ctx,
        &opts,
        &declared_refs,
        opts.compile_target.clone(),
    );

    if let Some(session) = session.as_deref_mut() {
        let stats = session.stats_mut();
        stats.passes += 1;
        stats.declared_files += declared.len();
        stats.declarations += start.elapsed();
    }
    let start = std::time::Instant::now();
    if let Some(session) = session.as_deref_mut() {
        session.lower(&mut declared, &prepared_file_asts, &full_ctx);
    } else {
        let constants =
            constant_queries::ConstantQueries::new(Arc::clone(&full_ctx.compile_time_eval_ast));
        for (typed, prepared_file_ast) in declared.iter_mut().zip(prepared_file_asts.iter()) {
            check_session::lower_unit(typed, prepared_file_ast, &full_ctx, &constants, None);
        }
    }
    if let Some(session) = session.as_deref_mut() {
        session.stats_mut().lowering += start.elapsed();
    }
    let start = std::time::Instant::now();
    crate::assembly::populate_package_specifications(&mut declared, &full_ctx.compile_target);
    crate::assembly::validate_package_assembly_expressions(&mut declared, &full_ctx.compile_target);
    if let Some(session) = session.as_deref_mut() {
        session.prepare_staging(&mut declared, &package_ctx.cross_file_callable_signatures);
    } else {
        for typed in &mut declared {
            staging::analyze_availability(typed, &package_ctx.cross_file_callable_signatures);
            staging::prepare_resolved_expressions(typed);
        }
    }

    if let Some(session) = session.as_deref_mut() {
        session.infer_references(&mut declared, &package_ctx.cross_file_callable_signatures);
    } else {
        references::infer_rounds(
            &mut declared,
            &package_ctx.cross_file_callable_signatures,
            false,
        );
    }

    if let Some(session) = session.as_deref_mut() {
        session.infer_effects(&mut declared, &package_ctx.cross_file_callable_signatures);
    } else {
        effects::infer_package_full(&mut declared, &package_ctx.cross_file_callable_signatures);
    }

    let converged_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let converged_ctx = build_package_context(
        package_ctx,
        &opts,
        &converged_refs,
        opts.compile_target.clone(),
    );

    if let Some(session) = session.as_deref_mut() {
        session.stats_mut().inference += start.elapsed();
    }
    (declared, converged_ctx)
}

fn finish_package_once(
    declared: &mut [TypedFileAst],
    files: &[(FileAst, FileId)],
    context: &TransformCtx,
    mut session: Option<&mut PackageCheckSession>,
    stage: FinishStage,
    external: &HashMap<DefId, TypedCallableSignature>,
) {
    let prepared_file_asts: Vec<_> = files.iter().map(|(source, _)| source).collect();
    let start = std::time::Instant::now();
    if let Some(session) = session.as_deref_mut() {
        session.finish_with(declared, &prepared_file_asts, context, stage);
        let stats = session.stats_mut();
        stats.finished_files += declared.len();
        stats.finishing += start.elapsed();
    } else {
        for (typed, prepared_file_ast) in declared.iter_mut().zip(prepared_file_asts.iter()) {
            stage.initialize(typed, context);
            stage.run(typed, prepared_file_ast, context);
        }
    }
    let start = std::time::Instant::now();
    if let Some(session) = session.as_deref_mut() {
        session.infer_effects(declared, external);
    } else {
        effects::infer_package_full(declared, external);
    }

    if !context.ide_package {
        inline_comptime_calls::stage_inline_package_calls(declared, &context.compile_target);
    }

    if let Some(session) = session {
        session.stats_mut().inference += start.elapsed();
    }
}

fn splice_cross_file_tag_types(typed: &mut TypedFileAst, ctx: &TransformCtx) {
    for (tag_id, ty) in &ctx.cross_file_tag_types {
        typed.tag_types.entry(*tag_id).or_insert_with(|| ty.clone());
    }
}

fn extend_variant_map_from_ctx(variant_map: &mut VariantMap, ctx: &TransformCtx) {
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }
}

/// Attach cross-file tag/variant index for hover on a single transformed file.
///
/// Package transforms must not call this per file (see [`collect_package_variant_map`]).
fn splice_cross_file_for_single_file(typed: &mut TypedFileAst, ctx: &TransformCtx) {
    splice_cross_file_tag_types(typed, ctx);
    extend_variant_map_from_ctx(&mut typed.variant_map, ctx);
}

/// This is the main entry point for external callers (ginc, ginlsp, database).
/// It wraps parsing output into a `FileAst`, assigns a `FileId`, and runs
/// the 3-stage pipeline.
pub fn transform_file(file_ast: FileAst, file_id: FileId) -> TypedFileAst {
    let ctx = TransformCtx::new();
    transform(&file_ast, file_id, &ctx)
}

pub fn member_completions(
    file_ast: &FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
    receiver: std::ops::Range<usize>,
) -> Vec<crate::CompletionCandidate> {
    let mut typed = stage_declare(file_ast, file_id, ctx);
    typed.completion_probe = Some((receiver.start, receiver.end));
    transform_finish(&mut typed, file_ast, ctx);
    typed.member_completions
}

pub fn transform(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut file = Cow::Borrowed(file_ast);
    for _ in 0..32 {
        let mut typed = [transform_once(&file, file_id, ctx)];
        if !typed[0]
            .defs
            .values()
            .any(|bind| bind.requires_specialization())
        {
            return typed.into_iter().next().unwrap();
        }
        let mut files = [file.as_ref().clone()];
        if !specialize_calls::expand(&mut files, &mut typed) {
            return typed.into_iter().next().unwrap();
        }
        file = Cow::Owned(files.into_iter().next().unwrap());
    }
    let mut typed = transform_once(&file, file_id, ctx);
    // type-generic-specialization-limit
    typed.parse_warnings.push(Diagnostic::new(
        GenericCode::SpecializationLimit,
        "generic specialization did not converge",
    ));
    typed
}

fn transform_once(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = stage_declare(file_ast, file_id, ctx);
    // Propagate parse-level diagnostics from the AST.
    typed.parse_warnings = crate::typed::Shared::new(file_ast.parse_warnings.clone());
    transform_finish(&mut typed, file_ast, ctx);
    splice_cross_file_for_single_file(&mut typed, ctx);
    typed
}
