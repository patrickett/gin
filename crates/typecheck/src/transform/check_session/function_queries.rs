use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ast::span::SpanId;

use super::super::callable_signatures::CallableSignatureScope;
use super::super::semantic_reads::SemanticReads;
use super::intern_capped;
use crate::typed::*;
use crate::{FileId, TypeDeclarationSemantics, TypeRegistry};
use ast::ty::{DeclarationKey, TypeId};

mod relocation;
pub(in crate::transform) use relocation::Relocation;
use relocation::optional;

const ANSWERS_PER_FUNCTION_MAX: usize = 4;
const FUNCTIONS_MAX: usize = 4096;
const INPUT_EXPRESSIONS_MAX: usize = 32_768;
const DEPENDENCIES_MAX: usize = 65_536;

#[derive(Clone, PartialEq)]
pub(in crate::transform) struct FunctionInput {
    bind: BindInput,
    expressions: Box<[ExpressionInput]>,
    places: Box<[(PlaceId, TypedPlace)]>,
    recursive: Vec<DefId>,
}

#[derive(Clone, PartialEq)]
struct BindInput {
    dependent_binder: ast::BinderId,
    params: Arc<[(internment::Intern<String>, ast::Ty)]>,
    param_kinds: ParamKinds,
    param_constraints: Arc<[Vec<ast::Ty>]>,
    param_conventions: ParamConventions,
    param_groups: GroupSlots,
    param_refinements: Arc<[Option<ast::ty::PredicateExpr>]>,
    payload_postconditions: Arc<[TypedPayloadPostcondition]>,
    groups: Arc<[TypedGroup]>,
    result_groups: GroupSlots,
    return_type: ast::Ty,
    body: BindBody,
    postcondition: Option<Box<TypedPostcondition>>,
}

impl BindInput {
    fn new(bind: &TypedBind, relocation: Relocation) -> Option<Self> {
        let mut body = bind.body.clone();
        relocation.body(&mut body)?;
        let mut postcondition = bind.postcondition.clone();
        if let Some(proof) = &mut postcondition {
            proof.span = SpanId::INVALID;
        }
        Some(Self {
            dependent_binder: bind.dependent_binder,
            params: Arc::clone(&bind.params),
            param_kinds: bind.param_kinds.clone(),
            param_constraints: Arc::clone(&bind.param_constraints),
            param_conventions: bind.param_conventions.clone(),
            param_groups: bind.param_groups.clone(),
            param_refinements: Arc::clone(&bind.param_refinements),
            payload_postconditions: Arc::clone(&bind.payload_postconditions),
            groups: Arc::clone(&bind.groups),
            result_groups: bind.result_groups.clone(),
            return_type: bind.return_type.clone(),
            body,
            postcondition,
        })
    }
}

#[derive(Clone, PartialEq)]
struct ExpressionInput {
    id: ExprId,
    kind: TypedExprKind,
    ty: ast::Ty,
    constant: Option<ast::ConstValue>,
    target: Option<ReferenceTargetSet>,
    results: Vec<Option<ReferenceTargetSet>>,
    place: Option<PlaceId>,
}

