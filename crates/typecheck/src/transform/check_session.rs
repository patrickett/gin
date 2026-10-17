use std::collections::HashMap;
use std::time::Duration;

use ast::FileAst;
use rayon::prelude::*;

use super::finish_reads::{self, FinishReadLog};
use super::implementation_reads::{ImplementationReadRecorder, ImplementationReads};
use super::{FinishStage, PackageTransformOptions, TransformCtx, lower_file_with_queries};
use crate::{FileId, TypedFileAst};

mod cache_inputs;
mod declarations;
mod function_queries;
mod semantic_queries;
use cache_inputs::{same_package_declarations, same_semantic_context};
pub(super) use function_queries::{FunctionInput, FunctionQueries, Relocation};
pub(super) use semantic_queries::{FileQueries, SourceQueries, intern_capped};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyReuse {
    Disabled,
    ExactInputs,
}

#[derive(Clone, Debug, Default)]
pub struct PackageCheckStats {
    pub passes: usize,
    pub retained_passes: usize,
    pub declared_files: usize,
    pub type_declaration_checks: usize,
    pub type_declaration_reused: usize,
    pub signature_checks: usize,
    pub signature_reused: usize,
    pub lowered_files: usize,
    pub reused_files: usize,
    pub finished_files: usize,
    pub finishing_reused: usize,
    pub lowered_expressions: usize,
    pub reference_checks: usize,
    pub reference_checks_reused: usize,
    pub retained_function_answers: usize,
    pub retained_function_expressions: usize,
    pub retained_semantic_dependencies: usize,
    pub reference_rounds: usize,
    pub effect_checks: usize,
    pub effect_checks_skipped: usize,
    pub effect_checks_reused: usize,
    pub staging_checks: usize,
    pub staging_reused: usize,
    pub constant_queries: usize,
    pub constant_environments: usize,
    pub declarations: Duration,
    pub type_declarations: Duration,
    pub signatures: Duration,
    pub lowering: Duration,
    pub inference: Duration,
    pub finishing: Duration,
    pub specialization: Duration,
    pub specialization_checks: usize,
    pub specialization_reused: usize,
}

pub struct PackageCheckSession {
    pool: Option<std::sync::Arc<rayon::ThreadPool>>,
    reuse: BodyReuse,
    semantic_reuse: bool,
    function_reuse: bool,
    retained_pass_limit: Option<std::num::NonZeroUsize>,
    effect_queries: bool,
    parallel_references: bool,
    finishing_reuse: bool,
    generations: Vec<LoweringGeneration>,
    references: super::references::ReferenceQueries,
    effects: super::effects::EffectQueries,
    declarations: declarations::DeclarationQueries,
    specializations: super::specialize_calls::SpecializationQueries,
    staging: FileQueries<(), crate::typed::Shared<crate::typed::TypedExprVec>>,
    stats: PackageCheckStats,
}

struct LoweringGeneration {
    context: TransformCtx,
    files: HashMap<FileId, LoweredFile>,
    constants: std::sync::Arc<super::constant_queries::ConstantQueries>,
    finished: HashMap<FileId, TypedFileAst>,
    finish_inputs: HashMap<FileId, TypedFileAst>,
    read_logs: HashMap<FileId, FinishReadLog>,
    finish_stage: Option<FinishStage>,
}

impl LoweringGeneration {
    fn clear_finishing(&mut self) {
        self.finished.clear();
        self.finish_inputs.clear();
        self.read_logs.clear();
        self.finish_stage = None;
    }
}

struct LoweredFile {
    source: std::sync::Arc<FileAst>,
    declaration: TypedFileAst,
    result: TypedFileAst,
    reads: ImplementationReads,
    reused: bool,
}

