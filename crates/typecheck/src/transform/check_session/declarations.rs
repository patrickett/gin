use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ast::{BindValue, FileAst};
use internment::Intern;

use super::super::declare::{self, DeclaredTypes};
use super::{TransformCtx, intern_capped, same_semantic_context};
use crate::ty::Ty;
use crate::{DefId, FileId, TagId, TypedFileAst};

const ANSWERS_PER_FILE_MAX: usize = 64;

#[derive(Clone)]
pub(in crate::transform) struct Environment {
    declarations: FileAst,
    assigned_methods: HashSet<DefId>,
    constants: HashSet<Intern<String>>,
    target_dependent: HashSet<Intern<String>>,
}

impl Environment {
    pub fn new(source: &FileAst) -> Self {
        let mut declarations = FileAst::default();
        declarations.tags = source.tags.clone();
        let mut assigned_methods = HashSet::new();
        let mut constants = HashSet::new();
        for (id, bind) in &source.defs {
            if bind.is_constant() {
                constants.insert(id.0);
            }
            if bind.receiver_type_surface().is_some() {
                if !matches!(bind.value, BindValue::Unassigned) {
                    assigned_methods.insert(*id);
                }
                let mut header = bind.clone();
                header.value = BindValue::Unassigned;
                declarations.defs.insert(*id, header);
            }
        }
        Self {
            declarations,
            assigned_methods,
            constants,
            target_dependent: crate::prepare_target::target_dependent_bind_names(source),
        }
    }

    fn same(&self, other: &Self) -> bool {
        self.declarations == other.declarations
            && super::same_ast_order(&self.declarations, &other.declarations)
            && self.assigned_methods == other.assigned_methods
            && self.constants == other.constants
            && self.target_dependent == other.target_dependent
    }
}

#[derive(Default)]
pub(super) struct DeclarationQueries {
    sources: Arc<super::semantic_queries::SourceQueries>,
    types: HashMap<FileId, Vec<TypeAnswer>>,
    signatures: Mutex<HashMap<FileId, Vec<SignatureAnswer>>>,
    environments: Vec<(Arc<Environment>, bool)>,
    signature_contexts: Vec<(Arc<TransformCtx>, bool)>,
}

struct TypeAnswer {
    source: Arc<FileAst>,
    context: TransformCtx,
    environment: Arc<Environment>,
    aliases: HashMap<TagId, Ty>,
    output: DeclaredTypes<'static>,
    used: bool,
}

struct SignatureAnswer {
    input: DeclaredTypes<'static>,
    context: Arc<TransformCtx>,
    environment: Arc<Environment>,
    output: TypedFileAst,
    used: bool,
}

impl DeclarationQueries {
    pub fn sources(&self) -> Arc<super::semantic_queries::SourceQueries> {
        Arc::clone(&self.sources)
    }
    pub fn environment(&mut self, source: &FileAst) -> Arc<Environment> {
        intern_capped(
            &mut self.environments,
            &Environment::new(source),
            ANSWERS_PER_FILE_MAX,
            |answer, value| answer.same(value),
        )
    }

    pub fn signature_context(&mut self, context: &TransformCtx) -> Arc<TransformCtx> {
        intern_capped(
            &mut self.signature_contexts,
            context,
            ANSWERS_PER_FILE_MAX,
            same_semantic_context,
        )
    }

    pub fn begin(&mut self) {
        self.sources.begin();
        for (_, used) in &mut self.environments {
            *used = false;
        }
        for (_, used) in &mut self.signature_contexts {
            *used = false;
        }
        for answer in self.types.values_mut().flatten() {
            answer.used = false;
        }
        for answer in self.signatures.get_mut().unwrap().values_mut().flatten() {
            answer.used = false;
        }
    }

    pub fn end(&mut self) {
        self.sources.end();
        self.environments.retain(|(_, used)| *used);
        self.signature_contexts.retain(|(_, used)| *used);
        self.types.retain(|_, answers| {
            answers.retain(|a| a.used);
            !answers.is_empty()
        });
        self.signatures.get_mut().unwrap().retain(|_, answers| {
            answers.retain(|a| a.used);
            !answers.is_empty()
        });
    }

    pub fn types<'a>(
        &mut self,
        source: &'a FileAst,
        id: FileId,
        context: &TransformCtx,
        environment: &Arc<Environment>,
        aliases: &HashMap<TagId, Ty>,
    ) -> (DeclaredTypes<'a>, bool) {
        let answers = self.types.entry(id).or_default();
        if let Some(answer) = answers.iter_mut().find(|answer| {
            answer.source.as_ref() == source
                && super::same_ast_order(&answer.source, source)
                && same_semantic_context(&answer.context, context)
                && Arc::ptr_eq(&answer.environment, environment)
                && answer.aliases == *aliases
        }) {
            answer.used = true;
            let mut output = answer.output.clone();
            output.typed.eval_ast = Arc::clone(&context.compile_time_eval_ast);
            return (output, true);
        }
        let output = declare::declare_types_with_aliases(source, id, context, aliases);
        let source = self.sources.intern(id, source);
        let output = output.snapshot_with_source(Arc::clone(&source));
        if answers.len() == ANSWERS_PER_FILE_MAX {
            answers.remove(0);
        }
        answers.push(TypeAnswer {
            source,
            context: context.clone(),
            environment: Arc::clone(environment),
            aliases: aliases.clone(),
            output: output.clone(),
            used: true,
        });
        (output, false)
    }

    pub fn signatures(
        &self,
        input: DeclaredTypes<'_>,
        context: &TransformCtx,
        environment: &Arc<Environment>,
        context_key: &Arc<TransformCtx>,
    ) -> (TypedFileAst, bool) {
        {
            let mut entries = self.signatures.lock().unwrap();
            if let Some(answer) = entries
                .entry(input.typed.file_id)
                .or_default()
                .iter_mut()
                .find(|answer| {
                    Arc::ptr_eq(&answer.context, context_key)
                        && input.same(&answer.input)
                        && Arc::ptr_eq(&answer.environment, environment)
                })
            {
                answer.used = true;
                let mut output = answer.output.clone();
                output.eval_ast = Arc::clone(&context.compile_time_eval_ast);
                return (output, true);
            }
        }
        let snapshot = input.snapshot();
        let output = declare::complete_signatures(input, context);
        let mut entries = self.signatures.lock().unwrap();
        let answers = entries.entry(output.file_id).or_default();
        if answers.len() == ANSWERS_PER_FILE_MAX {
            answers.remove(0);
        }
        answers.push(SignatureAnswer {
            input: snapshot,
            context: Arc::clone(context_key),
            environment: Arc::clone(environment),
            output: output.clone(),
            used: true,
        });
        (output, false)
    }
}