impl FunctionInput {
    fn matches(
        &self,
        file: &TypedFileAst,
        id: DefId,
        recursive: Option<&HashSet<DefId>>,
    ) -> Option<Relocation> {
        let bind = file.defs.get(&id)?;
        let anchor = |body: &BindBody| match body {
            BindBody::Expr(id) => Some(*id),
            BindBody::Body { exprs, ret } => exprs.first().copied().or(*ret),
            BindBody::Unlowered => None,
        };
        let expression = match (anchor(&bind.body), anchor(&self.bind.body)) {
            (Some(current), Some(stored)) => i64::from(current.0) - i64::from(stored.0),
            (None, None) => 0,
            _ => return None,
        };
        let relocation = Relocation::new(expression, 0);
        let place = if let Some(input) = self.expressions.iter().find(|input| input.place.is_some())
        {
            let current = relocation.inverse().expression(input.id)?;
            let current = file.exprs.place_of(current).flatten()?;
            i64::from(current.0) - i64::from(input.place?.0)
        } else if self.places.is_empty() {
            0
        } else {
            Relocation::for_function(file, bind)?.2.place_offset()
        };
        let relocation = Relocation::new(expression, place);
        if self.bind != BindInput::new(bind, relocation)? {
            return None;
        }
        if self.recursive.len() != recursive.map_or(0, HashSet::len)
            || self
                .recursive
                .iter()
                .any(|id| !recursive.is_some_and(|set| set.contains(id)))
        {
            return None;
        }
        for input in &self.expressions {
            let current = relocation.inverse().expression(input.id)?;
            if *input != ExpressionInput::new(file, current, relocation)? {
                return None;
            }
        }
        for (id, expected) in &self.places {
            let current = relocation.inverse().place(*id)?;
            let mut current = file.places.get(current.0 as usize)?.clone();
            current.parent = optional(current.parent, |id| relocation.place(id))?;
            if current != *expected {
                return None;
            }
        }
        Some(relocation)
    }

    pub fn new(
        file: &TypedFileAst,
        id: DefId,
        recursive: Option<&HashSet<DefId>>,
    ) -> Option<(Self, Relocation)> {
        let (expressions, places, relocation) = Relocation::for_function(file, &file.defs[&id])?;
        let bind = BindInput::new(&file.defs[&id], relocation)?;
        let expressions = expressions
            .into_iter()
            .map(|id| ExpressionInput::new(file, id, relocation))
            .collect::<Option<Box<[_]>>>()?;
        let places = places
            .into_iter()
            .map(|id| {
                let mut place = file.places[id.0 as usize].clone();
                place.parent = optional(place.parent, |id| relocation.place(id))?;
                Some((relocation.place(id)?, place))
            })
            .collect::<Option<Box<[_]>>>()?;
        let mut recursive: Vec<_> = recursive.into_iter().flatten().copied().collect();
        recursive
            .sort_by(|left, right| (left.0.as_str(), left.1).cmp(&(right.0.as_str(), right.1)));
        Some((
            Self {
                bind,
                expressions,
                places,
                recursive,
            },
            relocation,
        ))
    }
}

impl ExpressionInput {
    fn new(file: &TypedFileAst, id: ExprId, relocation: Relocation) -> Option<Self> {
        let index = id.index();
        let mut kind = file.exprs.kind.get(index)?.clone();
        relocation.kind(&mut kind)?;
        let facts = &file.exprs.facts.ownership_facts;
        Some(Self {
            id: relocation.expression(id)?,
            kind,
            ty: file.exprs.ty[index].clone(),
            constant: file.exprs.facts.const_facts.const_value[index].clone(),
            target: optional(facts.target_group[index].as_ref(), |x| {
                relocation.targets(x)
            })?,
            results: facts.result_target_groups[index]
                .iter()
                .map(|x| optional(x.as_ref(), |x| relocation.targets(x)))
                .collect::<Option<_>>()?,
            place: optional(facts.place[index], |id| relocation.place(id))?,
        })
    }
}

type Dependencies = Box<[(DefId, Option<Arc<TypedCallableSignature>>)]>;

struct TypeDependencies {
    declarations: Box<[(TypeId, Option<Arc<TypeDeclarationSemantics>>)]>,
    handles: Box<[(DeclarationKey, Option<TypeId>)]>,
}

impl TypeDependencies {
    fn unchanged(&self, registry: &TypeRegistry) -> bool {
        self.declarations
            .iter()
            .all(|(id, value)| registry.declaration(*id) == value.as_deref())
            && self
                .handles
                .iter()
                .all(|(key, value)| registry.handle(key) == *value)
    }
}

struct Answer<V> {
    input: Arc<FunctionInput>,
    reads: Dependencies,
    output: V,
    types: TypeDependencies,
    bodies: Box<[BodyDependency]>,
    references: Box<[ReferenceDependency]>,
    used: bool,
}

impl<V> Answer<V> {
    fn dependency_count(&self) -> usize {
        self.reads.len()
            + self.types.declarations.len()
            + self.types.handles.len()
            + self.bodies.len()
            + self.references.len()
    }
}