impl PackageCheckSession {
    pub fn new(
        workers: std::num::NonZeroUsize,
        reuse: BodyReuse,
    ) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pool = if workers.get() == 1 {
            None
        } else {
            Some(std::sync::Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(workers.get())
                    .build()?,
            ))
        };
        let declarations = declarations::DeclarationQueries::default();
        let specializations =
            super::specialize_calls::SpecializationQueries::new(declarations.sources());
        Ok(Self {
            pool,
            reuse,
            semantic_reuse: false,
            function_reuse: false,
            retained_pass_limit: None,
            effect_queries: false,
            parallel_references: false,
            finishing_reuse: false,
            generations: Vec::new(),
            references: Default::default(),
            effects: Default::default(),
            declarations,
            specializations,
            staging: Default::default(),
            stats: PackageCheckStats::default(),
        })
    }

    /// Reuse finishing (`flow` and later stages) for files whose recorded
    /// cross-file reads are unchanged. Opt-in: it trades memory for latency on
    /// edit-heavy call workloads.
    pub fn with_finishing_reuse(mut self) -> Self {
        self.finishing_reuse = true;
        self
    }

    pub fn with_parallel_references(mut self) -> Self {
        self.parallel_references = true;
        self
    }

    pub fn with_semantic_reuse(mut self) -> Self {
        self.semantic_reuse = true;
        self
    }

    pub fn with_function_reuse(mut self) -> Self {
        self.function_reuse = true;
        self.effect_queries = true;
        self
    }

    fn function_reuse_enabled(&self) -> bool {
        (self.function_reuse || self.semantic_reuse) && self.reuse == BodyReuse::ExactInputs
    }

    fn semantic_reuse_enabled(&self) -> bool {
        self.semantic_reuse && self.reuse == BodyReuse::ExactInputs
    }

    pub(super) fn infer_references(
        &mut self,
        files: &mut [TypedFileAst],
        external: &HashMap<crate::DefId, crate::TypedCallableSignature>,
    ) {
        let _profile = compiler_profile::span(compiler_profile::Phase::References);
        let profile_context = compiler_profile::Context::current();
        let queries = self.function_reuse_enabled().then_some(&self.references);
        let work = if self.parallel_references {
            if let Some(pool) = &self.pool {
                pool.install(|| {
                    let _profile_context = profile_context
                        .as_ref()
                        .map(compiler_profile::Context::enter);
                    super::references::infer_queries(files, external, true, queries)
                })
            } else {
                super::references::infer_queries(files, external, false, queries)
            }
        } else {
            super::references::infer_queries(files, external, false, queries)
        };
        self.stats.reference_checks += work.checked;
        self.stats.reference_checks_reused += work.reused;
        self.stats.reference_rounds += work.rounds;
    }

    pub fn with_effect_queries(mut self) -> Self {
        self.effect_queries = true;
        self
    }

    pub(super) fn infer_effects(
        &mut self,
        files: &mut [TypedFileAst],
        external: &HashMap<crate::DefId, crate::TypedCallableSignature>,
    ) {
        let _profile = compiler_profile::span(compiler_profile::Phase::Effects);
        let profile_context = compiler_profile::Context::current();
        let queries = self.function_reuse_enabled().then_some(&self.effects);
        let work = if self.effect_queries {
            if let Some(pool) = &self.pool {
                pool.install(|| {
                    let _profile_context = profile_context
                        .as_ref()
                        .map(compiler_profile::Context::enter);
                    super::effects::infer_cached(files, external, true, queries)
                })
            } else {
                super::effects::infer_cached(files, external, false, queries)
            }
        } else {
            super::effects::infer_package_full(files, external)
        };
        self.stats.effect_checks += work.checked;
        self.stats.effect_checks_skipped += work.skipped;
        self.stats.effect_checks_reused += work.reused;
    }

    pub fn with_retained_pass_limit(mut self, limit: std::num::NonZeroUsize) -> Self {
        self.retained_pass_limit = Some(limit);
        self.generations.truncate(limit.get());
        self
    }

    pub fn fork_empty(&self) -> Self {
        let declarations = declarations::DeclarationQueries::default();
        let specializations =
            super::specialize_calls::SpecializationQueries::new(declarations.sources());
        Self {
            pool: self.pool.clone(),
            reuse: self.reuse,
            semantic_reuse: self.semantic_reuse,
            function_reuse: self.function_reuse,
            retained_pass_limit: self.retained_pass_limit,
            effect_queries: self.effect_queries,
            parallel_references: self.parallel_references,
            finishing_reuse: self.finishing_reuse,
            generations: Vec::new(),
            references: Default::default(),
            effects: Default::default(),
            declarations,
            specializations,
            staging: Default::default(),
            stats: PackageCheckStats::default(),
        }
    }

    pub fn stats(&self) -> &PackageCheckStats {
        &self.stats
    }

    pub fn clear(&mut self) {
        self.generations.clear();
        self.references = Default::default();
        self.effects = Default::default();
        self.declarations = Default::default();
        self.specializations =
            super::specialize_calls::SpecializationQueries::new(self.declarations.sources());
        self.staging = Default::default();
    }

    pub fn transform(
        &mut self,
        files: &[(FileAst, FileId)],
        context: &TransformCtx,
        options: PackageTransformOptions,
    ) -> Vec<TypedFileAst> {
        assert_eq!(
            files
                .iter()
                .map(|(_, id)| id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            files.len(),
            "file identities must be unique"
        );
        self.stats = PackageCheckStats::default();
        self.references.begin();
        self.effects.begin();
        self.declarations.begin();
        self.specializations.begin();
        self.staging.begin();
        let result = super::transform_package_impl(files, context, options, Some(self));
        self.references.end();
        self.effects.end();
        self.declarations.end();
        self.specializations.end();
        self.staging.end();
        for (answers, expressions, dependencies) in
            [self.references.retained(), self.effects.retained()]
        {
            self.stats.retained_function_answers += answers;
            self.stats.retained_function_expressions += expressions;
            self.stats.retained_semantic_dependencies += dependencies;
        }
        self.generations.truncate(self.stats.passes);
        self.stats.retained_passes = self.generations.len();
        result
    }

    pub(super) fn stats_mut(&mut self) -> &mut PackageCheckStats {
        &mut self.stats
    }

    pub(super) fn specialize(&mut self, files: &mut [FileAst], typed: &mut [TypedFileAst]) -> bool {
        let expansion = self.specialize_pending(files, typed);
        expansion.publish(typed);
        expansion.expanded
    }

    pub(super) fn specialize_pending(
        &mut self,
        files: &mut [FileAst],
        typed: &[TypedFileAst],
    ) -> super::specialize_calls::Expansion {
        let queries = self
            .semantic_reuse_enabled()
            .then_some(&self.specializations);
        let (expansion, (checked, reused)) =
            super::specialize_calls::expand_pending(files, typed, queries);
        self.stats.specialization_checks += checked;
        self.stats.specialization_reused += reused;
        expansion
    }

    pub(super) fn prepare_staging(
        &mut self,
        files: &mut [TypedFileAst],
        external: &HashMap<crate::DefId, crate::TypedCallableSignature>,
    ) {
        let _profile = compiler_profile::span(compiler_profile::Phase::Staging);
        let scope =
            super::callable_signatures::CallableSignatureScope::borrowed(external, external);
        for file in files {
            if self.semantic_reuse_enabled() {
                if let Some(output) = self.staging.get(file, &(), &scope) {
                    file.exprs = output;
                    self.stats.staging_reused += 1;
                    continue;
                }
            }
            let input = self.semantic_reuse_enabled().then(|| file.clone());
            crate::staging::analyze_availability(file, external);
            crate::staging::prepare_resolved_expressions(file);
            if let Some(input) = input {
                let lookups = input.exprs.kind.iter().filter_map(|kind| match kind {
                    crate::typed::TypedExprKind::FnCall { target, .. } => Some(*target),
                    _ => None,
                });
                let lookups: Vec<_> = lookups.collect();
                self.staging
                    .insert(input, (), &scope, lookups, file.exprs.clone());
            }
            self.stats.staging_checks += 1;
        }
    }

    pub(super) fn declaration_environment(
        &mut self,
        context: &TransformCtx,
    ) -> Option<std::sync::Arc<declarations::Environment>> {
        self.semantic_reuse_enabled().then(|| {
            self.declarations
                .environment(&context.compile_time_eval_ast)
        })
    }

    pub(super) fn declare_types<'a>(
        &mut self,
        source: &'a FileAst,
        id: FileId,
        context: &TransformCtx,
        aliases: &HashMap<crate::TagId, crate::ty::Ty>,
        environment: Option<&std::sync::Arc<declarations::Environment>>,
    ) -> super::declare::DeclaredTypes<'a> {
        let (types, reused) = if let Some(environment) = environment {
            self.declarations
                .types(source, id, context, environment, aliases)
        } else {
            (
                super::declare::declare_types_with_aliases(source, id, context, aliases),
                false,
            )
        };
        self.stats.type_declaration_checks += usize::from(!reused);
        self.stats.type_declaration_reused += usize::from(reused);
        types
    }

    pub(super) fn resolve_signatures(
        &mut self,
        declarations: Vec<super::declare::DeclaredTypes<'_>>,
        context: &TransformCtx,
        environment: Option<&std::sync::Arc<declarations::Environment>>,
    ) -> Vec<TypedFileAst> {
        let _profile = compiler_profile::span(compiler_profile::Phase::Signatures);
        let context_key = environment.map(|_| self.declarations.signature_context(context));
        let profile_context = compiler_profile::Context::current();
        let resolve = |types| {
            let _profile_context = profile_context
                .as_ref()
                .map(compiler_profile::Context::enter);
            if let Some(environment) = environment {
                self.declarations.signatures(
                    types,
                    context,
                    environment,
                    context_key.as_ref().unwrap(),
                )
            } else {
                (super::declare::complete_signatures(types, context), false)
            }
        };
        let results: Vec<_> = match &self.pool {
            Some(pool) => pool.install(|| declarations.into_par_iter().map(resolve).collect()),
            None => declarations.into_iter().map(resolve).collect(),
        };
        results
            .into_iter()
            .map(|(typed, reused)| {
                self.stats.signature_checks += usize::from(!reused);
                self.stats.signature_reused += usize::from(reused);
                typed
            })
            .collect()
    }

    pub(super) fn lower(
        &mut self,
        declared: &mut [TypedFileAst],
        sources: &[&FileAst],
        context: &TransformCtx,
    ) {
        let _profile = compiler_profile::span(compiler_profile::Phase::Lowering);
        assert_eq!(
            declared.len(),
            sources.len(),
            "declared files and sources must have equal lengths"
        );
        let round = self
            .stats
            .passes
            .checked_sub(1)
            .expect("lowering requires an active package pass");
        let constants = self
            .generations
            .get(round)
            .filter(|generation| {
                generation.context.compile_time_eval_ast == context.compile_time_eval_ast
                    && generation
                        .context
                        .compile_time_eval_ast
                        .tags
                        .keys()
                        .eq(context.compile_time_eval_ast.tags.keys())
                    && same_ast_order(
                        &generation.context.compile_time_eval_ast,
                        &context.compile_time_eval_ast,
                    )
            })
            .map(|generation| std::sync::Arc::clone(&generation.constants))
            .unwrap_or_else(|| {
                std::sync::Arc::new(super::constant_queries::ConstantQueries::new(
                    std::sync::Arc::clone(&context.compile_time_eval_ast),
                ))
            });
        let before_constants = constants.stats();
        let previous = self.generations.get(round).filter(|generation| {
            same_semantic_context(&generation.context, context)
                && same_package_declarations(
                    &generation.context.compile_time_eval_ast,
                    &context.compile_time_eval_ast,
                )
                && generation
                    .context
                    .cross_file_tag_params
                    .iter()
                    .all(|(name, params)| {
                        params.keys().eq(context.cross_file_tag_params[name].keys())
                    })
                && same_ast_order(
                    &generation.context.compile_time_eval_ast,
                    &context.compile_time_eval_ast,
                )
        });
        let reuse = self.reuse == BodyReuse::ExactInputs
            && self
                .retained_pass_limit
                .is_none_or(|limit| round < limit.get());
        let profile_context = compiler_profile::Context::current();
        let check = |(typed, source): (&mut TypedFileAst, &&FileAst)| {
            let _profile_context = profile_context
                .as_ref()
                .map(compiler_profile::Context::enter);
            let previous = previous.and_then(|generation| generation.files.get(&typed.file_id));
            if let Some(previous) = previous.filter(|previous| {
                previous.source.as_ref() == *source
                    && same_ast_order(&previous.source, source)
                    && file_eq(&previous.declaration, typed, false)
                    && previous.reads.unchanged(
                        &previous.declaration.eval_ast,
                        &typed.eval_ast,
                        &constants,
                    )
            }) {
                let declaration = typed.clone();
                *typed = previous.result.clone();
                typed.eval_ast = std::sync::Arc::clone(&declaration.eval_ast);
                let result = typed.clone();
                return (
                    Some(LoweredFile {
                        source: previous.source.clone(),
                        declaration,
                        result,
                        reads: previous.reads.clone(),
                        reused: true,
                    }),
                    true,
                );
            }
            let tracking = reuse.then(|| {
                let declaration = typed.clone();
                (declaration, ImplementationReadRecorder::new())
            });
            lower_unit(
                typed,
                source,
                context,
                &constants,
                tracking.as_ref().map(|(_, reads)| reads),
            );
            let entry = tracking.map(|(declaration, reads)| {
                let result = typed.clone();
                LoweredFile {
                    source: if self.semantic_reuse_enabled() {
                        self.declarations.sources().intern(typed.file_id, source)
                    } else {
                        std::sync::Arc::new((*source).clone())
                    },
                    declaration,
                    result,
                    reads: reads.into_reads(),
                    reused: false,
                }
            });
            (entry, false)
        };
        let entries: Vec<_> = match &self.pool {
            Some(pool) => pool.install(|| {
                declared
                    .par_iter_mut()
                    .zip(sources.par_iter())
                    .map(check)
                    .collect()
            }),
            None => declared.iter_mut().zip(sources).map(check).collect(),
        };
        let reused = entries.iter().filter(|(_, reused)| *reused).count();
        self.stats.reused_files += reused;
        let after_constants = constants.stats();
        self.stats.constant_queries += after_constants.0 - before_constants.0;
        self.stats.constant_environments += after_constants.1 - before_constants.1;
        self.stats.lowered_files += declared.len() - reused;
        self.stats.lowered_expressions += declared
            .iter()
            .zip(&entries)
            .filter(|(_, (_, reused))| !reused)
            .map(|(typed, _)| typed.exprs.kind.len())
            .sum::<usize>();
        if reuse {
            let (finished, finish_inputs, read_logs, finish_stage) = self
                .generations
                .get_mut(round)
                .map(|previous| {
                    (
                        std::mem::take(&mut previous.finished),
                        std::mem::take(&mut previous.finish_inputs),
                        std::mem::take(&mut previous.read_logs),
                        previous.finish_stage,
                    )
                })
                .unwrap_or_default();
            let mut generation = LoweringGeneration {
                context: context.clone(),
                constants,
                files: entries
                    .into_iter()
                    .filter_map(|(entry, _)| entry.map(|entry| (entry.result.file_id, entry)))
                    .collect(),
                finished,
                finish_inputs,
                read_logs,
                finish_stage,
            };
            generation
                .finished
                .retain(|id, _| generation.files.contains_key(id));
            generation
                .finish_inputs
                .retain(|id, _| generation.files.contains_key(id));
            generation
                .read_logs
                .retain(|id, _| generation.files.contains_key(id));
            if round == self.generations.len() {
                self.generations.push(generation);
            } else {
                self.generations[round] = generation;
            }
        }
    }

    pub(super) fn skip_finishing(&mut self) {
        if let Some(generation) = self
            .generations
            .get_mut(self.stats.passes.saturating_sub(1))
        {
            generation.clear_finishing();
        }
    }

    pub(super) fn prepare_finishing(
        &self,
        typed: &mut [TypedFileAst],
        files: &[(FileAst, FileId)],
        context: &TransformCtx,
    ) {
        assert_eq!(typed.len(), files.len());
        let profile = compiler_profile::Context::current();
        let prepare = |(typed, (source, _)): (&mut TypedFileAst, &(FileAst, FileId))| {
            let _profile = profile.as_ref().map(compiler_profile::Context::enter);
            typed.type_registry = context.type_registry.clone();
            super::prepare_finishing_file(typed, source, context);
        };
        match &self.pool {
            Some(pool) => {
                pool.install(|| typed.par_iter_mut().zip(files.par_iter()).for_each(prepare))
            }
            None => typed.iter_mut().zip(files).for_each(prepare),
        }
    }

    pub(super) fn finish_with(
        &mut self,
        declared: &mut [TypedFileAst],
        sources: &[&FileAst],
        context: &TransformCtx,
        stage: FinishStage,
    ) {
        let _profile = compiler_profile::span(compiler_profile::Phase::Finishing);
        assert_eq!(
            declared.len(),
            sources.len(),
            "declared files and sources must have equal lengths"
        );
        if !self.finishing_reuse || self.reuse != BodyReuse::ExactInputs {
            let profile_context = compiler_profile::Context::current();
            let finish = |(typed, source): (&mut TypedFileAst, &&FileAst)| {
                let _profile_context = profile_context
                    .as_ref()
                    .map(compiler_profile::Context::enter);
                stage.initialize(typed, context);
                stage.run(typed, source, context);
            };
            match &self.pool {
                Some(pool) => pool.install(|| {
                    declared
                        .par_iter_mut()
                        .zip(sources.par_iter())
                        .for_each(finish)
                }),
                None => declared.iter_mut().zip(sources).for_each(finish),
            }
            return;
        }
        let round = self.stats.passes.saturating_sub(1);
        let reuse = self.reuse == BodyReuse::ExactInputs;
        for typed in declared.iter_mut() {
            stage.initialize(typed, context);
        }
        let mut reused_outputs: Vec<Option<TypedFileAst>> = vec![None; declared.len()];
        if reuse
            && let Some(generation) = self
                .generations
                .get(round)
                .filter(|generation| generation.finish_stage == Some(stage))
        {
            let constants = std::sync::Arc::clone(&generation.constants);
            for (slot, typed) in reused_outputs.iter_mut().zip(declared.iter()) {
                let id = typed.file_id;
                if !generation.files.get(&id).is_some_and(|file| file.reused) {
                    continue;
                }
                let (Some(log), Some(output)) =
                    (generation.read_logs.get(&id), generation.finished.get(&id))
                else {
                    continue;
                };
                if generation
                    .finish_inputs
                    .get(&id)
                    .is_some_and(|input| file_eq(input, typed, false))
                    && finish_log_valid(log, &context.cross_file_callable_signatures, &constants)
                {
                    *slot = Some(output.clone());
                }
            }
        }
        let inputs: Vec<_> = declared.iter().cloned().collect();
        let profile_context = compiler_profile::Context::current();
        let finish = |((typed, source), reused): (
            (&mut TypedFileAst, &&FileAst),
            &mut Option<TypedFileAst>,
        )|
         -> Option<FinishReadLog> {
            let _profile_context = profile_context
                .as_ref()
                .map(compiler_profile::Context::enter);
            if let Some(output) = reused.take() {
                let eval_ast = std::sync::Arc::clone(&typed.eval_ast);
                *typed = output;
                typed.eval_ast = eval_ast;
                return None;
            }
            finish_reads::begin();
            stage.run(typed, source, context);
            Some(finish_reads::end())
        };
        if reuse {
            self.stats.finishing_reused +=
                reused_outputs.iter().filter(|slot| slot.is_some()).count();
        }
        finish_reads::set_active(true);
        let logs: Vec<Option<FinishReadLog>> = match &self.pool {
            Some(pool) => pool.install(|| {
                declared
                    .par_iter_mut()
                    .zip(sources.par_iter())
                    .zip(reused_outputs.par_iter_mut())
                    .map(finish)
                    .collect()
            }),
            None => declared
                .iter_mut()
                .zip(sources)
                .zip(reused_outputs.iter_mut())
                .map(finish)
                .collect(),
        };
        finish_reads::set_active(false);
        if reuse && let Some(generation) = self.generations.get_mut(round) {
            if generation.finish_stage != Some(stage) {
                generation.clear_finishing();
                generation.finish_stage = Some(stage);
            }
            for ((typed, log), input) in declared.iter().zip(logs).zip(inputs) {
                let Some(log) = log else {
                    // A reused file's snapshot is already in the cache - it is
                    // where `typed` came from - and its finished state is
                    // unchanged. Re-cloning it would only duplicate it, but the
                    // cached `eval_ast` must not keep a superseded parse alive:
                    // reuse always re-points `eval_ast` from the fresh
                    // declaration, so only that field needs refreshing.
                    if let Some(entry) = generation.finished.get_mut(&typed.file_id) {
                        entry.eval_ast = std::sync::Arc::clone(&typed.eval_ast);
                    }
                    continue;
                };
                if log.scanned {
                    // A candidate scan cannot be validated as unchanged, so
                    // `finish_log_valid` will reject this file every pass;
                    // storing its snapshot would only be wasted memory.
                    continue;
                }
                generation.finished.insert(typed.file_id, typed.clone());
                generation.finish_inputs.insert(typed.file_id, input);
                generation.read_logs.insert(typed.file_id, log);
            }
        }
    }
}

