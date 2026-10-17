use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::*;
use crate::transform::check_session::intern_capped;
use crate::typed::ExprId;
use ast::SpanId;

#[derive(Default)]
pub(in crate::transform) struct SpecializationQueries {
    sources: Arc<crate::transform::check_session::SourceQueries>,
    answers: crate::transform::check_session::FileQueries<Inputs, Plan>,
    environments: std::sync::Mutex<Vec<(Arc<Environment>, bool)>>,
}

impl SpecializationQueries {
    pub fn new(sources: Arc<crate::transform::check_session::SourceQueries>) -> Self {
        Self {
            sources,
            ..Self::default()
        }
    }
    pub fn begin(&mut self) {
        self.answers.begin();
        for (_, used) in self.environments.get_mut().unwrap() {
            *used = false;
        }
    }

    pub fn end(&mut self) {
        self.answers.end();
        self.environments
            .get_mut()
            .unwrap()
            .retain(|(_, used)| *used);
    }

    pub(super) fn environment(&self, environment: Environment) -> Arc<Environment> {
        let mut entries = self.environments.lock().unwrap();
        intern_capped(&mut entries, &environment, 64, |answer, value| {
            answer.same(value)
        })
    }
}

pub(in crate::transform) struct Inputs {
    source: Arc<FileAst>,
    environment: Arc<Environment>,
}

impl PartialEq for Inputs {
    fn eq(&self, other: &Self) -> bool {
        self.matches(&other.source, &other.environment)
    }
}

impl Inputs {
    fn matches(&self, source: &FileAst, environment: &Arc<Environment>) -> bool {
        Arc::ptr_eq(&self.environment, environment)
            && self.source.as_ref() == source
            && super::super::check_session::same_ast_order(&self.source, source)
    }
}

#[derive(Clone)]
pub(super) struct Environment {
    templates: HashMap<DefId, (usize, TypedBind)>,
    sources: Vec<FileAst>,
    existing: Vec<HashSet<DefId>>,
    methods: Methods,
    declarations: ast::TagMap,
    eval_declarations: Vec<FileAst>,
    eval_slots: Vec<usize>,
    proof_members: HashSet<Intern<String>>,
}

impl Environment {
    pub fn uncached(
        typed: &[TypedFileAst],
        templates: HashMap<DefId, (usize, TypedBind)>,
        methods: Methods,
        declarations: ast::TagMap,
    ) -> Self {
        Self {
            templates,
            methods,
            declarations,
            sources: Vec::new(),
            existing: Vec::new(),
            eval_declarations: Vec::new(),
            eval_slots: Vec::new(),
            proof_members: proof_getters::members(typed),
        }
    }

    pub fn new(
        files: &[FileAst],
        typed: &[TypedFileAst],
        templates: &HashMap<DefId, (usize, TypedBind)>,
        methods: &Methods,
        declarations: ast::TagMap,
    ) -> Self {
        let sources = files
            .iter()
            .map(|file| {
                let mut source = FileAst::default();
                for (id, bind) in &file.defs {
                    if templates.contains_key(id) {
                        source.defs.insert(*id, bind.clone());
                        source.method_binds.extend(
                            file.method_binds
                                .iter()
                                .filter(|method| method.name_span == bind.name_span)
                                .cloned(),
                        );
                    }
                }
                source
            })
            .collect();
        let mut eval_sources = Vec::new();
        let eval_slots = typed
            .iter()
            .map(|file| {
                if let Some(index) = eval_sources
                    .iter()
                    .position(|source| Arc::ptr_eq(source, &file.eval_ast))
                {
                    return index;
                }
                let index = eval_sources.len();
                eval_sources.push(Arc::clone(&file.eval_ast));
                index
            })
            .collect();
        let eval_declarations = eval_sources
            .iter()
            .map(|source| {
                let mut declarations = FileAst::default();
                declarations.tags = source.tags.clone();
                declarations
            })
            .collect();
        Self {
            templates: templates.clone(),
            sources,
            existing: files
                .iter()
                .map(|file| file.defs.keys().copied().collect())
                .collect(),
            methods: methods.clone(),
            declarations,
            eval_declarations,
            eval_slots,
            proof_members: proof_getters::members(typed),
        }
    }

    fn same(&self, other: &Self) -> bool {
        self.templates == other.templates
            && self.eval_slots == other.eval_slots
            && self.existing == other.existing
            && self.methods == other.methods
            && self.declarations == other.declarations
            && self.proof_members == other.proof_members
            && same_sources(&self.sources, &other.sources)
            && same_sources(&self.eval_declarations, &other.eval_declarations)
    }
}

fn same_sources(left: &[FileAst], right: &[FileAst]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left == right && super::super::check_session::same_ast_order(left, right)
        })
}

type Addition = (usize, DefId, Bind, Option<Bind>);
type Rewrite = (DefId, SpanId, DefId, TypedBind, Option<ast::FnCall>);

#[derive(Clone, Default)]
pub(in crate::transform) struct Plan {
    pub(super) proofs: Vec<Addition>,
    pub(super) additions: Vec<Addition>,
    pub(super) rewrites: Vec<Rewrite>,
    pub(super) diagnostics: Vec<(ExprId, Diagnostic)>,
}

pub(super) fn plan(
    files: &[FileAst],
    index: usize,
    typed: &TypedFileAst,
    environment: &Arc<Environment>,
    queries: Option<&SpecializationQueries>,
) -> (Plan, bool) {
    let empty = HashMap::new();
    let signatures =
        super::super::callable_signatures::CallableSignatureScope::borrowed(&empty, &empty);
    if let Some(output) = queries.and_then(|queries| {
        queries.answers.get_by(typed, &signatures, |inputs| {
            inputs.matches(&files[index], environment)
        })
    }) {
        return (output, true);
    }
    let mut output = super::plan_calls(
        files,
        index,
        typed,
        &environment.templates,
        &environment.methods,
        &environment.declarations,
    );
    proof_getters::expand(
        files,
        std::slice::from_ref(typed),
        &environment.templates,
        &environment.methods,
        &mut output.proofs,
        &environment.proof_members,
    );
    if let Some(queries) = queries {
        let inputs = Inputs {
            source: queries.sources.intern(typed.file_id, &files[index]),
            environment: Arc::clone(environment),
        };
        queries
            .answers
            .insert(typed.clone(), inputs, &signatures, [], output.clone());
    }
    (output, false)
}