struct ReferenceDependency {
    id: ExprId,
    value: Option<Box<ReferenceValue>>,
}

impl ReferenceDependency {
    fn unchanged(&self, file: &TypedFileAst, relocation: Relocation) -> bool {
        let Some(id) = relocation.inverse().expression(self.id) else {
            return false;
        };
        let mut current = file.exprs.references_of(id.index()).clone();
        if relocation.reference(&mut current).is_none() {
            return false;
        }
        self.value.as_deref().unwrap_or(ReferenceValue::empty()) == &current
    }
}

struct BodyDependency {
    id: DefId,
    input: Option<Arc<FunctionInput>>,
    relative: (i64, i64),
}

impl BodyDependency {
    fn unchanged(&self, file: &TypedFileAst, relocation: Relocation) -> bool {
        if !file.defs.contains_key(&self.id) {
            return self.input.is_none();
        }
        let Some(current) = self
            .input
            .as_ref()
            .and_then(|input| input.matches(file, self.id, None))
        else {
            return false;
        };
        self.relative == current.relative_to(relocation)
    }
}

struct State<V> {
    entries: HashMap<(FileId, DefId), Vec<Answer<V>>>,
    signatures: HashMap<DefId, Vec<(Arc<TypedCallableSignature>, bool)>>,
    declarations: HashMap<TypeId, Vec<(Arc<TypeDeclarationSemantics>, bool)>>,
    inputs: HashMap<(FileId, DefId), Vec<Arc<FunctionInput>>>,
    input_expressions: usize,
    dependencies: usize,
}

impl<V> State<V> {
    fn intern_input(
        &mut self,
        file: FileId,
        id: DefId,
        input: FunctionInput,
    ) -> Option<Arc<FunctionInput>> {
        let inputs = self.inputs.entry((file, id)).or_default();
        if let Some(existing) = inputs.iter().find(|existing| existing.as_ref() == &input) {
            return Some(Arc::clone(existing));
        }
        if inputs.len() == ANSWERS_PER_FUNCTION_MAX
            || self.input_expressions + input.expressions.len() > INPUT_EXPRESSIONS_MAX
        {
            return None;
        }
        self.input_expressions += input.expressions.len();
        let input = Arc::new(input);
        inputs.push(Arc::clone(&input));
        Some(input)
    }
}

impl<V> State<V> {
    fn body_dependencies(
        &mut self,
        file: &TypedFileAst,
        relocation: Relocation,
        reads: HashSet<DefId>,
    ) -> Option<Box<[BodyDependency]>> {
        reads
            .into_iter()
            .map(|id| {
                let (input, relative) = if file.defs.contains_key(&id) {
                    let (input, current) = FunctionInput::new(file, id, None)?;
                    (
                        Some(self.intern_input(file.file_id, id, input)?),
                        current.relative_to(relocation),
                    )
                } else {
                    (None, (0, 0))
                };
                Some(BodyDependency {
                    id,
                    input,
                    relative,
                })
            })
            .collect()
    }

    fn type_dependencies(
        &mut self,
        registry: &TypeRegistry,
        reads: HashSet<TypeId>,
        keys: HashSet<DeclarationKey>,
    ) -> TypeDependencies {
        let declarations = reads
            .into_iter()
            .map(|id| {
                let value = registry.declaration(id).map(|value| {
                    intern_capped(
                        self.declarations.entry(id).or_default(),
                        value,
                        ANSWERS_PER_FUNCTION_MAX,
                        |stored, value| stored == value,
                    )
                });
                (id, value)
            })
            .collect();
        let handles = keys
            .into_iter()
            .map(|key| {
                let value = registry.handle(&key);
                (key, value)
            })
            .collect();
        TypeDependencies {
            declarations,
            handles,
        }
    }

    fn call_dependencies(
        &mut self,
        signatures: &CallableSignatureScope<'_>,
        reads: HashSet<DefId>,
    ) -> Dependencies {
        reads
            .into_iter()
            .map(|id| {
                let value = signatures.get(&id).map(|value| {
                    intern_capped(
                        self.signatures.entry(id).or_default(),
                        value,
                        ANSWERS_PER_FUNCTION_MAX,
                        |stored, value| stored == value,
                    )
                });
                (id, value)
            })
            .collect()
    }
}