fn finish_log_valid(
    log: &FinishReadLog,
    signatures: &HashMap<crate::DefId, crate::TypedCallableSignature>,
    constants: &super::constant_queries::ConstantQueries,
) -> bool {
    !log.scanned
        && log
            .signatures
            .iter()
            .all(|(id, recorded)| signatures.get(id) == recorded.as_ref())
        && log
            .constants
            .iter()
            .all(|(id, answer)| &constants.value(*id) == answer)
}

// IndexMap equality ignores order, which determines positional and arena identities.
pub(super) fn same_ast_order(left: &FileAst, right: &FileAst) -> bool {
    use ast::folder::Visitor;
    use std::ops::ControlFlow;

    #[derive(Default)]
    struct Orders(Vec<Vec<ast::prelude::Intern<String>>>);

    impl Visitor for Orders {
        fn visit_bind(&mut self, bind: &ast::Bind) -> ControlFlow<()> {
            if let Some(params) = &bind.params {
                self.0.push(params.keys().copied().collect());
            }
            self.0.push(bind.param_slots.keys().copied().collect());
            self.0
                .push(bind.param_conventions.keys().copied().collect());
            self.0.push(bind.param_groups.keys().copied().collect());
            self.0
                .push(bind.param_refinements.keys().copied().collect());
            ast::folder::walk_bind(self, bind)
        }
    }

    impl Orders {
        fn declaration(&mut self, declaration: &ast::Declare) {
            if let Some(params) = &declaration.params {
                self.0.push(params.keys().copied().collect());
            }
            if let ast::DeclareValue::Has(members) = &declaration.value {
                for member in members {
                    if let ast::HasMember::Function(function) = member {
                        self.0.push(function.params.keys().copied().collect());
                        self.0.push(function.conventions.keys().copied().collect());
                        self.0.push(function.param_groups.keys().copied().collect());
                        self.0
                            .push(function.param_refinements.keys().copied().collect());
                    }
                }
            }
            for extension in &declaration.extensions {
                self.declaration(extension);
            }
        }

        fn file(file: &FileAst) -> Self {
            let mut orders = Self::default();
            let _ = orders.visit_file_ast(file);
            for bind in &file.method_binds {
                let _ = orders.visit_bind(bind);
            }
            let mut tags: Vec<_> = file.tags.iter().collect();
            tags.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
            for (_, tag) in tags {
                orders.declaration(tag);
            }
            orders
        }
    }

    left.defs.keys().eq(right.defs.keys()) && Orders::file(left).0 == Orders::file(right).0
}