pub(in crate::transform) struct FunctionQueries<V> {
    state: Mutex<State<V>>,
}

impl<V> Default for FunctionQueries<V> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                signatures: HashMap::new(),
                declarations: HashMap::new(),
                inputs: HashMap::new(),
                input_expressions: 0,
                dependencies: 0,
            }),
        }
    }
}

impl<V: Clone> FunctionQueries<V> {
    pub fn inputs(
        &self,
        file: &TypedFileAst,
        recursive: Option<&HashMap<DefId, HashSet<DefId>>>,
    ) -> HashMap<DefId, (Arc<FunctionInput>, Relocation)> {
        let mut state = self.state.lock().unwrap();
        file.defs
            .keys()
            .filter_map(|id| {
                let recursive = recursive.and_then(|recursive| recursive.get(id));
                if let Some(inputs) = state.inputs.get(&(file.file_id, *id)) {
                    for input in inputs {
                        if let Some(relocation) = input.matches(file, *id, recursive) {
                            return Some((*id, (Arc::clone(input), relocation)));
                        }
                    }
                }
                let (input, relocation) = FunctionInput::new(file, *id, recursive)?;
                let input = state.intern_input(file.file_id, *id, input)?;
                Some((*id, (input, relocation)))
            })
            .collect()
    }

    pub fn get(
        &self,
        file: &TypedFileAst,
        id: DefId,
        input: &FunctionInput,
        relocation: Relocation,
        signatures: &CallableSignatureScope<'_>,
    ) -> Option<V> {
        let mut state = self.state.lock().unwrap();
        let answer = state
            .entries
            .get_mut(&(file.file_id, id))?
            .iter_mut()
            .find(|answer| {
                (std::ptr::eq(answer.input.as_ref(), input) || *answer.input == *input)
                    && answer
                        .bodies
                        .iter()
                        .all(|body| body.unchanged(file, relocation))
                    && answer.types.unchanged(&file.type_registry)
                    && answer
                        .references
                        .iter()
                        .all(|read| read.unchanged(file, relocation))
                    && answer
                        .reads
                        .iter()
                        .all(|(id, value)| signatures.get(id) == value.as_deref())
            })?;
        answer.used = true;
        Some(answer.output.clone())
    }

    pub fn insert(
        &self,
        file: &TypedFileAst,
        id: DefId,
        input: FunctionInput,
        relocation: Relocation,
        signatures: &CallableSignatureScope<'_>,
        lookups: HashSet<DefId>,
        types: SemanticReads,
        output: V,
    ) {
        let mut state = self.state.lock().unwrap();
        if !state.entries.contains_key(&(file.file_id, id)) && state.entries.len() == FUNCTIONS_MAX
        {
            return;
        }
        assert!(!types.scanned);
        let dependency_count = lookups.len()
            + types.declarations.len()
            + types.handles.len()
            + types.bodies.len()
            + types.references.len();
        if state.dependencies + dependency_count > DEPENDENCIES_MAX {
            return;
        }
        let Some(bodies) = state.body_dependencies(file, relocation, types.bodies) else {
            return;
        };
        let Some(input) = state.intern_input(file.file_id, id, input) else {
            return;
        };
        let references: Option<Box<[_]>> = types
            .references
            .into_iter()
            .map(|(id, mut value)| {
                relocation.reference(&mut value)?;
                let value = (value != ReferenceValue::default()).then(|| Box::new(value));
                Some(ReferenceDependency {
                    id: relocation.expression(id)?,
                    value,
                })
            })
            .collect();
        let Some(references) = references else {
            return;
        };
        let types = state.type_dependencies(&file.type_registry, types.declarations, types.handles);
        let reads = state.call_dependencies(signatures, lookups);
        state.dependencies += dependency_count;
        let answers = state.entries.entry((file.file_id, id)).or_default();
        let removed = if answers.len() == ANSWERS_PER_FUNCTION_MAX {
            Some(answers.remove(0))
        } else {
            None
        };
        answers.push(Answer {
            input,
            reads,
            output,
            types,
            bodies,
            references,
            used: true,
        });
        if let Some(removed) = removed {
            state.dependencies -= removed.dependency_count();
        }
    }