pub(super) fn lower_unit(
    typed: &mut TypedFileAst,
    source: &FileAst,
    context: &TransformCtx,
    constants: &super::constant_queries::ConstantQueries,
    reads: Option<&ImplementationReadRecorder>,
) {
    typed.type_registry = context.type_registry.clone();
    lower_file_with_queries(typed, source, context, constants, reads);
    super::splice_cross_file_tag_types(typed, context);
    crate::assembly::populate_target_constants(typed, &context.compile_target);
}

// TEMPORARY design probe: report which fields differ between two retained snapshots.
// PartialEq omits diagnostic and ownership state. The exhaustive cache matcher
// checks every field except the evaluation AST, whose reads are validated separately.
#[cfg(test)]
pub(crate) fn exact_file_eq(left: &TypedFileAst, right: &TypedFileAst) -> bool {
    file_eq(left, right, true)
}

pub(super) fn file_eq(left: &TypedFileAst, right: &TypedFileAst, include_eval_ast: bool) -> bool {
    macro_rules! fields {
        ($($field:ident),+ $(,)?) => {{
            let TypedFileAst { eval_ast, completion_probe: _, member_completions: _, $($field),+ } = left;
            (!include_eval_ast || eval_ast == &right.eval_ast) $(&& $field == &right.$field)+
        }};
    }
    fields!(
        span_table,
        file_id,
        type_registry,
        target_layout,
        constant_objects,
        tags,
        defs,
        overload_groups,
        private_tags,
        private_defs,
        exprs,
        resolved_member_calls,
        local_callable_parameters,
        local_callable_calls,
        sequence_index_checks,
        root_exprs,
        places,
        place_versions,
        place_version_components,
        import_mod_paths,
        tag_types,
        fn_return_types,
        variant_map,
        variant_annotations,
        imported_trait_names,
        self_contexts,
        semantic_origin,
        module_doc,
        warnings,
        declaration_flaws,
        parse_warnings,
        eval_conditions,
    )
}

#[cfg(test)]
mod tests;