    pub fn retained(&self) -> (usize, usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state.entries.values().map(Vec::len).sum(),
            state.input_expressions,
            state.dependencies,
        )
    }

    pub fn begin(&mut self) {
        for answer in self.state.get_mut().unwrap().entries.values_mut().flatten() {
            answer.used = false;
        }
    }

    pub fn end(&mut self) {
        let state = self.state.get_mut().unwrap();
        state.entries.retain(|_, answers| {
            answers.retain(|answer| answer.used);
            !answers.is_empty()
        });
        state.signatures.retain(|_, values| {
            values.retain(|(value, _)| Arc::strong_count(value) > 1);
            !values.is_empty()
        });
        state.inputs.retain(|_, values| {
            values.retain(|value| Arc::strong_count(value) > 1);
            !values.is_empty()
        });
        state.declarations.retain(|_, values| {
            values.retain(|(value, _)| Arc::strong_count(value) > 1);
            !values.is_empty()
        });
        state.input_expressions = state
            .inputs
            .values()
            .flatten()
            .map(|input| input.expressions.len())
            .sum();
        state.dependencies = state
            .entries
            .values()
            .flatten()
            .map(Answer::dependency_count)
            .sum();
        assert!(state.input_expressions <= INPUT_EXPRESSIONS_MAX);
        assert!(state.dependencies <= DEPENDENCIES_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::semantic_reads::SemanticReadGuard;
    use parser::query::SourceParseExt;

    #[test]
    fn type_dependencies_track_missing_and_changed_answers_without_global_invalidation() {
        let parsed = format!(
            "{}read(value Word) Word: return value\n",
            test_fixtures::preludes::WORD_PRELUDE
        )
        .parse_source_full();
        let mut file = crate::transform::transform_file(parsed.ast, FileId(0));
        let word = TypeId {
            file: 0,
            name: internment::Intern::from_ref("Word"),
        };
        let absent = TypeId {
            file: 0,
            name: internment::Intern::from_ref("Absent"),
        };
        let id = DefId::named(internment::Intern::from_ref("read"));
        let (input, relocation) = FunctionInput::new(&file, id, None).unwrap();
        let external = HashMap::new();
        let signatures = CallableSignatureScope::new(&external, []);
        let queries = FunctionQueries::default();
        let get = |file: &TypedFileAst| queries.get(file, id, &input, relocation, &signatures);
        let recording = SemanticReadGuard::begin();
        assert!(file.type_registry.declaration(word).is_some());
        let types = recording.finish();
        queries.insert(
            &file,
            id,
            input.clone(),
            relocation,
            &signatures,
            HashSet::new(),
            types,
            7,
        );
        assert_eq!(get(&file), Some(7));
        let mut extra = file.type_registry.declaration(word).unwrap().clone();
        extra.key.declaration = absent.name;
        file.type_registry.insert(absent, extra);
        assert_eq!(get(&file), Some(7));
        let mut changed = file.type_registry.declaration(word).unwrap().clone();
        changed.definition = ast::Ty::Unit;
        file.type_registry.insert(word, changed);
        assert_eq!(get(&file), None);

        let missing = TypeId {
            file: 0,
            name: internment::Intern::from_ref("Missing"),
        };
        let recording = SemanticReadGuard::begin();
        assert!(file.type_registry.declaration(missing).is_none());
        let types = recording.finish();
        queries.insert(
            &file,
            id,
            input.clone(),
            relocation,
            &signatures,
            HashSet::new(),
            types,
            9,
        );
        assert_eq!(get(&file), Some(9));
        let mut added = file.type_registry.declaration(word).unwrap().clone();
        added.key.declaration = missing.name;
        file.type_registry.insert(missing, added);
        assert_eq!(get(&file), None);
    }
}
